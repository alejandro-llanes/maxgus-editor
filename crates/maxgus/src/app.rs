//! The event loop.
//!
//! One `select!` over two sources — the terminal's key events and the results
//! coming back from the executor — with a redraw after each. Every command runs
//! synchronously between those awaits, so editor state is never touched from
//! two places at once, and nothing that touches the world is done here.

use anyhow::Result;
use futures_util::StreamExt;
use maxgus_core::{Dispatcher, Editor, Task, TaskResult};
use maxgus_tui::{Rect, Size, Surface, Suspension, Terminal, TuiEvent, render::Renderer};
use std::time::Duration;
use tokio::sync::mpsc;

/// Drives the editor until it is asked to leave.
pub struct App {
    editor: Editor,
    dispatcher: Dispatcher,
    terminal: Terminal,
    renderer: Renderer,
    /// The frame being drawn into, kept between redraws so it can be diffed.
    surface: Surface,
    tasks: mpsc::UnboundedSender<Task>,
    results: mpsc::UnboundedReceiver<TaskResult>,
    /// True while an idle pass is owed: something changed and the expensive
    /// follow-up work has not run yet.
    idle_owed: bool,
    /// The half-typed key sequence, held back until the user has hesitated
    /// long enough for showing it to be helpful rather than distracting.
    unechoed_prefix: Option<String>,
    /// True until the startup time has been announced or given up on.
    ///
    /// The measurement is taken before the loop starts; only the saying of it
    /// waits, for the files named on the command line to finish reporting
    /// themselves. A keystroke cancels it: someone already typing does not
    /// want the echo area taken from under them.
    greeting_owed: bool,
}

impl App {
    pub fn new(
        editor: Editor,
        dispatcher: Dispatcher,
        terminal: Terminal,
        tasks: mpsc::UnboundedSender<Task>,
        results: mpsc::UnboundedReceiver<TaskResult>,
    ) -> App {
        let size = terminal.size();
        let depth = maxgus_faces::ColorDepth::from_env(
            std::env::var("COLORTERM").ok().as_deref(),
            std::env::var("TERM").ok().as_deref(),
        );
        App {
            editor,
            dispatcher,
            terminal,
            renderer: Renderer::new(size, depth),
            surface: Surface::new(size),
            tasks,
            results,
            idle_owed: false,
            unechoed_prefix: None,
            greeting_owed: true,
        }
    }

    /// How long a half-typed key sequence waits before being shown.
    fn echo_delay(&self) -> Duration {
        Duration::from_millis(self.editor.settings.echo_keystrokes_ms.max(1))
    }

    /// How long after the last keystroke the idle work runs.
    fn idle_delay(&self) -> Duration {
        Duration::from_millis(self.editor.settings.idle_delay_ms.max(1))
    }

    /// Long enough for the files named on the command line to have been read
    /// and to have said so, short enough to be the first thing seen.
    const GREETING_DELAY: Duration = Duration::from_millis(200);

    /// Says how long the editor took, unless something worth more is showing.
    fn announce_startup(&mut self) {
        self.greeting_owed = false;
        if let Some(text) = self.editor.startup_message() {
            // A configuration problem outranks a boast about speed.
            self.editor.message_unless_error(text);
        }
    }

