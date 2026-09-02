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
    mut editor: Editor,
    dispatcher: Dispatcher,
    settings: Settings,
    tasks: std::sync::mpsc::Sender<Task>,
    results_in: mpsc::Receiver<TaskResult>,
) -> Result<()> {
    settle_the_beacon(&mut editor.settings);
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
        backdrop: Surface::new(Size::new(1, 1)),
        scroll: Scroll::new(),
        incoming: None,
        cursor: crate::cursor::Cursor::new(),
        vfx: crate::vfx::Vfx::new(),
        modifiers: ModifiersState::empty(),
        pointer: (0.0, 0.0),
        selecting: false,
        scrolling: None,
        pending: None,
        idle_at: None,
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

/// A rectangle with `reach` cells of margin, clipped at nothing: what a
/// blur behind a popup has to be drawn over, since a blur reads outside the
/// shape it fills.
fn grow(area: Rect, reach: u16) -> Rect {
    Rect::new(
        area.x.saturating_sub(reach),
        area.y.saturating_sub(reach),
        area.width + reach * 2,
        area.height + reach * 2,
    )
}

/// The cursor's effects, as the configuration asks for them.
///
/// Read afresh each frame rather than captured at startup, because
/// `cursor-vfx` can change under the window the way any setting can.
fn vfx_settings(settings: &maxgus_config::Settings) -> crate::vfx::Settings {
    let percent = |n: usize| n as f32 / 100.0;
    let seconds = |ms: usize| ms as f32 / 1000.0;
    crate::vfx::Settings {
        // An unknown name was reported when the file was read; here it is
        // simply nothing, which is what it draws.
        mode: crate::vfx::Mode::parse(&settings.cursor_vfx).unwrap_or_default(),
        opacity: percent(settings.cursor_vfx_opacity),
        particle_lifetime: seconds(settings.cursor_vfx_particle_lifetime_ms),
        highlight_lifetime: seconds(settings.cursor_vfx_highlight_lifetime_ms),
        density: percent(settings.cursor_vfx_particle_density),
        speed: settings.cursor_vfx_particle_speed as f32,
        phase: percent(settings.cursor_vfx_particle_phase),
        curl: percent(settings.cursor_vfx_particle_curl),
    }
}

/// Puts the beacon away when the cursor animates.
///
/// They are the same job: after point has jumped, say where it went. The
/// beacon does it by lighting the place it landed, because a terminal cannot
/// show it travelling. A window can, so it shows it travelling instead —
/// and the light is the half of the answer that only exists because the
/// other half was impossible. Both at once is the question answered twice,
/// with the eye pulled two ways.
///
/// Only in a window, and only in this direction: `cursor-animation-ms=0`
/// gives the beacon back, and the terminal front end never asks.
fn settle_the_beacon(settings: &mut maxgus_config::Settings) {
    if settings.cursor_animation_ms > 0 {
        settings.beacon = false;
    }
}

/// What a set of cached incoming lines was fetched for: the window, its
/// `top_line`, which edge they arrive at, and how many were asked for.
type IncomingKey = (maxgus_core::WindowId, usize, isize, usize);

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
    /// The frame without the things floating over it, which is what a blur
    /// behind a popup is a blur *of*.
    backdrop: Surface,
    scroll: Scroll,
    /// The block, and where it is on its way to.
    cursor: crate::cursor::Cursor,
    /// What it leaves behind it, when it has been asked to leave anything.
    vfx: crate::vfx::Vfx,
    modifiers: ModifiersState,
    pointer: (f64, f64),
    selecting: bool,
    /// The window the wheel is scrolling, which is the one under the pointer
    /// rather than the one being typed into.
    scrolling: Option<maxgus_core::WindowId>,
    /// The lines just past the edge of the window being scrolled, and what
    /// they were fetched for: the window, where its view is, which way it
    /// is going, and how many were asked for.
    ///
    /// Fetching them means drawing the whole frame again into a scratch
    /// surface. They do not change while a slide runs — the view has
    /// already moved and the buffer cannot change without a key — so doing
    /// it every frame, which is what this did, was a second full redisplay
    /// per frame for the whole length of every scroll.
    incoming: Option<(IncomingKey, Vec<Vec<maxgus_tui::Cell>>)>,
    /// A half-typed key sequence, and when it was half-typed.
    ///
    /// The echo area and the which-key panel each wait their own pause
    /// before appearing, and both are measured from here.
    pending: Option<(String, std::time::Instant)>,
    /// When the idle work is due, if it is owed. Re-highlighting and telling
    /// the language server what changed both wait for typing to stop.
    idle_at: Option<std::time::Instant>,
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
            // Highlighting arriving mid-slide changes how those lines are
            // drawn, and the ones already fetched were drawn without it.
            self.incoming = None;
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
        // As many lines as the gap is deep. A wheel notch never opens more
        // than one, but a command that moved the view several lines slides
        // the last few of them and opens a gap that deep.
        let deep = (pixels.abs() / self.metrics().height).ceil().max(1.0) as usize;
        let top = self.editor.windows.get(id).map_or(0, |w| w.top_line);
        // Reuse what was fetched for this same view unless it is no longer
        // enough. A slide only ever gets shallower as it settles, so the
        // first frame of one asks for the most and every frame after asks
        // for nothing.
        let stale = match self.incoming.as_ref() {
            Some(((had_id, had_top, had_way, had_deep), _)) => {
                (*had_id, *had_top, *had_way) != (id, top, direction) || *had_deep < deep
            }
            None => true,
        };
        if stale {
            self.incoming =
                maxgus_core::edge_rows(&mut self.editor, id, direction, deep, &mut self.scratch)
                    .map(|rows| ((id, top, direction, deep), rows));
        }
        // Counted from the deepest the gap ever got, so the rows keep the
        // places they were fetched for as it closes.
        let incoming = self.incoming.as_ref().map(|(_, rows)| {
            let row = match direction > 0 {
                true => area.height as i32,
                false => -(rows.len() as i32),
            };
            (row, rows.clone())
        });
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
        // Drawn in its two halves — what the windows hold, then what floats
        // over them — because a blur behind a popup needs what is behind it,
        // and once the popup is in the grid there is nothing behind it. The
        // two halves cost what the one did; the copy between them is what
        // the blur is bought with, and only when there is a popup to blur
        // behind.
        maxgus_core::draw_background(&self.editor, &mut self.surface);
        let blurring =
            self.editor.settings.floating_blur && self.editor.settings.floating_blur_radius > 0;
        if blurring {
            self.backdrop.resize(self.surface.size());
            self.backdrop.copy_from(&self.surface);
        }
        let floating = maxgus_core::draw_floating(&self.editor, &mut self.surface);
        let at = self.editor.cursor_position();
        let metrics = self.metrics();
        let settings = &self.editor.settings;
        self.cursor.go_to(
            crate::cursor::Cell {
                x: at.0 as f32 * metrics.width,
                y: at.1 as f32 * metrics.height,
                width: metrics.width,
                height: metrics.height,
            },
            settings.cursor_animation_ms,
            settings.cursor_short_animation_ms,
            settings.cursor_trail,
        );
        // While the block is in transit it *is* the cursor, and the cell it
        // is heading for is drawn like any other. Doing both would be a
        // cursor in two places, one of which is not where point is.
        let (cell, smear) = match self.cursor.is_moving() {
            true => (None, Some(self.cursor.corners())),
            false => (Some(at), None),
        };
        let shift = self.shift();
        let palette = self.settings.palette;
        let ligatures = match self.editor.settings.ligatures {
            true => maxgus_core::render::code_areas(&self.editor),
            false => Vec::new(),
        };
        let dividers = maxgus_core::render::divided_windows(&self.editor);
        let vfx = vfx_settings(&self.editor.settings);

        let (Some(renderer), Some(fonts)) = (self.renderer.as_mut(), self.fonts.as_mut()) else {
            return;
        };
        renderer.background = palette.background;
        let opacity = self.editor.settings.floating_opacity as f32 / 100.0;
        let blurring = blurring && !floating.is_empty();
        let look = crate::quads::Look {
            palette: &palette,
            shift: shift.as_ref(),
            cursor: cell,
            smear,
            ligatures: &ligatures,
            dividers: &dividers,
            floating: &floating,
            only: &[],
            // Only where there is something blurred underneath to show.
            translucent: match blurring {
                true => &floating,
                false => &[],
            },
            opacity,
        };
        let mut frame = crate::quads::build(&self.surface, fonts, &look);
        self.vfx.draw(
            &mut frame,
            palette.cursor,
            (metrics.width, metrics.height),
            &vfx,
        );
        if fonts.atlas().is_dirty() {
            let atlas = fonts.atlas();
            let (width, height) = (atlas.width(), atlas.height());
            let pixels = atlas.pixels().to_vec();
            renderer.upload_atlas(width, height, &pixels);
            fonts.atlas_mut().mark_uploaded();
        }
        // The backdrop, drawn only near the popups: a blur reaches no
        // further than its radius, so the rest of the screen would be drawn
        // a second time only to be thrown away.
        let radius = self.editor.settings.floating_blur_radius as f32;
        let (backdrop, areas) = match blurring {
            true => {
                let reach = (radius / metrics.width).ceil() as u16 + 1;
                let near: Vec<Rect> = floating.iter().map(|area| grow(*area, reach)).collect();
                let look = crate::quads::Look {
                    only: &near,
                    translucent: &[],
                    opacity: 1.0,
                    cursor: None,
                    smear: None,
                    ..look
                };
                let backdrop = crate::quads::build(&self.backdrop, fonts, &look);
                let pixels: Vec<[f32; 4]> = floating
                    .iter()
                    .map(|area| {
                        [
                            area.x as f32 * metrics.width,
                            area.y as f32 * metrics.height,
                            area.width as f32 * metrics.width,
                            area.height as f32 * metrics.height,
                        ]
                    })
                    .collect();
                (Some(backdrop), pixels)
            }
            false => (None, Vec::new()),
        };
        if let Err(error) = renderer.draw_over(&frame, backdrop.as_ref(), &areas, radius) {
            tracing::warn!("frame not drawn: {error}");
        }
    }

    /// Finishes any animation where it stands: the lines it had left to
    /// cross are crossed now, so the view is where the wheel asked for it
    /// rather than part of the way there.
    fn settle(&mut self) {
        self.incoming = None;
        if let Some(id) = self.scrolling.take() {
            let lines = self.scroll.remaining(self.metrics().height);
            if lines != 0 {
                self.editor.scroll_window_lines(id, lines);
            }
        }
        self.scroll.settle();
    }

    /// Where the current window's view is, for noticing that a command moved
    /// it. `None` when there is somehow no current window to ask.
    fn viewpoint(&self) -> (maxgus_core::WindowId, usize) {
        let id = self.editor.windows.current_id();
        let top = self.editor.windows.get(id).map_or(0, |w| w.top_line);
        (id, top)
    }

    /// Slides the drawing in after a command that moved the view.
    ///
    /// The editor has already gone: `top_line` is where the command put it
    /// before anything was drawn. So there is nothing left to ask for, and
    /// what makes it a scroll rather than a jump is starting the drawing
    /// back where the view was and letting it catch up.
    ///
    /// A long jump is not animated in full. `M->` in a long file is a
    /// thousand lines and sliding a thousand lines is an animation to watch
    /// rather than a view to read — so the last few are slid and the rest
    /// simply arrives. `scroll-animation-far-lines` is how many.
    fn slide_after(&mut self, was: (maxgus_core::WindowId, usize)) {
        let settings = &self.editor.settings;
        let (far, ms) = (
            settings.scroll_animation_far_lines,
            settings.smooth_scroll_ms,
        );
        if far == 0 || ms == 0 {
            return;
        }
        let (id, top) = self.viewpoint();
        // A different window is a different view, not a scroll of this one.
        if id != was.0 || top == was.1 {
            return;
        }
        let moved = top as isize - was.1 as isize;
        let slid = moved.signum() * (moved.abs().min(far as isize));
        self.scrolling = None;
        self.scroll.catch_up(slid as f32 * self.metrics().height);
    }

    /// A key, and everything that has to happen around one.
    fn on_key(&mut self, key: maxgus_keys::Key) {
        // A keyboard motion owns the view: an animation still running would
        // drag the text away from where the motion just put it.
        self.settle();
        let was = self.viewpoint();
        let outcome = self.dispatcher.handle_key(&mut self.editor, key);
        self.on_dispatch(&outcome);
        maxgus_core::frontend::after_key(&mut self.editor, &mut self.dispatcher);
        self.slide_after(was);
        // Something changed, so the buffer needs re-highlighting and the
        // language server needs telling — once the typing stops.
        let delay = self.editor.settings.idle_delay_ms.max(1);
        self.idle_at = Some(std::time::Instant::now() + std::time::Duration::from_millis(delay));
        self.dirty = true;
        self.pump();
    }

    /// Half-typed sequences, which the terminal front end handles the same
    /// way: remember one, show it once the hand has stopped.
    fn on_dispatch(&mut self, outcome: &maxgus_core::Dispatch) {
        match outcome {
            maxgus_core::Dispatch::Prefix { echo } => {
                let since = self
                    .pending
                    .as_ref()
                    .map(|(_, at)| *at)
                    .unwrap_or_else(std::time::Instant::now);
                self.pending = Some((echo.clone(), since));
                // Once something is on screen, the rest of the sequence
                // joins it rather than disappearing.
                if self.editor.pending_keys.is_some() {
                    self.editor.pending_keys = Some(echo.clone());
                }
                if self.editor.which_key.is_some() {
                    self.editor.which_key = Some(echo.clone());
                }
            }
            maxgus_core::Dispatch::Undefined { keys } => {
                self.editor.error(format!("{keys} is undefined"));
                self.forget_pending();
            }
            _ => self.forget_pending(),
        }
    }

    fn forget_pending(&mut self) {
        self.pending = None;
        self.editor.pending_keys = None;
        self.editor.which_key = None;
    }

    /// Shows the echo and the panel once each has waited long enough, and
    /// says when the next of them is due.
    fn pending_deadline(&mut self) -> Option<std::time::Instant> {
        let (keys, since) = self.pending.clone()?;
        let settings = &self.editor.settings;
        let mut next: Option<std::time::Instant> = None;
        let mut due = |after: u64, shown: bool| -> bool {
            let at = since + std::time::Duration::from_millis(after.max(1));
            if shown || at <= std::time::Instant::now() {
                return true;
            }
            next = Some(next.map_or(at, |soonest: std::time::Instant| soonest.min(at)));
            false
        };
        if due(
            settings.echo_keystrokes_ms,
            self.editor.pending_keys.is_some(),
        ) {
            self.editor.pending_keys = Some(keys.clone());
        }
        if settings.which_key
            && due(
                settings.which_key_delay_ms as u64,
                self.editor.which_key.is_some(),
            )
        {
            self.editor.which_key = Some(keys);
        }
        next
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
                // Every cell is somewhere else now. The block did not travel
                // there and should not be drawn as though it had.
                self.cursor.snap();
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
                self.cursor.snap();
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
        // And the block, on its way to where point went. Advanced by real
        // time like the others, so the setting is the same speed whatever
        // the display refreshes at.
        if self.cursor.is_moving() {
            self.cursor.step(since);
            self.dirty = true;
        }
        // And whatever it is leaving behind, which outlives the journey:
        // the particles are still going out after the block has landed.
        let vfx = vfx_settings(&self.editor.settings);
        if vfx.mode != crate::vfx::Mode::None || self.vfx.is_running() {
            let metrics = self.metrics();
            let at = self.cursor.destination();
            let centre = [(at[0][0] + at[3][0]) * 0.5, (at[0][1] + at[3][1]) * 0.5];
            let was_running = self.vfx.is_running();
            self.vfx
                .step(since, centre, (metrics.width, metrics.height), &vfx);
            if self.vfx.is_running() || was_running {
                self.dirty = true;
            }
        }
        let cursor_moving = self.cursor.is_moving() || self.vfx.is_running();
        // A half-typed sequence owes the echo and the panel a frame each,
        // at their own times.
        let was = (
            self.editor.pending_keys.clone(),
            self.editor.which_key.clone(),
        );
        let mut deadline = self.pending_deadline();
        if (
            self.editor.pending_keys.clone(),
            self.editor.which_key.clone(),
        ) != was
        {
            self.dirty = true;
        }
        // And the work that waits for the typing to stop.
        match self.idle_at {
            Some(at) if at <= now => {
                self.idle_at = None;
                maxgus_core::frontend::on_idle(&mut self.editor);
                self.pump();
            }
            Some(at) => {
                deadline = Some(deadline.map_or(at, |soonest: std::time::Instant| soonest.min(at)))
            }
            None => {}
        }
        // An animation owes a frame whether or not anything was typed;
        // everything else waits to be woken.
        let animating = moving || cursor_moving || self.editor.beacon.is_some();
        event_loop.set_control_flow(match (animating, deadline) {
            (true, _) => ControlFlow::Poll,
            (false, Some(at)) => ControlFlow::WaitUntil(at),
            (false, None) => ControlFlow::Wait,
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
    fn the_animated_cursor_puts_the_beacon_away() {
        // Both would answer "point went there" at once, in two different
        // ways, in two different places.
        let mut settings = maxgus_config::Settings {
            beacon: true,
            cursor_animation_ms: 90,
            ..Default::default()
        };
        settle_the_beacon(&mut settings);
        assert!(!settings.beacon, "the light is still on");
    }

    #[test]
    fn turning_the_cursor_animation_off_gives_the_beacon_back() {
        // Which is the only way to get it in a window, and has to work:
        // otherwise `cursor-animation-ms=0` is a setting that takes a
        // feature away and gives nothing back.
        let mut settings = maxgus_config::Settings {
            beacon: true,
            cursor_animation_ms: 0,
            ..Default::default()
        };
        settle_the_beacon(&mut settings);
        assert!(settings.beacon, "the beacon was put away for nothing");
    }

    #[test]
    fn a_beacon_nobody_asked_for_is_not_turned_on() {
        let mut settings = maxgus_config::Settings {
            beacon: false,
            cursor_animation_ms: 0,
            ..Default::default()
        };
        settle_the_beacon(&mut settings);
        assert!(!settings.beacon);
    }

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
