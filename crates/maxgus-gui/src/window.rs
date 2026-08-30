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

/// How much time an animation is advanced by, given how long it has been.
///
/// The loop sleeps when nothing is happening, so the gap before the frame
/// that follows a keystroke is however long the editor was left alone.
/// Feeding that to an animation runs it to its end before it has been seen:
/// the first notch of the wheel would teleport, and the light beside the
/// cursor would be over before it was ever drawn. A gap longer than a few
/// frames is a wait rather than a frame, and counts as one.
pub fn frame_time(since: std::time::Duration) -> std::time::Duration {
    const LONGEST: std::time::Duration = std::time::Duration::from_millis(50);
    since.min(LONGEST)
}

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
    results_in: mpsc::Receiver<TaskResult>,
) -> Result<()> {
    let event_loop = EventLoop::new()?;
    // Wait rather than poll. Results arrive from the executor on a channel
    // the window system knows nothing about, so a thread forwards them and
    // wakes the loop for each one; an animation asks for `Poll` while it
    // runs and gives it back when it settles. Polling regardless is what
    // made an editor showing a file and doing nothing take a sixth of a
    // core forever.
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    let (woken, results) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(result) = results_in.recv() {
            if woken.send(result).is_err() {
                return;
            }
            if proxy.send_event(()).is_err() {
                return;
            }
        }
    });
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
        scratch: Surface::new(Size::new(1, 1)),
        scroll: Scroll::new(),
        modifiers: ModifiersState::empty(),
        pointer: (0.0, 0.0),
        selecting: false,
        scrolling: None,
        clipboard: arboard::Clipboard::new().ok(),
        failure: None,
        last_frame: None,
        dirty: true,
        title: None,
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
    /// Somewhere to draw the line sliding in, kept rather than allocated
    /// every frame of an animation.
    scratch: Surface,
    scroll: Scroll,
    modifiers: ModifiersState,
    pointer: (f64, f64),
    selecting: bool,
    /// The window the wheel is scrolling, which is the one under the pointer
    /// rather than the one being typed into.
    scrolling: Option<maxgus_core::WindowId>,
    clipboard: Option<arboard::Clipboard>,
    /// Set when something went wrong badly enough to stop, and reported once
    /// the event loop has given control back.
    failure: Option<anyhow::Error>,
    /// When the last frame was, so the beacon is advanced by real time
    /// rather than by a frame count that depends on the machine.
    last_frame: Option<std::time::Instant>,
    /// Whether anything has happened that the last frame does not show.
    ///
    /// A window that redraws regardless is a window that keeps a core busy
    /// showing a file nobody is touching.
    dirty: bool,
    /// The title the window is wearing, so it is only set when it changes.
    title: Option<String>,
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
            self.dirty = true;
        }
        // Applying a result can queue more work — a file read asks for
        // highlighting — and nothing else would send it.
        for task in self.editor.tasks.drain() {
            let _ = self.tasks.send(task);
        }
    }

    fn apply(&mut self, result: TaskResult) {
        #[cfg(feature = "full")]
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
            self.dirty = true;
        }
    }

    /// What of the screen is sliding, and by how much.
    ///
    /// Only the current window's text: everything else — its mode line, the
    /// echo area, the file tree, any other window — holds still, which is
    /// the difference between a window that scrolls and one that judders.
    fn shift(&mut self) -> Option<crate::quads::Shift> {
        let pixels = self.scroll.pixels();
        if pixels == 0.0 {
            return None;
        }
        let id = self
            .scrolling
            .unwrap_or_else(|| self.editor.windows.current_id());
        let area = maxgus_core::text_area(&self.editor, id)?;
        // Which edge the gap opens at: text drawn higher leaves one at the
        // bottom, where the next line down is arriving.
        let direction = match pixels > 0.0 {
            true => 1,
            false => -1,
        };
        let row = match direction > 0 {
            true => area.height as i32,
            false => -1,
        };
        let incoming = maxgus_core::edge_row(&mut self.editor, id, direction, &mut self.scratch)
            .map(|cells| (row, cells));
        Some(crate::quads::Shift {
            area,
            pixels,
            incoming,
        })
    }

    /// Names the window after what is in it, the way every other program
    /// does: a taskbar full of windows called `maxgus` names nothing.
    fn retitle(&mut self) {
        let name = self.editor.current_buffer().name().to_string();
        let modified = match self.editor.current_buffer().is_modified() {
            true => "* ",
            false => "",
        };
        let title = format!("{modified}{name} — {}", self.settings.title);
        if self.title.as_deref() != Some(title.as_str())
            && let Some(window) = self.window.as_ref()
        {
            window.set_title(&title);
            self.title = Some(title);
        }
    }

    fn redraw(&mut self) {
        if self.renderer.is_none() || self.fonts.is_none() {
            return;
        }
        self.retitle();
        // The theme can change under the window — `load-theme` is a command
        // like any other — so the palette is what the theme says now rather
        // than what it said when the window opened.
        self.settings.palette = crate::quads::Palette::of(&self.editor.theme);
        maxgus_core::draw(&self.editor, &mut self.surface);
        let cursor = self.editor.cursor_position();
        let shift = self.shift();
        let palette = self.settings.palette;

        let (Some(renderer), Some(fonts)) = (self.renderer.as_mut(), self.fonts.as_mut()) else {
            return;
        };
        renderer.background = palette.background;
        let frame =
            crate::quads::build(&self.surface, fonts, &palette, shift.as_ref(), Some(cursor));
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

    /// Finishes any animation where it stands: the lines it had left to
    /// cross are crossed now, so the view is where the wheel asked for it
    /// rather than part of the way there.
    fn settle(&mut self) {
        if let Some(id) = self.scrolling.take() {
            let lines = self.scroll.remaining(self.metrics().height);
            if lines != 0 {
                self.editor.scroll_window_lines(id, lines);
            }
        }
        self.scroll.settle();
    }

    /// A key, and everything that has to happen around one.
    fn on_key(&mut self, key: maxgus_keys::Key) {
        // A keyboard motion owns the view: an animation still running would
        // drag the text away from where the motion just put it.
        self.settle();
        self.dispatcher.handle_key(&mut self.editor, key);
        self.dirty = true;
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
        self.dirty = true;
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
        self.dirty = true;
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
        // The size in the config is points on a normal display; the window
        // is measured in physical pixels, so a display that reports a scale
        // wants the glyphs rasterised that much larger or the text comes out
        // half-size on it.
        let scale = window.scale_factor() as f32;
        let fonts = match Fonts::load(&self.settings.font, self.settings.font_size * scale) {
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
                // The same command `C-x C-c` runs, so the same thing
                // happens: unsaved work refuses to be thrown away and says
                // which buffers are holding it. Setting `quit` here — which
                // is what this did — closed the window over the top of it.
                self.dispatcher
                    .execute(&mut self.editor, "save-buffers-kill-terminal", None);
                self.dirty = true;
                self.pump();
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width, size.height);
                }
                self.fit(size.width, size.height);
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                // Moved to a display with a different scale: the glyphs have
                // to be cut again at the new size.
                match Fonts::load(
                    &self.settings.font,
                    self.settings.font_size * scale_factor as f32,
                ) {
                    Ok(fonts) => self.fonts = Some(fonts),
                    Err(error) => tracing::warn!("the font would not reload: {error}"),
                }
                if let Some(window) = self.window.as_ref() {
                    let size = window.inner_size();
                    self.fit(size.width, size.height);
                }
                self.dirty = true;
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
                    self.dirty = true;
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
                // The window under the pointer, not the one being typed
                // into: a wheel over the file tree scrolls the file tree.
                let metrics = self.metrics();
                let size = self.surface.size();
                let (column, row) = crate::mouse::cell_at(
                    self.pointer.0,
                    self.pointer.1,
                    metrics,
                    size.width,
                    size.height,
                    0.0,
                );
                let under = self
                    .editor
                    .windows
                    .window_at(column, row)
                    .or_else(|| Some(self.editor.windows.current_id()));
                // An animation belongs to one window; moving to another
                // finishes the first where it stands rather than dragging it.
                if self.scrolling != under {
                    self.settle();
                    self.scrolling = under;
                }
                let per_notch = self.editor.settings.mouse_wheel_lines;
                self.scroll
                    .nudge(crate::mouse::wheel_pixels(delta, metrics.height, per_notch));
                self.dirty = true;
            }
            WindowEvent::RedrawRequested => {
                self.dirty = false;
                self.redraw();
            }
            _ => {}
        }
        if self.editor.quit {
            event_loop.exit();
        }
    }

    /// Woken because the executor finished something.
    fn user_event(&mut self, _: &ActiveEventLoop, _: ()) {
        self.pump();
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.editor.quit {
            event_loop.exit();
            return;
        }
        // Whatever the executor finished while the window was quiet.
        self.pump();
        // The light beside the cursor, by however long the last frame took.
        let now = std::time::Instant::now();
        let since = frame_time(
            self.last_frame
                .map(|last| now.duration_since(last))
                .unwrap_or_default(),
        );
        self.last_frame = Some(now);
        if self.editor.advance_beacon(since) {
            self.dirty = true;
        }
        // The scroll animation, a frame at a time. Crossing a line moves the
        // window the way `C-v` would, and the remainder is drawn as a shift.
        let moving = self.scroll.is_moving() || self.scroll.pixels() != 0.0;
        if moving {
            let settle = self.editor.settings.smooth_scroll_ms;
            let lines = self.scroll.step(self.metrics().height, since, settle);
            if lines != 0 {
                let id = self
                    .scrolling
                    .unwrap_or_else(|| self.editor.windows.current_id());
                self.editor.scroll_window_lines(id, lines);
            }
            self.dirty = true;
        }
        // An animation owes a frame whether or not anything was typed;
        // everything else waits to be woken.
        event_loop.set_control_flow(match moving || self.editor.beacon.is_some() {
            true => ControlFlow::Poll,
            false => ControlFlow::Wait,
        });
        if self.dirty
            && let Some(window) = self.window.as_ref()
        {
            window.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn an_ordinary_frame_is_taken_at_its_word() {
        for frame in [1u64, 7, 16, 33, 50] {
            let since = Duration::from_millis(frame);
            assert_eq!(frame_time(since), since, "{frame}ms is a frame");
        }
    }

    #[test]
    fn the_wait_before_the_first_frame_is_not_a_frame() {
        // The window sleeps when nothing is happening. Handing the sleep to
        // an animation as though it were a frame ran the animation out
        // before anyone saw it: the first turn of the wheel jumped, and the
        // beacon was over by the time it was drawn.
        let slept = frame_time(Duration::from_secs(30));
        assert!(
            slept <= Duration::from_millis(50),
            "half a minute of idling counted as a frame of {slept:?}"
        );
        assert!(slept > Duration::ZERO, "and it is still a frame");
    }
}