    /// Runs until `C-x C-c`.
    pub async fn run(mut self) -> Result<()> {
        let mut events = Terminal::events();
        self.terminal
            .set_cursor_blinking(self.editor.settings.blink_cursor)?;
        self.resize(self.terminal.size());
        // Whatever startup queued — the files named on the command line, the
        // first tree read — has to reach the executor before the loop blocks
        // waiting for a key, or none of it would happen until one was pressed.
        self.drain_tasks();
        self.redraw()?;

        // Re-parsing and talking to a language server are too expensive to do
        // on every keystroke, so they wait until the typing stops. The timer
        // starts far out and is pulled in whenever something changes.
        let idle = tokio::time::sleep(Duration::from_secs(86_400));
        tokio::pin!(idle);
        // A second timer for the key echo: Emacs waits before showing a
        // half-typed sequence, so a fluent `C-x C-s` never flashes anything.
        let echo = tokio::time::sleep(Duration::from_secs(86_400));
        tokio::pin!(echo);
        let greeting = tokio::time::sleep(App::GREETING_DELAY);
        tokio::pin!(greeting);

        while !self.editor.quit {
            tokio::select! {
                // Input and results are equally urgent; taking them in a
                // random order stops either starving the other.
                Some(event) = events.next() => {
                    self.on_event(event)?;
                    self.idle_owed = true;
                    // Someone is typing: the echo area is theirs now.
                    self.greeting_owed = false;
                    idle.as_mut().reset(tokio::time::Instant::now() + self.idle_delay());
                    match self.unechoed_prefix.is_some() {
                        true => echo.as_mut().reset(
                            tokio::time::Instant::now() + self.echo_delay(),
                        ),
                        // The sequence completed, so nothing is waiting.
                        false => echo.as_mut().reset(
                            tokio::time::Instant::now() + Duration::from_secs(86_400),
                        ),
                    }
                }
                Some(result) = self.results.recv() => self.on_result(result),
                () = &mut echo, if self.unechoed_prefix.is_some() => {
                    // The user hesitated; show them where they are.
                    self.editor.pending_keys = self.unechoed_prefix.clone();
                    echo.as_mut().reset(
                        tokio::time::Instant::now() + Duration::from_secs(86_400),
                    );
                }
                () = &mut greeting, if self.greeting_owed => self.announce_startup(),
                () = &mut idle, if self.idle_owed => {
                    self.on_idle();
                    // Park the timer until something changes again.
                    idle.as_mut().reset(
                        tokio::time::Instant::now() + Duration::from_secs(86_400),
                    );
                }
                else => break,
            }
            self.after_turn()?;
        }
        // What was open, for next time. Queued and run before the executor
        // is dropped, so a session is written even though leaving is
        // otherwise immediate.
        if self.editor.settings.session {
            let _ = self
                .dispatcher
                .execute(&mut self.editor, "save-session", None);
            for task in self.editor.tasks.drain() {
                let _ = self.tasks.send(task);
            }
            // The write is a round trip through the executor; waiting for it
            // is the difference between a session and a truncated one.
            let _ = tokio::time::timeout(Duration::from_secs(2), self.results.recv()).await;
        }
        self.terminal.restore()?;
        Ok(())
    }

    /// Handles one terminal event.
    fn on_event(&mut self, event: TuiEvent) -> Result<()> {
        match event {
            TuiEvent::Key(key) => {
                let outcome = self.dispatcher.handle_key(&mut self.editor, key);
                self.echo_pending(&outcome);
            }
            TuiEvent::Resize(size) => self.resize(size),
            // A paste is inserted literally, so a pasted `C-x` is text.
            TuiEvent::Paste(text) => {
                if self.editor.minibuffer.is_active() {
                    self.editor.minibuffer.insert(&text.replace('\n', " "));
                } else if let Err(error) = self
                    .editor
                    .with_current_buffer(|b| b.insert_at_point(&text))
                {
                    self.editor.error(error.to_string());
                }
                self.editor.follow_point();
            }
            TuiEvent::FocusGained | TuiEvent::FocusLost => {}
        }
        Ok(())
    }

    /// Records a half-typed key sequence, to be shown once the user pauses.
    fn echo_pending(&mut self, outcome: &maxgus_core::Dispatch) {
        match outcome {
            maxgus_core::Dispatch::Prefix { echo } => {
                self.unechoed_prefix = Some(echo.clone());
                // Once something is on screen, the rest of the sequence joins
                // it immediately rather than disappearing.
                if self.editor.pending_keys.is_some() {
                    self.editor.pending_keys = Some(echo.clone());
                }
            }
            maxgus_core::Dispatch::Undefined { keys } => {
                self.editor.error(format!("{keys} is undefined"));
                self.unechoed_prefix = None;
                self.editor.pending_keys = None;
            }
            _ => {
                self.unechoed_prefix = None;
                self.editor.pending_keys = None;
            }
        }
    }

    fn on_result(&mut self, result: TaskResult) {
        // Shell output is the one result the editor cannot fold in by itself,
        // because where it goes depends on how the command was invoked.
        if let TaskResult::ShellOutput {
            command,
            output,
            insert_at,
            ..
        } = &result
        {
            match insert_at {
                Some((buffer, offset)) => {
                    let (buffer, offset) = (*buffer, *offset);
                    if let Some(target) = self.editor.buffers.get_mut(buffer) {
                        let at = offset.min(target.len_chars());
                        let _ = target.insert(at, output);
                    }
                }
                None => {
                    let (command, output) = (command.clone(), output.clone());
                    let _ = maxgus_core::commands::misc::show_shell_output(
                        &mut self.editor,
                        &command,
                        &output,
                    );
                }
            }
            return;
        }
        #[cfg(feature = "lsp")]
        if let TaskResult::LspResponse { .. } | TaskResult::LspApplyEdit { .. } = &result {
            self.editor.apply_lsp_response(result);
            return;
        }
        if let Err(error) = self.editor.apply_task_result(result) {
            self.editor.error(error.to_string());
        }
    }

