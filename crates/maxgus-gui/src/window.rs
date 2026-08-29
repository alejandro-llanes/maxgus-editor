//! The window, and the loop that keeps it in step with the editor.
//!
//! The editor is the same one the terminal front end drives: the same
//! commands, the same keymaps, the same `draw`. What differs is where the keys
//! come from and where the cells go — and that the window can draw a fraction
//! of a line, which is what smooth scrolling is.

use crate::font::{CellMetrics, Fonts};
use crate::quads::Palette;
use crate::renderer::Renderer;
use crate::scroll::Scroll;
use anyhow::Result;
use maxgus_core::{Dispatcher, Editor, Task, TaskResult};
use maxgus_tui::{Rect, Size, Surface};
use std::sync::Arc;
use std::sync::mpsc;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

/// How the editor is set up before the window opens.
pub struct Settings {
    pub title: String,
    pub font: String,
    pub font_size: f32,
    /// The window's own colours, which a terminal would have supplied.
    pub palette: Palette,
}

/// Runs the editor in a window until it is asked to leave.
pub fn run(
    editor: Editor,
    dispatcher: Dispatcher,
    settings: Settings,
    tasks: std::sync::mpsc::Sender<Task>,
    results: mpsc::Receiver<TaskResult>,
) -> Result<()> {
    let event_loop = EventLoop::new()?;
    // Poll rather than wait: results arrive from the executor on a channel
    // that the window system knows nothing about, and an animation in flight
    // owes a frame whether or not anything was typed.
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        editor,
        dispatcher,
        settings,
        tasks,
        results,
        window: None,
        renderer: None,
        fonts: None,
        surface: Surface::new(Size::new(1, 1)),
        scroll: Scroll::new(),
        modifiers: ModifiersState::empty(),
        pointer: (0.0, 0.0),
        selecting: false,
        clipboard: arboard::Clipboard::new().ok(),
        failure: None,
    };
    event_loop.run_app(&mut app)?;
    match app.failure.take() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

struct App {
    editor: Editor,
    dispatcher: Dispatcher,
    settings: Settings,
    tasks: std::sync::mpsc::Sender<Task>,
    results: mpsc::Receiver<TaskResult>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    fonts: Option<Fonts>,
    surface: Surface,
    scroll: Scroll,
    modifiers: ModifiersState,
    pointer: (f64, f64),
    selecting: bool,
    clipboard: Option<arboard::Clipboard>,
    /// Set when something went wrong badly enough to stop, and reported once
    /// the event loop has given control back.
    failure: Option<anyhow::Error>,
}

impl App {
    fn metrics(&self) -> CellMetrics {
        self.fonts
            .as_ref()
            .map(Fonts::metrics)
            .unwrap_or(CellMetrics {
                width: 8.0,
                height: 16.0,
                ascent: 12.0,
            })
    }

    /// Sends whatever the last commands queued, and folds in whatever the
    /// executor has finished.
    fn pump(&mut self) {
        for task in self.editor.tasks.drain() {
            let _ = self.tasks.send(task);
        }
        while let Ok(result) = self.results.try_recv() {
            self.apply(result);
        }
        // Applying a result can queue more work — a file read asks for
        // highlighting — and nothing else would send it.
        for task in self.editor.tasks.drain() {
            let _ = self.tasks.send(task);
        }
    }

    fn apply(&mut self, result: TaskResult) {
        #[cfg(feature = "lsp")]
        if let TaskResult::LspResponse { .. } | TaskResult::LspApplyEdit { .. } = &result {
            self.editor.apply_lsp_response(result);
            return;
        }
        if let Err(error) = self.editor.apply_task_result(result) {
            self.editor.error(error.to_string());
        }
    }

    /// Lays the editor out for a window of this many pixels.
    fn fit(&mut self, width: u32, height: u32) {
        let metrics = self.metrics();
        let (columns, rows) = metrics.grid(width as f32, height as f32);
        let size = Size::new(columns, rows);
        if self.surface.size() != size {
            self.surface.resize(size);
            self.editor.set_frame(Rect::from_size(size));
        }
    }

    fn redraw(&mut self) {
        let (Some(renderer), Some(fonts)) = (self.renderer.as_mut(), self.fonts.as_mut()) else {
            return;
        };
        maxgus_core::draw(&self.editor, &mut self.surface);
        let cursor = self.editor.cursor_position();
        let frame = crate::quads::build(
            &self.surface,
            fonts,
            &self.settings.palette,
            self.scroll.pixels(),
            Some(cursor),
        );
        if fonts.atlas().is_dirty() {
            let atlas = fonts.atlas();
            let (width, height) = (atlas.width(), atlas.height());
            let pixels = atlas.pixels().to_vec();
            renderer.upload_atlas(width, height, &pixels);
            fonts.atlas_mut().mark_uploaded();
        }
        if let Err(error) = renderer.draw(&frame) {
            tracing::warn!("frame not drawn: {error}");
        }
    }