    /// The bookkeeping that happens after every event: replaying macros,
    /// draining queued work, re-parsing, and redrawing.
    fn after_turn(&mut self) -> Result<()> {
        self.replay_macro();
        self.follow_tree();
        self.drain_tasks();
        if self.editor.suspend {
            self.suspend()?;
        }
        if !self.editor.quit {
            self.redraw()?;
        }
        Ok(())
    }

    /// Replays the last keyboard macro, if a command asked for it.
    fn replay_macro(&mut self) {
        let repeats = std::mem::take(&mut self.editor.macro_repeats);
        if repeats == 0 {
            return;
        }
        let keys = self.editor.last_macro.clone();
        self.editor.replaying_macro = true;
        for _ in 0..repeats {
            for key in &keys {
                self.dispatcher.handle_key(&mut self.editor, *key);
            }
        }
        self.editor.replaying_macro = false;
    }

    /// The work that waits for typing to stop: re-highlighting the buffer and
    /// telling the language server what changed.
    fn on_idle(&mut self) {
        self.idle_owed = false;
        let id = self.editor.current_buffer_id();
        #[cfg(feature = "syntax")]
        if self.editor.highlights_are_stale(id) {
            self.editor.request_highlighting(id);
        }
        self.editor.sync_language_server(id);
    }

    /// Keeps the tree cursor on the file being edited, when follow mode is on.
    fn follow_tree(&mut self) {
        if !self.editor.tree_follow || self.editor.tree_window.is_none() {
            return;
        }
        // Only when the user is editing, not while they walk the tree itself.
        if Some(self.editor.windows.current_id()) == self.editor.tree_window {
            return;
        }
        let Some(path) = self
            .editor
            .current_buffer()
            .path()
            .map(std::path::Path::to_path_buf)
        else {
            return;
        };
        if self.editor.tree.iter().any(|n| n.path == path) {
            self.editor.select_tree_path(&path);
        } else {
            self.editor
                .spawn(Task::Tree(maxgus_core::TreeAction::Reveal(path)));
        }
    }

    /// Hands everything the commands queued to the executor.
    fn drain_tasks(&mut self) {
        for task in self.editor.tasks.drain() {
            if self.tasks.send(task).is_err() {
                self.editor.error("Background worker has stopped");
                break;
            }
        }
    }

    fn resize(&mut self, size: Size) {
        self.terminal.set_size(size);
        self.editor.set_frame(Rect::from_size(size));
        self.surface.resize(size);
        // The terminal's contents are unknown after a resize.
        self.renderer.invalidate();
    }

    fn redraw(&mut self) -> Result<()> {
        if !self.terminal.is_usable() {
            return Ok(());
        }
        maxgus_core::draw(&self.editor, &mut self.surface);
        let surface = self.surface.clone();
        self.terminal.hide_cursor()?;
        self.renderer.render(self.terminal.writer(), &surface)?;
        let (x, y) = self.editor.cursor_position();
        self.terminal.place_cursor(x, y)?;
        Ok(())
    }

    /// `C-z`: hands the terminal back, stops the process, and takes the
    /// terminal again when the shell brings the job forward.
    fn suspend(&mut self) -> Result<()> {
        self.editor.suspend = false;
        // The terminal has to be given up before stopping, not after: once the
        // process is stopped nothing here runs, and the shell would get its
        // prompt back inside raw mode on the alternate screen.
        self.terminal.restore()?;
        let outcome = maxgus_tui::job::suspend();

        // Taken again whichever way that went, because `restore` has already
        // handed it over. A window resized while the editor was stopped raised
        // no event — a stopped process is told nothing — so the size is asked
        // for again rather than carried across.
        self.terminal = Terminal::new()?;
        self.terminal
            .set_cursor_blinking(self.editor.settings.blink_cursor)?;
        self.resize(self.terminal.size());
        match outcome {
            // Nothing to say: the screen coming back is the whole answer.
            Ok(Suspension::Resumed) => {}
            Ok(Suspension::NoJobControl) => self
                .editor
                .message("No job control here; nothing could resume the editor"),
            Err(error) => self.editor.error(format!("Cannot suspend: {error}")),
        }
        Ok(())
    }
}