    /// A key, and everything that has to happen around one.
    fn on_key(&mut self, key: maxgus_keys::Key) {
        // A keyboard motion owns the view: an animation still running would
        // drag the text away from where the motion just put it.
        self.scroll.settle();
        self.dispatcher.handle_key(&mut self.editor, key);
        self.pump();
    }

    /// Puts point where the pointer is.
    fn on_click(&mut self) {
        let metrics = self.metrics();
        let size = self.surface.size();
        let (column, row) = crate::mouse::cell_at(
            self.pointer.0,
            self.pointer.1,
            metrics,
            size.width,
            size.height,
            self.scroll.pixels(),
        );
        self.editor.point_at_cell(column, row);
        self.pump();
    }

    /// The clipboard, which a window has and a terminal has to be asked for.
    fn paste(&mut self) {
        let Some(text) = self
            .clipboard
            .as_mut()
            .and_then(|clipboard| clipboard.get_text().ok())
        else {
            return;
        };
        if self.editor.minibuffer.is_active() {
            self.editor.minibuffer.insert(&text.replace('\n', " "));
        } else if let Err(error) = self
            .editor
            .with_current_buffer(|b| b.insert_at_point(&text))
        {
            self.editor.error(error.to_string());
        }
        self.editor.follow_point();
        self.pump();
    }

    fn copy(&mut self) {
        let Some(text) = self.editor.region_text() else {
            return;
        };
        if let Some(clipboard) = self.clipboard.as_mut() {
            let _ = clipboard.set_text(text);
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title(self.settings.title.clone())
            .with_inner_size(winit::dpi::LogicalSize::new(1100.0, 720.0));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.failure = Some(error.into());
                event_loop.exit();
                return;
            }
        };
        let fonts = match Fonts::load(&self.settings.font, self.settings.font_size) {
            Ok(fonts) => fonts,
            Err(error) => {
                self.failure = Some(error);
                event_loop.exit();
                return;
            }
        };
        let renderer = pollster::block_on(Renderer::new(
            window.clone(),
            self.settings.palette.background,
        ));
        match renderer {
            Ok(renderer) => {
                let size = window.inner_size();
                self.fonts = Some(fonts);
                self.renderer = Some(renderer);
                self.window = Some(window);
                self.fit(size.width, size.height);
                self.pump();
            }
            Err(error) => {
                self.failure = Some(error);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.editor.quit = true;
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width, size.height);
                }
                self.fit(size.width, size.height);
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let Some(key) =
                    crate::keys::translate(event.state, &event.logical_key, self.modifiers)
                {
                    self.on_key(key);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = (position.x, position.y);
                if self.selecting {
                    let metrics = self.metrics();
                    let size = self.surface.size();
                    let (column, row) = crate::mouse::cell_at(
                        self.pointer.0,
                        self.pointer.1,
                        metrics,
                        size.width,
                        size.height,
                        self.scroll.pixels(),
                    );
                    self.editor.extend_to_cell(column, row);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => match (button, state) {
                (MouseButton::Left, ElementState::Pressed) => {
                    self.selecting = true;
                    self.on_click();
                    self.editor.set_mark_here();
                }
                (MouseButton::Left, ElementState::Released) => {
                    self.selecting = false;
                    // X11's habit, and a useful one: what was just selected is
                    // there to be pasted with the middle button.
                    self.copy();
                }
                // The middle button pastes, as it does everywhere else.
                (MouseButton::Middle, ElementState::Pressed) => self.paste(),
                _ => {}
            },
            WindowEvent::MouseWheel { delta, .. } => {
                let pixels = crate::mouse::wheel_pixels(delta, self.metrics().height);
                self.scroll.nudge(pixels);
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
        if self.editor.quit {
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.editor.quit {
            event_loop.exit();
            return;
        }
        // Whatever the executor finished while the window was quiet.
        self.pump();
        // The scroll animation, a frame at a time. Crossing a line moves the
        // window the way `C-v` would, and the remainder is drawn as a shift.
        let lines = self.scroll.step(self.metrics().height);
        if lines != 0 {
            self.editor.scroll_lines(lines);
        }
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}
