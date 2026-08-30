//! Smoke tests against the real binary in a real terminal.
//!
//! Everything else in this workspace tests the editor as a library. These are
//! the only tests that run `maxgus` itself: they open a pseudo-terminal, start
//! the binary in it, send keystrokes, and interpret what it draws. That covers
//! the parts nothing else reaches — argument handling, terminal setup, the
//! event loop, and putting the terminal back on the way out.
//!
//! A pseudo-terminal and a stop signal are POSIX ideas, so these run on
//! Unix only. Everything they cover is platform-independent code reached
//! through a platform-specific door.
#![cfg(unix)]

use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The terminal the editor is given.
const COLUMNS: u16 = 80;
const ROWS: u16 = 24;

/// How long to wait for the editor to settle after a keystroke.
const SETTLE: Duration = Duration::from_millis(250);
/// How long to wait for it to exit before giving up.
const EXIT_TIMEOUT: Duration = Duration::from_secs(20);

/// A running editor, with the terminal it is drawing into.
struct Session {
    child: Child,
    /// The controlling end of the pseudo-terminal: what the editor's display
    /// arrives on, and where its keystrokes go.
    controller: std::fs::File,
    output: Vec<u8>,
}

impl Session {
    /// Starts `maxgus` with `arguments` in `directory`, sharing this process's
    /// group — which is what a program spawned by a test runner ordinarily
    /// does, and which leaves the group orphaned.
    fn start(directory: &std::path::Path, arguments: &[&str]) -> Session {
        Session::spawn(directory, arguments, false)
    }

    /// Starts `maxgus` in a process group of its own, in this session.
    ///
    /// That is the arrangement a shell sets up for a foreground job, and the
    /// only one in which a stop signal is not discarded — so it is the only
    /// one in which `C-z` can be seen to work. This test then has to play the
    /// shell's part and continue the job itself.
    fn start_in_its_own_job(directory: &std::path::Path, arguments: &[&str]) -> Session {
        Session::spawn(directory, arguments, true)
    }

    /// Starts the editor with extra environment, which is how a test gets
    /// its own state directory instead of the real one.
    fn start_with_env(
        directory: &std::path::Path,
        arguments: &[&str],
        environment: &[(&str, &str)],
    ) -> Session {
        Session::spawn_with(directory, arguments, false, environment)
    }

    fn spawn(directory: &std::path::Path, arguments: &[&str], own_group: bool) -> Session {
        Session::spawn_with(directory, arguments, own_group, &[])
    }

    fn spawn_with(
        directory: &std::path::Path,
        arguments: &[&str],
        own_group: bool,
        environment: &[(&str, &str)],
    ) -> Session {
        let pty =
            rustix::pty::openpt(rustix::pty::OpenptFlags::RDWR | rustix::pty::OpenptFlags::NOCTTY)
                .expect("a pseudo-terminal");
        rustix::pty::grantpt(&pty).expect("grant");
        rustix::pty::unlockpt(&pty).expect("unlock");
        let name = rustix::pty::ptsname(&pty, Vec::new()).expect("the terminal's name");
        let name = PathBuf::from(String::from_utf8_lossy(name.as_bytes()).into_owned());

        // The editor refuses to start in a terminal of unknown size.
        rustix::termios::tcsetwinsize(
            &pty,
            rustix::termios::Winsize {
                ws_row: ROWS,
                ws_col: COLUMNS,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )
        .expect("a window size");

        let open = |path: &PathBuf| {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .expect("the terminal")
        };
        let mut command = Command::new(env!("CARGO_BIN_EXE_maxgus"));
        command
            .args(arguments)
            .current_dir(directory)
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .envs(environment.iter().copied())
            .stdin(Stdio::from(open(&name)))
            .stdout(Stdio::from(open(&name)))
            .stderr(Stdio::from(open(&name)));
        if own_group {
            // `setpgid(0, 0)` in the child. The pseudo-terminal was opened
            // `NOCTTY` and the child never claims it, so being outside the
            // terminal's foreground group costs it nothing: reads and writes
            // raise no `SIGTTIN` or `SIGTTOU` without a controlling terminal.
            command.process_group(0);
        }
        let child = command.spawn().expect("the editor starts");

        // Reads must not block, or settling would wait for output that is not
        // coming.
        rustix::io::ioctl_fionbio(&pty, true).expect("non-blocking reads");
        let mut session = Session {
            child,
            controller: std::fs::File::from(pty),
            output: Vec::new(),
        };
        session.settle();
        session
    }

    /// Reads whatever the editor has drawn, for a while.
    fn settle(&mut self) {
        let deadline = Instant::now() + SETTLE;
        while Instant::now() < deadline {
            let mut chunk = [0u8; 8192];
            match self.controller.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => self.output.extend_from_slice(&chunk[..n]),
                // Nothing to read yet; wait rather than spin.
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    }

    /// Sends keystrokes and waits for the redraw.
    fn send(&mut self, keys: &[u8]) -> &mut Session {
        self.controller
            .write_all(keys)
            .expect("the editor is listening");
        self.controller.flush().ok();
        self.settle();
        self
    }

    /// Sends `C-x C-c` and waits for the editor to leave.
    fn quit(&mut self) -> i32 {
        self.send(b"\x18\x03");
        let deadline = Instant::now() + EXIT_TIMEOUT;
        while Instant::now() < deadline {
            if let Ok(Some(status)) = self.child.try_wait() {
                return status.code().unwrap_or(-1);
            }
            self.settle();
        }
        self.child.kill().ok();
        panic!("the editor did not leave when asked");
    }

    /// The single letter Linux reports for the editor's state: `R` running,
    /// `S` sleeping, `T` stopped.
    fn state(&self) -> char {
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", self.child.id()))
            .expect("the editor is still a process");
        // The command name sits in parentheses and may contain spaces itself,
        // so the fields after it are counted from the last `)`.
        let after_name = &stat[stat.rfind(')').expect("a well-formed stat line") + 1..];
        after_name
            .split_whitespace()
            .next()
            .and_then(|field| field.chars().next())
            .expect("a state letter")
    }

    /// Waits for the editor to stop itself, and drains what it wrote on the
    /// way down. Nothing more can arrive from a stopped process, so the buffer
    /// is complete once this returns.
    fn wait_until_stopped(&mut self) {
        let deadline = Instant::now() + EXIT_TIMEOUT;
        while Instant::now() < deadline {
            if self.state() == 'T' {
                self.settle();
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the editor never stopped; it is in state {}", self.state());
    }

    /// Continues the editor, as `fg` does.
    fn continue_job(&mut self) {
        let pid = rustix::process::Pid::from_raw(self.child.id() as i32).expect("a pid");
        rustix::process::kill_process(pid, rustix::process::Signal::CONT).expect("continue");
    }

    /// True when the editor has written `needle` to the terminal at any point.
    fn wrote(&self, needle: &str) -> bool {
        self.output
            .windows(needle.len())
            .any(|window| window == needle.as_bytes())
    }

    /// Where the cursor sits after everything the editor has drawn.
    fn cursor(&self) -> (usize, usize) {
        let mut screen = Screen::new();
        screen.feed(&self.output);
        screen.cursor()
    }

    /// The screen as the user would see it.
    fn screen(&self) -> Vec<String> {
        let mut screen = Screen::new();
        screen.feed(&self.output);
        screen.lines()
    }

    /// The screen as painted by the output written since `mark`, with nothing
    /// carried over from before it. A full repaint fills this on its own; an
    /// incremental one leaves it mostly blank.
    fn screen_from(&self, mark: usize) -> Vec<String> {
        let mut screen = Screen::new();
        screen.feed(&self.output[mark..]);
        screen.lines()
    }

    /// True when any line on screen contains `needle`.
    fn shows(&self, needle: &str) -> bool {
        self.screen().iter().any(|line| line.contains(needle))
    }

    /// True when the mode line reports unsaved changes, whichever way it is
    /// drawing that today.
    fn says_modified(&mut self) -> bool {
        self.mode_line().contains(maxgus_core::icons::MODIFIED) || self.mode_line().contains("**")
    }

    fn says_read_only(&mut self) -> bool {
        self.mode_line().contains(maxgus_core::icons::READ_ONLY) || self.mode_line().contains("%%")
    }

    /// The mode line: the row above the echo area.
    fn mode_line(&self) -> String {
        let screen = self.screen();
        screen[screen.len() - 2].clone()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

/// A directory of test files, removed on drop.
struct Fixture(PathBuf);

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let directory = std::env::temp_dir().join(format!("maxgus-smoke-{tag}"));
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(directory.join("src")).expect("a fixture directory");
        std::fs::write(directory.join("hello.txt"), "first line\nsecond line\n").unwrap();
        std::fs::write(
            directory.join("src/main.rs"),
            "fn main() {\n    let x = 1;\n}\n",
        )
        .unwrap();
        Fixture(directory)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

// ---- a very small terminal emulator ------------------------------------

/// Replays the editor's output onto a grid.
///
/// Redisplay writes *differences*, so the escape stream has to be interpreted
/// rather than stripped: reading only the printable bytes would show a mixture
/// of every frame drawn, not the one on screen.
struct Screen {
    grid: Vec<Vec<char>>,
    x: usize,
    y: usize,
}

impl Screen {
    fn new() -> Screen {
        Screen {
            grid: vec![vec![' '; COLUMNS as usize]; ROWS as usize],
            x: 0,
            y: 0,
        }
    }

    fn feed(&mut self, data: &[u8]) {
        let text = String::from_utf8_lossy(data).into_owned();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\x1b' => self.escape(&mut chars),
                '\r' => self.x = 0,
                '\n' => {
                    self.y += 1;
                    self.x = 0;
                }
                '\u{8}' => self.x = self.x.saturating_sub(1),
                c if c >= ' ' => {
                    if self.y < self.grid.len() && self.x < self.grid[self.y].len() {
                        self.grid[self.y][self.x] = c;
                    }
                    self.x += 1;
                }
                _ => {}
            }
        }
    }

    /// Consumes one escape sequence, acting on the few that move or erase.
    fn escape(&mut self, chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
        let Some(introducer) = chars.next() else {
            return;
        };
        if introducer != '[' {
            // A two-character escape: charset selection and the like.
            return;
        }
        let mut parameters = String::new();
        let mut final_byte = None;
        for c in chars.by_ref() {
            if c.is_ascii_alphabetic() {
                final_byte = Some(c);
                break;
            }
            parameters.push(c);
        }
        let Some(final_byte) = final_byte else { return };
        let values: Vec<usize> = parameters
            .split(';')
            .filter_map(|p| p.parse().ok())
            .collect();
        match final_byte {
            // Cursor position.
            'H' => {
                self.y = values.first().copied().unwrap_or(1).saturating_sub(1);
                self.x = values.get(1).copied().unwrap_or(1).saturating_sub(1);
            }
            // Erase in display.
            'J' if values.first().copied().unwrap_or(0) == 2 => {
                self.grid = vec![vec![' '; COLUMNS as usize]; ROWS as usize];
            }
            // Erase in line.
            'K' if self.y < self.grid.len() => {
                for x in self.x..self.grid[self.y].len() {
                    self.grid[self.y][x] = ' ';
                }
            }
            _ => {}
        }
    }

    /// Where the terminal's own cursor was left — the last `MoveTo` the editor
    /// wrote, which is what a person actually sees blinking.
    fn cursor(&self) -> (usize, usize) {
        (self.x, self.y)
    }

    fn lines(&self) -> Vec<String> {
        self.grid
            .iter()
            .map(|row| row.iter().collect::<String>().trim_end().to_string())
            .collect()
    }
}

// ---- the tests ---------------------------------------------------------

#[test]
fn the_editor_starts_draws_a_file_and_leaves_cleanly() {
    let fixture = Fixture::new("open");
    let mut session = Session::start(fixture.path(), &["-Q", "hello.txt"]);

    assert!(
        session.shows("first line"),
        "the file was not drawn:\n{:#?}",
        session.screen()
    );
    assert!(session.shows("second line"));
    assert!(
        session.mode_line().contains("hello.txt"),
        "got `{}`",
        session.mode_line()
    );

    assert_eq!(session.quit(), 0, "the editor left with an error");
}

#[test]
fn a_file_named_on_the_command_line_is_open_before_any_key_is_pressed() {
    // The file is read at startup, not on the first keystroke: this is what
    // makes `maxgus file` followed straight by `C-x C-c` still have opened it.
    let fixture = Fixture::new("startup");
    let session = Session::start(fixture.path(), &["-Q", "src/main.rs"]);
    assert!(session.shows("fn main()"), "got:\n{:#?}", session.screen());
    assert!(session.mode_line().contains("main.rs"));
    assert!(
        session.mode_line().contains("rust"),
        "the language was recognised"
    );
}

#[test]
fn typing_and_saving_writes_the_file() {
    let fixture = Fixture::new("save");
    let mut session = Session::start(fixture.path(), &["-Q", "typed.txt"]);

    session.send(b"hello from a test");
    assert!(
        session.shows("hello from a test"),
        "got:\n{:#?}",
        session.screen()
    );
    assert!(
        session.says_modified(),
        "the buffer reads as modified: `{}`",
        session.mode_line()
    );

    // `C-x C-s`.
    session.send(b"\x18\x13");
    assert_eq!(session.quit(), 0);

    let written = std::fs::read_to_string(fixture.path().join("typed.txt")).expect("the file");
    // The final newline is added on the way out, as the setting asks.
    assert_eq!(written, "hello from a test\n");
}

#[test]
fn find_file_opens_a_file_typed_at_the_prompt() {
    let fixture = Fixture::new("findfile");
    let mut session = Session::start(fixture.path(), &["-Q"]);

    // `C-x C-f`, clear the offered directory with `C-k`, type a name, `RET`.
    session.send(b"\x18\x06");
    assert!(
        session.shows("Find file:"),
        "no prompt:\n{:#?}",
        session.screen()
    );
    session.send(b"\x0b");
    session.send(b"hello.txt\r");

    assert!(
        session.shows("first line"),
        "the file did not open:\n{:#?}",
        session.screen()
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn the_tutorial_opens_from_its_key() {
    let fixture = Fixture::new("tutorial");
    let mut session = Session::start(fixture.path(), &["-Q"]);

    // `C-h t`.
    session.send(b"\x08t");
    assert!(
        session.shows("a short guide"),
        "got:\n{:#?}",
        session.screen()
    );
    assert!(session.shows("C-x C-c"), "the guide says how to leave");
    assert!(session.mode_line().contains("*Help*"));
}

#[test]
fn the_file_tree_opens_beside_the_buffer() {
    let fixture = Fixture::new("tree");
    let mut session = Session::start(fixture.path(), &["-Q", "hello.txt"]);

    // `C-x t t`.
    session.send(b"\x18tt");
    let screen = session.screen();
    assert!(
        screen[0].contains("src") || screen.iter().any(|l| l.contains("src")),
        "the tree was not drawn:\n{screen:#?}"
    );
    // The buffer is still visible beside it.
    assert!(session.shows("first line"), "got:\n{screen:#?}");
}

#[test]
fn an_unsaved_buffer_refuses_to_quit_and_then_lets_go() {
    let fixture = Fixture::new("unsaved");
    let mut session = Session::start(fixture.path(), &["-Q", "hello.txt"]);
    session.send(b"edited");

    // `C-x C-c` is refused while there is unsaved work.
    session.send(b"\x18\x03");
    assert!(
        session.shows("hello.txt"),
        "the editor should still be running:\n{:#?}",
        session.screen()
    );
    assert!(session.child.try_wait().expect("still running").is_none());

    // `C-u C-x C-c` leaves anyway.
    session.send(b"\x15\x18\x03");
    let deadline = Instant::now() + EXIT_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(Some(status)) = session.child.try_wait() {
            assert_eq!(status.code(), Some(0));
            return;
        }
        session.settle();
    }
    panic!("the editor did not leave when told to");
}

#[test]
fn the_help_flag_prints_usage_without_a_terminal() {
    // No pseudo-terminal here: `--help` must work when piped, so that
    // `maxgus --help | less` behaves.
    let output = Command::new(env!("CARGO_BIN_EXE_maxgus"))
        .arg("--help")
        .output()
        .expect("the binary runs");
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("Usage: maxgus"), "got `{text}`");
    assert!(text.contains("--no-config"));
}

#[test]
fn starting_without_a_terminal_fails_with_an_explanation() {
    let output = Command::new(env!("CARGO_BIN_EXE_maxgus"))
        .arg("-Q")
        .stdin(Stdio::null())
        .output()
        .expect("the binary runs");
    assert!(
        !output.status.success(),
        "it should refuse rather than misbehave"
    );
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("terminal"), "got `{text}`");
}

/// True when `program` can be found on the path.
fn available(program: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {program}"))
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// A C project with a deliberate error, plus the configuration to analyse it.
fn c_project(tag: &str) -> Fixture {
    let fixture = Fixture::new(tag);
    std::fs::write(
        fixture.path().join("main.c"),
        "#include <stdio.h>\n\nint main(void) {\n    int unused = 5;\n    undefined_call();\n    return 0;\n}\n",
    )
    .unwrap();
    std::fs::write(
        fixture.path().join("compile_commands.json"),
        format!(
            r#"[{{"directory":"{}","command":"cc -c main.c","file":"main.c"}}]"#,
            fixture.path().display()
        ),
    )
    .unwrap();
    std::fs::write(
        fixture.path().join("config.kdl"),
        "set idle-delay-ms=100\nlsp \"c\" command=\"clangd\" {\n    root-markers \"compile_commands.json\"\n}\n",
    )
    .unwrap();
    fixture
}

/// A language server takes noticeably longer to answer than a redraw does.
fn wait_for(session: &mut Session, needle: &str, tries: usize) -> bool {
    for _ in 0..tries {
        if session.shows(needle) {
            return true;
        }
        session.settle();
    }
    false
}

#[cfg(feature = "full")]
#[test]
fn a_language_server_reports_diagnostics_and_follows_edits() {
    // The only test that exercises the whole language-server path against a
    // real server: spawning it, the handshake, opening the document, and
    // keeping it in step as the buffer changes.
    if !available("clangd") {
        eprintln!("skipping: clangd is not installed");
        return;
    }
    let fixture = c_project("lsp");
    let mut session = Session::start(fixture.path(), &["--config", "config.kdl", "main.c"]);

    // The mode line shows the counts behind their glyphs.
    let errors = maxgus_core::icons::ERROR.to_string();
    assert!(
        wait_for(&mut session, &format!("{errors} 1"), 40),
        "clangd never reported the error:\n{:#?}",
        session.screen()
    );
    assert!(
        session.mode_line().contains(maxgus_core::icons::WARNING),
        "clangd never reported the warning: `{}`",
        session.mode_line()
    );

    // Delete the line that does not compile: four lines down, kill the line
    // and its newline.
    session.send(b"\x0e\x0e\x0e\x0e");
    session.send(b"\x01\x0b\x0b");
    assert!(!session.shows("undefined_call"), "the line was not removed");

    // The server has to be told what changed, or it would still be looking at
    // the text as it was when the file was opened.
    // Waited for on the mode line rather than the screen: the error glyph may
    // still be sitting in the buffer's own text or an earlier frame, and it is
    // the *mode line* that reports what the server currently thinks.
    let cleared = (0..40).any(|_| {
        session.settle();
        !session.mode_line().contains(maxgus_core::icons::ERROR)
    });
    assert!(
        cleared,
        "the error never cleared, so the server did not see the edit: `{}`",
        session.mode_line()
    );
    assert!(
        session.mode_line().contains(maxgus_core::icons::WARNING)
            && !session.mode_line().contains(maxgus_core::icons::ERROR),
        "expected only the warning to remain, got `{}`",
        session.mode_line()
    );
}

#[test]
fn hover_answers_from_a_real_language_server() {
    if !available("clangd") {
        eprintln!("skipping: clangd is not installed");
        return;
    }
    let fixture = c_project("hover");
    let mut session = Session::start(fixture.path(), &["--config", "config.kdl", "main.c"]);
    assert!(
        wait_for(&mut session, "main.c", 20),
        "the file never opened"
    );

    // Onto `unused` on line 4, then `C-c l d`.
    session.send(b"\x0e\x0e\x0e");
    session.send(&[0x06; 8]);
    session.send(b"\x03ld");

    assert!(
        wait_for(&mut session, "unused", 40),
        "no description arrived:\n{:#?}",
        session.screen()
    );
    assert!(
        session.shows("int"),
        "the type was not described:\n{:#?}",
        session.screen()
    );
}

/// Writes a configuration file into `fixture` and returns its name.
fn with_config(fixture: &Fixture, contents: &str) -> &'static str {
    std::fs::write(fixture.path().join("config.kdl"), contents).unwrap();
    "config.kdl"
}

#[test]
fn a_half_typed_key_sequence_waits_before_being_shown() {
    // Emacs holds the echo back so a fluent `C-x C-s` never flashes anything;
    // showing it at once would make every prefix key blink the echo area.
    let fixture = Fixture::new("echoslow");
    let config = with_config(&fixture, "set echo-keystrokes-ms=3000\n");
    let mut session = Session::start(fixture.path(), &["--config", config, "hello.txt"]);

    session.send(b"\x18");
    assert!(
        !session.mode_line().is_empty(),
        "the editor is drawing:\n{:#?}",
        session.screen()
    );
    let echo = session.screen().last().cloned().unwrap_or_default();
    assert!(
        !echo.contains("C-x"),
        "the sequence was shown at once: `{echo}`"
    );
}

#[test]
fn a_short_echo_delay_shows_the_sequence_promptly() {
    let fixture = Fixture::new("echofast");
    let config = with_config(&fixture, "set echo-keystrokes-ms=1\n");
    let mut session = Session::start(fixture.path(), &["--config", config, "hello.txt"]);

    session.send(b"\x18");
    assert!(
        wait_for(&mut session, "C-x", 10),
        "the sequence was never shown:\n{:#?}",
        session.screen()
    );
}

#[test]
fn the_cursor_blinks_when_the_configuration_asks_it_to() {
    let fixture = Fixture::new("blink");
    let config = with_config(&fixture, "set blink-cursor=#true\n");
    let mut session = Session::start(fixture.path(), &["--config", config, "hello.txt"]);
    session.settle();
    // `CSI 1 SP q` selects a blinking block; `CSI 2 SP q` a steady one.
    let written = String::from_utf8_lossy(&session.output).into_owned();
    assert!(
        written.contains("\x1b[1 q"),
        "no blinking cursor was requested"
    );
    assert!(
        !written.contains("\x1b[2 q"),
        "a steady cursor was requested too"
    );
}

#[test]
fn the_cursor_is_steady_by_default() {
    let fixture = Fixture::new("noblink");
    let config = with_config(&fixture, "set blink-cursor=#false\n");
    let mut session = Session::start(fixture.path(), &["--config", config, "hello.txt"]);
    session.settle();
    let written = String::from_utf8_lossy(&session.output).into_owned();
    assert!(
        written.contains("\x1b[2 q"),
        "no steady cursor was requested"
    );
}

#[test]
fn a_mode_keymap_from_the_configuration_applies_only_to_that_mode() {
    // A `keymap "rust-mode"` block was parsed and stored but never installed,
    // so bindings written for a mode did nothing at all.
    let fixture = Fixture::new("modemap");
    std::fs::write(fixture.path().join("plain.txt"), "abcd\n").unwrap();
    let config = with_config(
        &fixture,
        "keymap \"rust-mode\" {\n    bind \"C-t\" \"list-buffers\"\n}\n",
    );

    // In a Rust buffer, `C-t` runs the mode's binding.
    let mut session = Session::start(fixture.path(), &["--config", config, "src/main.rs"]);
    session.send(b"\x14");
    assert!(
        session.mode_line().contains("*Buffer List*"),
        "the mode binding did not run, got `{}`",
        session.mode_line()
    );
    drop(session);

    // In a plain-text buffer, the global binding applies instead.
    let mut session = Session::start(fixture.path(), &["--config", config, "plain.txt"]);
    session.send(b"\x06\x06\x14");
    assert!(
        session.shows("acbd"),
        "the global `transpose-chars` did not run:\n{:#?}",
        session.screen()
    );
}

#[test]
fn ctrl_z_stops_the_editor_and_it_draws_again_when_continued() {
    // The whole of `C-z`, against a real kernel: the terminal is handed back,
    // the process really stops, and continuing it puts the screen back.
    let fixture = Fixture::new("suspend-job");
    let mut session = Session::start_in_its_own_job(fixture.path(), &["-Q", "hello.txt"]);
    assert!(
        session.shows("first line"),
        "the file was not drawn:\n{:#?}",
        session.screen()
    );

    session.send(b"\x1a");
    session.wait_until_stopped();

    // Leaving the alternate screen is what gives the shell its own screen
    // back. Stopping while still holding it would strand the user in a raw
    // terminal showing the editor's last frame.
    assert!(
        session.wrote("\x1b[?1049l"),
        "the editor stopped without handing the terminal back"
    );

    let mark = session.output.len();
    session.continue_job();
    session.settle();

    // A stopped process is told nothing about what happened to its terminal,
    // so everything on screen has to be written again rather than diffed
    // against a frame that may no longer be there. Judged from the output
    // since the job was continued, with nothing carried over.
    let redrawn = session.screen_from(mark);
    assert!(
        redrawn.iter().any(|line| line.contains("first line")),
        "the editor did not repaint after being continued:\n{redrawn:#?}"
    );
    assert!(
        redrawn[redrawn.len() - 2].contains("hello.txt"),
        "the mode line came back: `{}`",
        redrawn[redrawn.len() - 2]
    );

    assert_eq!(
        session.quit(),
        0,
        "the editor still works after being continued"
    );
}

#[test]
fn ctrl_z_says_so_when_nothing_could_bring_the_editor_back() {
    // Spawned without a group of its own, the editor shares this test's
    // process group, which makes the group orphaned: Linux discards a stop
    // signal sent to it, and there is no shell waiting to run `fg` anyway.
    // The editor has to say so rather than appear to ignore the key.
    let fixture = Fixture::new("suspend-orphan");
    let mut session = Session::start(fixture.path(), &["-Q", "hello.txt"]);

    session.send(b"\x1a");
    assert_ne!(
        session.state(),
        'T',
        "the editor stopped with nothing able to continue it"
    );
    assert!(
        session.shows("No job control"),
        "got:\n{:#?}",
        session.screen()
    );
    assert!(session.shows("first line"), "the buffer is still on screen");

    assert_eq!(session.quit(), 0);
}

#[test]
fn a_theme_defined_only_in_the_configuration_can_be_loaded_at_runtime() {
    // `load-theme` refuses a name it does not know, so a theme that exists
    // nowhere but the configuration file can only be loaded if startup handed
    // those blocks to the editor. That is the whole point of keeping them.
    let fixture = Fixture::new("theme-config");
    std::fs::write(
        fixture.path().join("config.kdl"),
        "theme \"midnight\" {\n    face \"region\" background=\"#001133\"\n}\n",
    )
    .unwrap();
    let mut session = Session::start(fixture.path(), &["--config", "config.kdl", "hello.txt"]);

    // `M-x load-theme RET midnight RET`.
    session.send(b"\x1bx");
    session.send(b"load-theme\r");
    session.send(b"midnight\r");

    assert!(
        session.shows("Theme midnight"),
        "got:\n{:#?}",
        session.screen()
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn a_misspelled_face_in_the_configuration_is_reported_with_a_suggestion() {
    // Which faces exist is knowledge the config parser does not have, so this
    // check lives in `main.rs` and nothing but a real start exercises it.
    let fixture = Fixture::new("bad-face");
    std::fs::write(
        fixture.path().join("config.kdl"),
        "theme \"maxgus-dark\" {\n    face \"font-lock-coment\" fg=\"#ffffff\"\n}\n",
    )
    .unwrap();
    let mut session = Session::start(fixture.path(), &["--config", "config.kdl", "hello.txt"]);

    assert!(
        session.shows("unknown face `font-lock-coment`"),
        "got:\n{:#?}",
        session.screen()
    );
    // The "did you mean" that follows is cut off by the width of the echo
    // area, so its wording is checked where it is produced, in
    // `maxgus_faces::names`, rather than off the screen here.
    assert_eq!(
        session.quit(),
        0,
        "a bad face never stops the editor starting"
    );
}

#[test]
fn a_face_that_exists_draws_no_complaint() {
    let fixture = Fixture::new("good-face");
    std::fs::write(
        fixture.path().join("config.kdl"),
        "theme \"maxgus-dark\" {\n    face \"font-lock-comment\" fg=\"#ffffff\"\n}\n",
    )
    .unwrap();
    let mut session = Session::start(fixture.path(), &["--config", "config.kdl", "hello.txt"]);

    assert!(
        !session.shows("unknown face"),
        "got:\n{:#?}",
        session.screen()
    );
    assert!(
        !session.shows("configuration problem"),
        "got:\n{:#?}",
        session.screen()
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn a_configuration_problem_survives_the_files_opening() {
    // Opening a file is how an editor is normally started, and the "(N lines)"
    // notice that follows used to talk straight over the complaint — so a
    // mistake in the config file was invisible to everyone who passed a
    // filename, which is everyone.
    let fixture = Fixture::new("warning-order");
    std::fs::write(fixture.path().join("config.kdl"), "set tab-widht=4\n").unwrap();
    let mut session = Session::start(fixture.path(), &["--config", "config.kdl", "hello.txt"]);

    assert!(
        session.shows("unknown setting `tab-widht`"),
        "the complaint was drowned out:\n{:#?}",
        session.screen()
    );
    assert!(session.shows("first line"), "the file still opened");
    assert_eq!(session.quit(), 0);
}

#[test]
fn an_ordinary_notice_still_shows_when_nothing_is_wrong() {
    // The other half: giving way to an error must not stop the notice being
    // shown when there is no error to give way to.
    //
    // The file is opened from inside the editor rather than named on the
    // command line, so this is about the notice and not a race with the
    // startup greeting, which owns the echo area for the first moment.
    let fixture = Fixture::new("notice");
    let mut session = Session::start(fixture.path(), &["-Q"]);
    assert!(
        wait_for(&mut session, "maxgus started in", 60),
        "no greeting"
    );
    session.send(b"\x18\x06hello.txt\r"); // C-x C-f hello.txt RET
    assert!(
        wait_for(&mut session, "hello.txt (3 lines)", 60),
        "got:\n{:#?}",
        session.screen()
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn a_file_that_is_not_text_cannot_be_saved_over_its_own_bytes() {
    // Opening a Latin-1 file and saving it used to replace every byte the
    // decoder could not read with U+FFFD — `0xe9` came back as `ef bf bd`,
    // and the user's file was quietly destroyed by looking at it.
    let fixture = Fixture::new("latin1");
    let original: Vec<u8> = b"caf\xe9 latin1\n".to_vec();
    let path = fixture.path().join("latin1.txt");
    std::fs::write(&path, &original).unwrap();

    let mut session = Session::start(fixture.path(), &["-Q", "latin1.txt"]);
    assert!(
        session.shows("not text"),
        "the reason was not given:\n{:#?}",
        session.screen()
    );
    assert!(
        session.says_read_only(),
        "read-only: `{}`",
        session.mode_line()
    );

    // Typing and saving must both be refused rather than silently letting the
    // replacement characters reach the disk.
    session.send(b"X");
    session.send(b"\x18\x13"); // C-x C-s
    session.settle();

    assert_eq!(
        std::fs::read(&path).unwrap(),
        original,
        "the file on disk was changed"
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn an_ordinary_text_file_is_still_writable() {
    // The guard must not catch files that are perfectly good text.
    let fixture = Fixture::new("utf8");
    let path = fixture.path().join("utf8.txt");
    std::fs::write(&path, "café utf8\n").unwrap();

    let mut session = Session::start(fixture.path(), &["-Q", "utf8.txt"]);
    assert!(!session.shows("not text"), "got:\n{:#?}", session.screen());
    session.send(b"X");
    session.send(b"\x18\x13");
    session.settle();

    let after = String::from_utf8(std::fs::read(&path).unwrap()).expect("still utf-8");
    assert_eq!(after, "Xcafé utf8\n", "the edit was saved");
    assert_eq!(session.quit(), 0);
}

#[test]
fn a_file_changed_underneath_the_buffer_is_not_written_over() {
    // A pull, a formatter, another editor. Saving used to overwrite whatever
    // had arrived and report "Wrote ..." as though nothing had happened.
    let fixture = Fixture::new("external");
    let path = fixture.path().join("shared.txt");
    std::fs::write(&path, "mine\n").unwrap();

    let mut session = Session::start(fixture.path(), &["-Q", "shared.txt"]);
    std::fs::write(&path, "theirs, important\n").unwrap();

    session.send(b"X");
    session.send(b"\x18\x13"); // C-x C-s

    assert!(
        session.shows("has changed on disk"),
        "the refusal was not reported:\n{:#?}",
        session.screen()
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "theirs, important\n",
        "the other change was overwritten"
    );

    // The way past it is a named command, not a second press of the key that
    // just refused.
    session.send(b"\x1bx");
    session.send(b"save-buffer-anyway\r");
    assert!(
        session.shows("Wrote"),
        "the forced save did not happen:\n{:#?}",
        session.screen()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "Xmine\n");

    assert_eq!(session.quit(), 0);
}

#[test]
fn saving_twice_over_with_nobody_else_touching_it_is_fine() {
    // The buffer's idea of the file has to follow its own writes, or the
    // second save would think somebody else had been at it.
    let fixture = Fixture::new("resave");
    let path = fixture.path().join("mine.txt");
    std::fs::write(&path, "one\n").unwrap();

    let mut session = Session::start(fixture.path(), &["-Q", "mine.txt"]);
    session.send(b"A");
    session.send(b"\x18\x13");
    assert!(
        session.shows("Wrote"),
        "first save:\n{:#?}",
        session.screen()
    );

    session.send(b"B");
    session.send(b"\x18\x13");
    assert!(
        !session.shows("has changed on disk"),
        "its own write was mistaken for somebody else's:\n{:#?}",
        session.screen()
    );
    // Point moved on after the first character, so the second lands after it.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "ABone\n");
    assert_eq!(session.quit(), 0);
}

#[test]
fn write_file_does_not_destroy_a_file_it_was_merely_named_at() {
    // `C-x C-w` onto an existing path used to overwrite it and say `Wrote …`.
    // Worse than saving over a changed file: this destroys something the user
    // never opened and has no buffer of.
    let fixture = Fixture::new("clobber");
    let victim = fixture.path().join("important.txt");
    std::fs::write(&victim, "months of work\n").unwrap();

    let mut session = Session::start(fixture.path(), &["-Q", "hello.txt"]);
    session.send(b"\x18\x17"); // C-x C-w
    session.send(b"important.txt\r");

    assert!(
        session.shows("already exists"),
        "no warning was given:\n{:#?}",
        session.screen()
    );
    assert_eq!(
        std::fs::read_to_string(&victim).unwrap(),
        "months of work\n",
        "the file was destroyed"
    );

    // Overwriting on purpose still works.
    session.send(b"\x1bx");
    session.send(b"save-buffer-anyway\r");
    assert!(
        session.shows("Wrote"),
        "the deliberate overwrite failed:\n{:#?}",
        session.screen()
    );
    assert_eq!(
        std::fs::read_to_string(&victim).unwrap(),
        "first line\nsecond line\n"
    );

    assert_eq!(session.quit(), 0);
}

#[test]
fn write_file_to_a_new_name_just_writes_it() {
    // The guard must not stand in the way of the ordinary case.
    let fixture = Fixture::new("write-new");
    let mut session = Session::start(fixture.path(), &["-Q", "hello.txt"]);

    session.send(b"\x18\x17");
    session.send(b"copy.txt\r");

    assert!(session.shows("Wrote"), "got:\n{:#?}", session.screen());
    assert_eq!(
        std::fs::read_to_string(fixture.path().join("copy.txt")).unwrap(),
        "first line\nsecond line\n"
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn write_file_back_to_its_own_name_is_an_ordinary_save() {
    // `C-x C-w` naming the file the buffer already visits is a save, not an
    // attempt to create something new, so "already exists" would be wrong.
    let fixture = Fixture::new("write-same");
    let mut session = Session::start(fixture.path(), &["-Q", "hello.txt"]);
    session.send(b"Z");
    session.send(b"\x18\x17");
    session.send(b"hello.txt\r");

    assert!(
        !session.shows("already exists"),
        "got:\n{:#?}",
        session.screen()
    );
    assert!(session.shows("Wrote"), "got:\n{:#?}", session.screen());
    assert_eq!(
        std::fs::read_to_string(fixture.path().join("hello.txt")).unwrap(),
        "Zfirst line\nsecond line\n"
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn a_theme_dropped_into_the_themes_directory_is_found_by_name() {
    // The whole point: a theme is a file you drop in. Nothing in config.kdl
    // mentions its faces, only `set theme=`.
    //
    // Two files, and the one named in `config.kdl` is *not* the one asserted
    // on: the theme in use appears in the `load-theme` prompt as its default,
    // so it would show whether the directory had been read or not. `seaside`
    // can only reach the screen from the candidate list.
    let fixture = Fixture::new("themedir");
    std::fs::create_dir_all(fixture.path().join("themes")).unwrap();
    std::fs::write(
        fixture.path().join("themes/midnight.kdl"),
        "theme \"midnight\" base=\"maxgus-dark\" {\n    face \"region\" bg=\"#001133\"\n}\n",
    )
    .unwrap();
    std::fs::write(
        fixture.path().join("themes/seaside.kdl"),
        "theme \"seaside\" base=\"maxgus-light\" {\n    face \"region\" bg=\"#cceeff\"\n}\n",
    )
    .unwrap();
    std::fs::write(
        fixture.path().join("config.kdl"),
        "set theme=\"midnight\"\n",
    )
    .unwrap();

    let mut session = Session::start(fixture.path(), &["--config", "config.kdl", "hello.txt"]);

    assert!(
        !session.shows("configuration problem"),
        "loading the themes directory complained:\n{:#?}",
        session.screen()
    );

    session.send(b"\x1bx");
    session.send(b"load-theme\r");
    assert!(
        session.shows("seaside"),
        "the themes directory was never read:\n{:#?}",
        session.screen()
    );
    session.send(b"\x07"); // C-g

    // And the one `config.kdl` asked for is the one in use.
    session.send(b"\x08v"); // C-h v
    session.send(b"theme\r");
    assert!(
        session.shows("midnight"),
        "the theme was not taken up:\n{:#?}",
        session.screen()
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn a_theme_file_is_offered_by_load_theme() {
    // And it has to be reachable at runtime, not only at startup.
    let fixture = Fixture::new("themedir-list");
    std::fs::create_dir_all(fixture.path().join("themes")).unwrap();
    std::fs::write(
        fixture.path().join("themes/seaside.kdl"),
        "theme \"seaside\" base=\"maxgus-light\" {\n    face \"region\" bg=\"#cceeff\"\n}\n",
    )
    .unwrap();
    std::fs::write(
        fixture.path().join("config.kdl"),
        "set line-numbers=#true\n",
    )
    .unwrap();

    let mut session = Session::start(fixture.path(), &["--config", "config.kdl", "hello.txt"]);

    // `M-x load-theme` lists what is on offer as it opens.
    session.send(b"\x1bx");
    session.send(b"load-theme\r");
    assert!(
        session.shows("seaside"),
        "the theme file was not offered:\n{:#?}",
        session.screen()
    );
    session.send(b"seaside\r");
    assert!(
        session.shows("Theme seaside"),
        "it would not load:\n{:#?}",
        session.screen()
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn a_broken_theme_file_is_reported_and_does_not_stop_the_editor() {
    let fixture = Fixture::new("themedir-bad");
    std::fs::create_dir_all(fixture.path().join("themes")).unwrap();
    std::fs::write(
        fixture.path().join("themes/broken.kdl"),
        "theme \"broken\" base=\"maxgus-dark\" {\n    face \"font-lock-coment\" fg=\"#fff\"\n}\n",
    )
    .unwrap();
    std::fs::write(fixture.path().join("config.kdl"), "set tab-width=4\n").unwrap();

    let mut session = Session::start(fixture.path(), &["--config", "config.kdl", "hello.txt"]);

    assert!(
        session.shows("font-lock-coment"),
        "the bad face was not reported:\n{:#?}",
        session.screen()
    );
    assert!(session.shows("first line"), "the editor still started");
    assert_eq!(session.quit(), 0);
}

#[test]
fn the_cursor_follows_the_selected_window_with_the_tree_open() {
    // Reported as "C-x o selects the buffer but the cursor is frozen". This
    // asserts on the terminal's own cursor — the last `MoveTo` written — in
    // each window in turn, which is the thing a person actually watches.
    let fixture = Fixture::new("treecursor");
    let long: String = (1..=20).map(|n| format!("line {n}\n")).collect();
    std::fs::write(fixture.path().join("long.txt"), &long).unwrap();

    let mut session = Session::start(fixture.path(), &["-Q", "long.txt"]);
    session.send(b"\x18tt"); // C-x t t
    let in_code = session.cursor();
    assert!(
        in_code.0 > 0,
        "the code window sits to the right of the tree: {in_code:?}"
    );

    // Moving in the code moves the cursor, and keeps moving.
    let mut seen = vec![in_code];
    for _ in 0..3 {
        session.send(b"\x0e"); // C-n
        seen.push(session.cursor());
    }
    assert!(
        seen.windows(2)
            .all(|w| w[1].1 == w[0].1 + 1 && w[1].0 == w[0].0),
        "the cursor did not walk down the code window: {seen:?}"
    );

    // Into the tree, where it moves too — and lands in the tree's column.
    // `C-x t 1` rather than `C-x o`: the panel is a column of windows, and
    // `C-x o` reaches whichever of them comes next.
    session.send(b"\x18t1"); // C-x t 1
    let in_tree = session.cursor();
    assert!(
        in_tree.0 < in_code.0,
        "the cursor is not in the tree: {in_tree:?}"
    );
    session.send(b"\x0e");
    assert_eq!(
        session.cursor(),
        (in_tree.0, in_tree.1 + 1),
        "the tree cursor is stuck"
    );

    // And back, to where the code window was left rather than to its top.
    // `C-<right>` leaves the column; `C-x o` would reach the next window in
    // it, which is the outline or the buffer list.
    session.send(b"\x1b[1;5C");
    assert_eq!(
        session.cursor(),
        *seen.last().expect("moves"),
        "coming back did not restore the code window's cursor"
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn control_arrows_move_the_cursor_between_the_tree_and_the_code() {
    let fixture = Fixture::new("windmove");
    let mut session = Session::start(fixture.path(), &["-Q", "hello.txt"]);
    session.send(b"\x18tt"); // C-x t t
    let in_code = session.cursor();

    session.send(b"\x1b[1;5D"); // C-<left>
    let in_tree = session.cursor();
    assert!(
        in_tree.0 < in_code.0,
        "C-<left> did not reach the tree: {in_tree:?}"
    );

    session.send(b"\x1b[1;5C"); // C-<right>
    assert_eq!(
        session.cursor(),
        in_code,
        "C-<right> did not come back to the code"
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn arrows_navigate_a_file_opened_from_the_tree() {
    // Reported exactly: "when I open a file from treefile, the navigation with
    // Left/Right/Up/Down does not work, however PgUp/PgDown does work". The
    // tree's map binds the arrows and not the paging keys, and it was applied
    // in every buffer rather than only in the tree.
    let fixture = Fixture::new("treearrows");
    let long: String = (1..=40).map(|n| format!("line {n} here\n")).collect();
    std::fs::write(fixture.path().join("long.txt"), &long).unwrap();

    let mut session = Session::start(fixture.path(), &["-Q", "hello.txt"]);
    session.send(b"\x18tt"); // C-x t t
    session.send(b"\x18t1"); // C-x t 1 : into the tree
    // Walk down to `long.txt` and open it. The number of steps is read off
    // the screen rather than counted by hand: the panel draws from its first
    // row, so a screen row is a panel row, and a heading appearing above the
    // tree should not silently change which file this opens. Only one RET:
    // after it the file window has the keyboard, so a further `n` would be
    // typed into the file.
    session.send(b"\x1b<"); // M-< : the top of the panel
    let steps = session
        .screen()
        .iter()
        .position(|line| line.contains("long.txt"))
        .expect("long.txt in the panel");
    for _ in 0..steps {
        session.send(b"n");
    }
    session.send(b"\r");

    assert!(
        session.mode_line().contains("long.txt"),
        "got `{}`",
        session.mode_line()
    );
    let opened = session.cursor();

    // The keys that were reported broken.
    session.send(b"\x1b[B"); // <down>
    let down = session.cursor();
    assert_eq!(
        down,
        (opened.0, opened.1 + 1),
        "<down> did not move: {down:?}"
    );

    session.send(b"\x1b[C"); // <right>
    assert_eq!(
        session.cursor(),
        (down.0 + 1, down.1),
        "<right> did not move"
    );

    session.send(b"\x1b[D"); // <left>
    assert_eq!(session.cursor(), down, "<left> did not move");

    session.send(b"\x1b[A"); // <up>
    assert_eq!(session.cursor(), opened, "<up> did not move");

    // And the keys that were reported working still do. Read from the code
    // window's own part of the row: the panel's bottom window has a mode
    // line on the same row, and it legitimately says line one.
    session.send(b"\x1b[6~"); // <next> / PgDn
    let bar = session.mode_line();
    let at = bar.find("long.txt").expect("the file's own mode line");
    assert!(
        !bar[at..].contains(" 1:0 "),
        "PgDn stopped working: `{bar}`"
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn the_trees_own_arrow_keys_still_work_inside_the_tree() {
    // The other half of the fix: moving the tree's map out of every buffer
    // must not take it away from the tree.
    let fixture = Fixture::new("treearrows-own");
    let mut session = Session::start(fixture.path(), &["-Q", "hello.txt"]);
    session.send(b"\x18tt");
    session.send(b"\x18o"); // into the tree
    let start = session.cursor();

    // Upwards, because follow mode has already put the cursor on the file
    // being edited — which is the last node here, so there is nothing below.
    session.send(b"\x1b[A"); // <up>
    let up = session.cursor();
    assert_eq!(up, (start.0, start.1 - 1), "the tree lost its arrows");

    session.send(b"\x1b[B"); // <down>
    assert_eq!(session.cursor(), start, "<down> did not come back");

    // `<right>` expands a directory rather than moving a character, which is
    // the tree's binding and not the global one.
    session.send(b"\x1b[A"); // onto `src`
    session.send(b"\x1b[C"); // <right>
    assert!(
        session.shows("main.rs"),
        "<right> did not expand:\n{:#?}",
        session.screen()
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn a_treefile_mode_binding_adds_to_the_built_in_ones() {
    // The tree's keymap is a mode map now, so the configuration can extend it
    // — and extending it must not cost the fifty-eight bindings it ships with.
    let fixture = Fixture::new("treemode-config");
    std::fs::write(
        fixture.path().join("config.kdl"),
        "keymap \"treefile-mode\" {\n    bind \"C-c C-r\" \"treefile-refresh\"\n}\n",
    )
    .unwrap();

    let mut session = Session::start(fixture.path(), &["--config", "config.kdl", "hello.txt"]);
    assert!(
        !session.shows("configuration problem"),
        "got:\n{:#?}",
        session.screen()
    );

    session.send(b"\x18tt");
    session.send(b"\x18o"); // into the tree
    let start = session.cursor();

    // A built-in tree binding still works alongside the configured one.
    session.send(b"\x1b[A"); // <up>
    assert_eq!(
        session.cursor(),
        (start.0, start.1 - 1),
        "adding a binding cost the built-in ones"
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn o_o_from_the_tree_opens_into_the_window_you_choose() {
    // Reported: with split buffers, `o o` should choose where the file opens,
    // "but the treefile does not capture the keybinding, it interprets the
    // inputs as attempts to edit in the treefile buffer".
    let fixture = Fixture::new("treeoo");
    let mut session = Session::start(fixture.path(), &["-Q", "hello.txt"]);
    session.send(b"\x182"); // C-x 2 : split, so there is a choice to make
    session.send(b"\x18tt"); // C-x t t
    // `C-x t 1`, not `C-x o` or an arrow: the panel is a column of windows,
    // and both of those reach whichever one happens to be adjacent.
    session.send(b"\x18t1");
    // In the column, which is what matters: the bottom mode line belongs to
    // whichever panel window is lowest, not to the selected one.
    assert!(
        session.cursor().0 < 32,
        "not in the panel: {:?}",
        session.cursor()
    );

    session.send(b"nn"); // down to hello.txt
    session.send(b"oo"); // o o : visit the node

    assert!(
        !session.shows("read-only"),
        "the keys were typed into the tree instead of being a binding:\n{:#?}",
        session.screen()
    );
    // The mode line, not the echo area: the startup greeting owns that at
    // startup, and the tree shows the file's name whether it is open or not.
    assert!(
        // The mode line writes it with its project in front; the tree shows
        // a bare name, so the slash is what tells them apart.
        wait_for(&mut session, "/hello.txt", 60),
        "`o o` did not open the file:\n{:#?}",
        session.screen()
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn the_trees_other_two_key_bindings_work_too() {
    // `o o` was one of five prefixes; all of them were lost the same way.
    let fixture = Fixture::new("treeprefix");
    let mut session = Session::start(fixture.path(), &["-Q", "hello.txt"]);
    session.send(b"\x18tt");
    session.send(b"\x18o");

    session.send(b"th"); // t h : show dotfiles
    assert!(
        !session.shows("read-only"),
        "`t h` typed itself:\n{:#?}",
        session.screen()
    );

    session.send(b"gr"); // g r : refresh
    assert!(
        !session.shows("read-only"),
        "`g r` typed itself:\n{:#?}",
        session.screen()
    );

    session.send(b"ya"); // y a : copy the absolute path
    assert!(
        !session.shows("read-only"),
        "`y a` typed itself:\n{:#?}",
        session.screen()
    );
    assert!(
        session.shows("Copied"),
        "`y a` did not run:\n{:#?}",
        session.screen()
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn the_cursor_sits_on_the_text_when_line_numbers_are_on() {
    // With the gutter three columns wide, the cursor was drawn at column 0
    // while the text it pointed at began at column 3.
    let fixture = Fixture::new("linenumbers");
    std::fs::write(
        fixture.path().join("config.kdl"),
        "set line-numbers=#true\n",
    )
    .unwrap();
    let mut session = Session::start(fixture.path(), &["--config", "config.kdl", "hello.txt"]);

    let start = session.cursor();
    assert!(
        start.0 >= 2,
        "the cursor is sitting in the line-number column: {start:?}"
    );

    // The character under the cursor is the first of the line, not a digit.
    let row = session.screen()[start.1].clone();
    let under = row.chars().nth(start.0).unwrap_or(' ');
    assert_eq!(
        under, 'f',
        "the cursor is not on the text: row `{row}` at {start:?}"
    );

    session.send(b"\x06"); // C-f
    assert_eq!(
        session.cursor(),
        (start.0 + 1, start.1),
        "it lost the offset"
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn visit_theme_previews_keeps_and_writes_the_choice() {
    let fixture = Fixture::new("visit-theme");
    std::fs::create_dir_all(fixture.path().join("themes")).unwrap();
    std::fs::write(
        fixture.path().join("themes/daylight.kdl"),
        "theme \"daylight\" base=\"maxgus-light\" {\n    face \"region\" bg=\"#cceeff\"\n}\n",
    )
    .unwrap();
    let config = fixture.path().join("config.kdl");
    std::fs::write(&config, "set tab-width=4\nset theme=\"maxgus-dark\"\n").unwrap();

    let mut session = Session::start(fixture.path(), &["--config", "config.kdl", "hello.txt"]);

    session.send(b"\x1bx");
    session.send(b"visit-theme\r");
    assert!(
        session.shows("daylight"),
        "the themes were not listed to walk through:\n{:#?}",
        session.screen()
    );

    session.send(b"daylight\r");
    assert!(
        session.shows("config file"),
        "it did not offer to write the choice down:\n{:#?}",
        session.screen()
    );

    session.send(b"yes\r");
    assert!(
        session.shows("written to"),
        "it did not report the write:\n{:#?}",
        session.screen()
    );

    let after = std::fs::read_to_string(&config).unwrap();
    assert_eq!(
        after, "set tab-width=4\nset theme=\"daylight\"\n",
        "the configuration file was not rewritten cleanly"
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn visit_theme_can_keep_a_theme_without_touching_the_config() {
    let fixture = Fixture::new("visit-theme-session");
    let config = fixture.path().join("config.kdl");
    let original = "set theme=\"maxgus-dark\"\n";
    std::fs::write(&config, original).unwrap();

    let mut session = Session::start(fixture.path(), &["--config", "config.kdl", "hello.txt"]);
    session.send(b"\x1bx");
    session.send(b"visit-theme\r");
    session.send(b"maxgus-light\r");
    session.send(b"no\r");

    assert!(
        session.shows("session only"),
        "got:\n{:#?}",
        session.screen()
    );
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        original,
        "answering no wrote to the configuration file anyway"
    );

    // And the theme really is in use, not merely reported.
    session.send(b"\x08v");
    session.send(b"theme\r");
    assert!(
        session.shows("maxgus-light"),
        "got:\n{:#?}",
        session.screen()
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn m_x_draws_a_popup_at_the_top_of_a_real_terminal() {
    // The box, the count, the fuzzy match and the cursor sitting inside it
    // are all things a real terminal can get wrong while every unit test in
    // the workspace still passes.
    let fixture = Fixture::new("mx-popup");
    let mut session = Session::start(fixture.path(), &["hello.txt"]);
    // Down one line first, so there is a buffer position for the arrows to
    // disturb if the popup lets them through.
    session.send(b"\x0e");
    assert_eq!(session.cursor().1, 1, "the editor did not move down a line");

    session.send(b"\x1bx");
    let screen = session.screen();
    // The box is centred across the frame, so its border is on the first row
    // but not at its first column.
    let column = |corner: char| {
        screen[0]
            .chars()
            .position(|c| c == corner)
            .unwrap_or_else(|| panic!("no `{corner}` on the top row:\n{screen:#?}"))
    };
    let left = column('\u{256d}');
    // Lines come back with their trailing blanks trimmed, so the right margin
    // is measured against the terminal's width rather than the line's.
    let margin = COLUMNS as usize - 1 - column('\u{256e}');
    assert!(
        left.abs_diff(margin) <= 1,
        "the popup is not centred: {left} to the left, {margin} to the right:\n{screen:#?}"
    );
    assert!(
        screen[1].contains("M-x"),
        "no prompt inside the popup:\n{screen:#?}"
    );
    assert_eq!(
        session.cursor().1,
        1,
        "the cursor is not on the popup's prompt line"
    );
    // And on the prompt's column inside the box, which moved with it when the
    // box was centred: a cursor left at the frame's edge sits on the buffer
    // behind the popup, where nothing is being typed.
    let prompt = screen[1]
        .find("M-x")
        .map(|byte| screen[1][..byte].chars().count())
        .expect("the prompt on its line");
    assert_eq!(
        session.cursor().0,
        prompt + "M-x ".chars().count(),
        "the cursor is not where the typing goes:\n{screen:#?}"
    );

    // `sbfr` is a prefix of nothing and a subsequence of `save-buffer`.
    session.send(b"sbfr");
    assert!(
        session.shows("save-buffer"),
        "fuzzy matching found nothing:\n{:#?}",
        session.screen()
    );

    // The arrows and the page keys walk the list. Whatever they do to it,
    // they must not reach the buffer underneath.
    session.send(b"\x1b[B\x1b[B\x1b[6~\x1b[A\x1b[5~");
    session.send(b"\x07"); // C-g
    assert!(
        !session.shows("M-x sbfr"),
        "the prompt is still up:\n{:#?}",
        session.screen()
    );
    assert_eq!(
        session.cursor().1,
        1,
        "the arrows moved the buffer's cursor"
    );
    assert_eq!(session.quit(), 0);
}

#[cfg(feature = "full")]
#[test]
fn a_real_shell_runs_in_the_terminal_panel() {
    // Everything else about the terminal is tested against an emulator fed
    // bytes. This is the only test that opens a pseudo-terminal, starts a
    // real shell in it, types at it and reads what came back — which is the
    // whole of the part that cannot be faked.
    let fixture = Fixture::new("terminal");
    let mut session = Session::start(fixture.path(), &["hello.txt"]);

    session.send(b"\x18tv"); // C-x t v
    assert!(
        wait_for(&mut session, "1 ", 40),
        "no tab bar appeared:\n{:#?}",
        session.screen()
    );

    // `sh` reads a line and runs it. The marker is deliberately unlike
    // anything the editor draws, so finding it means the shell echoed it.
    session.send(b"echo maxgus-terminal-works\r");
    assert!(
        wait_for(&mut session, "maxgus-terminal-works", 60),
        "the shell did not run the command:\n{:#?}",
        session.screen()
    );

    // A second tab, and the bar naming both.
    session.send(b"\x03t"); // C-c t
    assert!(
        wait_for(&mut session, " 2 ", 40),
        "no second tab:\n{:#?}",
        session.screen()
    );

    // And the editor's own prefix still works from inside it.
    session.send(b"\x18tv");
    assert_eq!(session.quit(), 0);
}

/// Runs git in a directory and returns its output.
#[cfg(feature = "full")]
fn git_in(directory: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[cfg(feature = "full")]
#[test]
fn a_hunk_is_staged_through_the_running_editor() {
    // The whole chain, with nothing faked: a real repository, the real
    // binary, real keystrokes, and git's own index checked afterwards.
    // Staging one hunk and leaving the other is the operation magit exists
    // for, and the one where being wrong stages code nobody looked at.
    let fixture = Fixture::new("magit");
    let repo = fixture.path();
    for args in [
        vec!["init", "--initial-branch=main"],
        vec!["config", "user.email", "test@example.invalid"],
        vec!["config", "user.name", "Test"],
    ] {
        git_in(repo, &args);
    }
    let numbered: String = (1..=20).map(|n| format!("line {n}\n")).collect();
    std::fs::write(repo.join("file.txt"), &numbered).unwrap();
    git_in(repo, &["add", "."]);
    git_in(repo, &["commit", "-m", "first"]);

    // Two edits, far enough apart to be two hunks.
    let edited = numbered
        .replace("line 2\n", "LINE TWO\n")
        .replace("line 18\n", "LINE EIGHTEEN\n");
    std::fs::write(repo.join("file.txt"), &edited).unwrap();

    let mut session = Session::start(repo, &["file.txt"]);
    session.send(b"\x18g"); // C-x g
    assert!(
        wait_for(&mut session, "Unstaged changes", 60),
        "no status view:\n{:#?}",
        session.screen()
    );
    assert!(
        session.shows("Head:"),
        "no head line:\n{:#?}",
        session.screen()
    );

    // To the unstaged section, down to the file, open it, down to the first
    // hunk, and stage that one.
    session.send(b"\x1bn"); // M-n
    while !session
        .screen()
        .iter()
        .any(|line| line.contains("Unstaged changes"))
    {
        session.send(b"\x1bn");
    }
    // `M-n` lands on the heading; `n` steps to the file under it.
    session.send(b"n");
    session.send(b"\t");
    assert!(
        wait_for(&mut session, "@@", 40),
        "the file did not open:\n{:#?}",
        session.screen()
    );
    session.send(b"n");
    session.send(b"s");

    // Give the staging and the refresh that follows it time to land.
    assert!(
        wait_for(&mut session, "Staged changes", 60),
        "nothing was staged:\n{:#?}",
        session.screen()
    );

    let staged = git_in(repo, &["diff", "--cached"]);
    assert!(
        staged.contains("LINE TWO"),
        "the chosen hunk was not staged:\n{staged}"
    );
    assert!(
        !staged.contains("LINE EIGHTEEN"),
        "the whole file was staged instead of one hunk:\n{staged}"
    );
    let unstaged = git_in(repo, &["diff"]);
    assert!(
        unstaged.contains("LINE EIGHTEEN"),
        "the other hunk was lost:\n{unstaged}"
    );

    assert_eq!(session.quit(), 0);
}

#[cfg(feature = "full")]
#[test]
fn the_git_menus_are_driven_from_the_keyboard() {
    // The menus are how magit is used, and a menu that does not take the
    // keyboard is a menu that quietly does something else instead.
    let fixture = Fixture::new("magit-menu");
    let repo = fixture.path();
    for args in [
        vec!["init", "--initial-branch=main"],
        vec!["config", "user.email", "test@example.invalid"],
        vec!["config", "user.name", "Test"],
    ] {
        git_in(repo, &args);
    }
    std::fs::write(repo.join("file.txt"), "one\n").unwrap();
    git_in(repo, &["add", "."]);
    git_in(repo, &["commit", "-m", "first"]);
    std::fs::write(repo.join("file.txt"), "two\n").unwrap();

    let mut session = Session::start(repo, &["file.txt"]);
    session.send(b"\x18g"); // C-x g
    assert!(
        wait_for(&mut session, "Unstaged changes", 60),
        "no status view"
    );

    session.send(b"?");
    assert!(
        wait_for(&mut session, "Manipulate", 40),
        "the menu did not open:\n{:#?}",
        session.screen()
    );

    // Into the push menu, and turn a switch on.
    session.send(b"P");
    assert!(
        wait_for(&mut session, "Force with lease", 40),
        "the push menu did not open:\n{:#?}",
        session.screen()
    );
    session.send(b"-f");
    assert!(
        session
            .screen()
            .iter()
            .any(|line| line.contains("Force with lease") && line.contains('\u{2713}')),
        "the switch is not marked as on:\n{:#?}",
        session.screen()
    );

    // Back out, twice, and the menu is gone.
    session.send(b"\x07\x07"); // C-g C-g
    assert!(
        !session
            .screen()
            .iter()
            .any(|line| line.contains("Force with lease")),
        "the menu would not close:\n{:#?}",
        session.screen()
    );
    // And the status view is still there and still working.
    assert!(session.shows("Unstaged changes"));
    assert_eq!(session.quit(), 0);
}

#[cfg(feature = "full")]
#[test]
fn a_commit_is_shown_in_full_from_the_log() {
    let fixture = Fixture::new("magit-log");
    let repo = fixture.path();
    for args in [
        vec!["init", "--initial-branch=main"],
        vec!["config", "user.email", "test@example.invalid"],
        vec!["config", "user.name", "Test Person"],
    ] {
        git_in(repo, &args);
    }
    std::fs::write(repo.join("file.txt"), "one\n").unwrap();
    git_in(repo, &["add", "."]);
    git_in(repo, &["commit", "-m", "the first commit"]);
    std::fs::write(repo.join("file.txt"), "one\ntwo\n").unwrap();
    git_in(repo, &["commit", "-am", "the second commit"]);

    let mut session = Session::start(repo, &["file.txt"]);
    session.send(b"\x18g");
    assert!(
        wait_for(&mut session, "Recent commits", 60),
        "no status view"
    );

    // `l l`: the log menu, then the current branch.
    session.send(b"l");
    assert!(
        wait_for(&mut session, "Current branch", 40),
        "no log menu:\n{:#?}",
        session.screen()
    );
    session.send(b"l");
    // Waited for by the buffer's own name in the mode line: the commit
    // subject is already on screen in the status view's recent commits, so
    // waiting for that would be satisfied before the log had arrived.
    assert!(
        wait_for(&mut session, "magit: log", 60),
        "no log buffer:\n{:#?}",
        session.screen()
    );
    assert!(
        session.shows("the second commit"),
        "the log is empty:\n{:#?}",
        session.screen()
    );

    // `RET` on the newest commit shows it in full: who, when, and the diff.
    session.send(b"\r");
    assert!(
        wait_for(&mut session, "magit: revision", 60),
        "no revision buffer:\n{:#?}",
        session.screen()
    );
    assert!(
        session.shows("Author:"),
        "no author line:\n{:#?}",
        session.screen()
    );
    assert!(
        session.shows("Test Person"),
        "no author:\n{:#?}",
        session.screen()
    );
    assert!(session.shows("the second commit"), "no message");
    assert!(session.shows("+two"), "no diff:\n{:#?}", session.screen());

    assert_eq!(session.quit(), 0);
}

#[test]
fn the_panel_opens_at_startup_when_the_configuration_asks() {
    // The one thing a unit test cannot check: whether the flag is read on
    // the way up, before any key has been pressed.
    let fixture = Fixture::new("panelstart");
    std::fs::write(
        fixture.path().join("config.kdl"),
        "set panel-at-startup=#true\nset nerd-font-icons=#false\n",
    )
    .unwrap();
    let mut session = Session::start(fixture.path(), &["--config", "config.kdl", "hello.txt"]);

    assert!(
        wait_for(&mut session, "*treefile*", 60),
        "the panel did not open on its own:\n{:#?}",
        session.screen()
    );
    assert!(
        session.shows("*buffers*"),
        "the buffer list is missing:\n{:#?}",
        session.screen()
    );
    assert!(session.shows("hello.txt"), "the file is not shown");
    assert_eq!(session.quit(), 0);
}

#[test]
fn the_panels_windows_are_reached_with_the_ordinary_window_keys() {
    // The whole point of making the panel three windows: `C-<up>` and
    // `C-<down>` are window movement, with nothing special about the panel.
    let fixture = Fixture::new("panelmove");
    let mut session = Session::start(fixture.path(), &["-Q", "hello.txt"]);
    session.send(b"\x18tt");
    assert!(
        wait_for(&mut session, "*buffers*", 60),
        "no panel:\n{:#?}",
        session.screen()
    );

    session.send(b"\x18t1"); // C-x t 1 : the tree
    let in_tree = session.cursor();
    assert!(in_tree.0 < 32, "not in the panel column: {in_tree:?}");

    // Down into the buffer list, which is lower in the same column.
    session.send(b"\x1b[1;5B"); // C-<down>
    let in_list = session.cursor();
    assert!(
        in_list.1 > in_tree.1,
        "C-<down> did not move down the column: {in_list:?}"
    );
    assert!(in_list.0 < 32, "it left the column: {in_list:?}");

    // And back up.
    session.send(b"\x1b[1;5A"); // C-<up>
    assert_eq!(session.cursor(), in_tree, "C-<up> did not come back");
    assert_eq!(session.quit(), 0);
}

#[cfg(feature = "full")]
#[test]
fn the_outline_fills_from_a_real_server_without_disturbing_anything() {
    // The panel and `M-x lsp-document-symbols` ask the same question of the
    // server. Only a real one sends the answers that told the two apart
    // wrongly: the panel refreshes itself more than once, and a second answer
    // used to be read as a person asking, opening a listing over the file.
    if !available("clangd") {
        eprintln!("skipping: clangd is not installed");
        return;
    }
    let fixture = c_project("panellsp");
    std::fs::write(
        fixture.path().join("config.kdl"),
        "set idle-delay-ms=100\nset panel-at-startup=#true\nset nerd-font-icons=#false\n\
         lsp \"c\" command=\"clangd\" {\n    root-markers \"compile_commands.json\"\n}\n",
    )
    .unwrap();
    let mut session = Session::start(fixture.path(), &["--config", "config.kdl", "main.c"]);

    assert!(
        wait_for(&mut session, "*symbols*", 200),
        "the outline window never appeared:\n{:#?}",
        session.screen()
    );
    assert!(
        wait_for(&mut session, "main", 100),
        "the outline is empty:\n{:#?}",
        session.screen()
    );
    // The three windows are all still there, in order, and the file with them.
    for expected in ["*treefile*", "*symbols*", "*buffers*", "main.c"] {
        assert!(
            session.shows(expected),
            "{expected} is gone:\n{:#?}",
            session.screen()
        );
    }
    assert!(
        session.shows("#include <stdio.h>"),
        "the file's text is not displayed:\n{:#?}",
        session.screen()
    );
    // Nothing popped a listing over it.
    assert!(
        !session.shows("*xref*"),
        "a listing opened:\n{:#?}",
        session.screen()
    );

    // Switching buffers asks again; the second answer must behave like the
    // first. `C-x b RET` goes to the previous buffer and back.
    session.send(b"\x18b\r");
    session.settle();
    session.send(b"\x18b\r");
    for _ in 0..40 {
        session.settle();
    }
    assert!(
        !session.shows("*xref*"),
        "a later answer opened a listing:\n{:#?}",
        session.screen()
    );
    assert!(
        session.shows("#include <stdio.h>"),
        "the file went away after a refresh:\n{:#?}",
        session.screen()
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn the_startup_time_is_reported_when_the_editor_opens() {
    // The first thing the echo area says, and it has to be a real measurement
    // rather than a fixed string.
    let fixture = Fixture::new("startup");
    let mut session = Session::start(fixture.path(), &["-Q", "hello.txt"]);
    assert!(
        wait_for(&mut session, "maxgus started in", 60),
        "no startup time:\n{:#?}",
        session.screen()
    );
    let line = session
        .screen()
        .into_iter()
        .find(|l| l.contains("maxgus started in"))
        .expect("the line");
    let reported = line
        .split("started in ")
        .nth(1)
        .expect("a duration")
        .trim()
        .to_string();
    assert!(
        reported.ends_with("ms") || reported.ends_with('s'),
        "not a duration: `{reported}`"
    );
    let value: f64 = reported
        .trim_end_matches("ms")
        .trim_end_matches('s')
        .parse()
        .unwrap_or_else(|_| panic!("not a number: `{reported}`"));
    assert!(value > 0.0, "it claims to have taken no time at all");

    // And `M-x startup-time` says it again once the echo area has moved on.
    session.send(b"\x0e");
    session.settle();
    session.send(b"\x1bxstartup-time\r");
    assert!(
        wait_for(&mut session, "maxgus started in", 60),
        "`M-x startup-time` said nothing:\n{:#?}",
        session.screen()
    );
    assert_eq!(session.quit(), 0);
}

/// All three builds have to build, not just the one being worked on.
///
/// Ignored by default because it is a build rather than a test — it takes
/// minutes. `cargo test -- --ignored` runs it, and so does the CI.
#[test]
#[ignore]
fn every_build_builds() {
    for features in [Some("minimal"), Some("full"), Some("gui")] {
        let mut command = std::process::Command::new(env!("CARGO"));
        command.args(["build", "-q", "-p", "maxgus", "--no-default-features"]);
        if let Some(features) = features {
            command.args(["--features", features]);
        }
        let output = command
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("cargo runs");
        let complaints = String::from_utf8_lossy(&output.stderr);
        let named = features.unwrap_or("none");
        assert!(
            output.status.success(),
            "`--features {named}` does not build:\n{complaints}"
        );
        // A build that only compiles by accident of a warning is not one
        // anyone should ship.
        assert!(
            !complaints.contains("warning:"),
            "`--features {named}` builds with warnings:\n{complaints}"
        );
    }
}

#[cfg(feature = "full")]
#[test]
fn a_project_is_searched_and_the_results_are_edited_back_into_it() {
    // The whole of it against real files: the walk, the ignore rules, the
    // results buffer, and the rename written back to disk.
    let fixture = Fixture::new("grep");
    let root = fixture.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    std::fs::write(root.join("src/one.rs"), "fn alpha() {}\nfn other() {}\n").unwrap();
    std::fs::write(root.join("src/two.rs"), "// alpha is here\n").unwrap();
    std::fs::write(root.join("target/built.rs"), "fn alpha() {}\n").unwrap();

    let mut session = Session::start(root, &["-Q", "src/one.rs"]);
    assert!(
        wait_for(&mut session, "maxgus started in", 60),
        "no startup"
    );

    // M-s g alpha RET
    session.send(b"\x1bsg");
    session.settle();
    session.send(b"alpha\r");
    assert!(
        wait_for(&mut session, "matches for `alpha`", 100),
        "no results:\n{:#?}",
        session.screen()
    );
    let screen = session.screen();
    let shows = |needle: &str| screen.iter().any(|l| l.contains(needle));
    assert!(shows("one.rs"), "the first file is missing:\n{screen:#?}");
    assert!(shows("two.rs"), "the second file is missing:\n{screen:#?}");
    assert!(
        !shows("built.rs"),
        "it searched an ignored directory:\n{screen:#?}"
    );

    // Which file point starts on: the heading above the first result line.
    // The walk decides the order, so it is read rather than assumed.
    let first_result = screen
        .iter()
        .position(|l| l.trim_start().starts_with("1:"))
        .expect("a result line");
    let heading = screen[..first_result]
        .iter()
        .rev()
        .find(|l| l.contains(".rs") && !l.trim_start().starts_with("1:"))
        .expect("a file heading")
        .trim()
        .to_string();
    let editing_one = heading.ends_with("one.rs");

    // Make the results editable, replace the whole result line, and apply.
    session.send(b"\x03\x10"); // C-c C-p
    session.settle();
    session.send(b"\x01\x0b"); // C-a C-k : the line, without its newline
    session.send(b"     1:fn renamed() {}");
    session.settle();
    session.send(b"\x03\x03"); // C-c C-c
    assert!(
        wait_for(&mut session, "Wrote 1 line", 100),
        "nothing was written:\n{:#?}",
        session.screen()
    );

    let (edited, untouched) = match editing_one {
        true => ("src/one.rs", "src/two.rs"),
        false => ("src/two.rs", "src/one.rs"),
    };
    let written = std::fs::read_to_string(root.join(edited)).unwrap();
    assert!(
        written.starts_with("fn renamed() {}"),
        "{edited} was not rewritten: {written:?}"
    );
    if editing_one {
        assert!(
            written.contains("fn other() {}"),
            "the rest of the file was lost: {written:?}"
        );
    }
    let other = std::fs::read_to_string(root.join(untouched)).unwrap();
    assert!(
        other.contains("alpha"),
        "{untouched} was rewritten too: {other:?}"
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn a_real_editorconfig_decides_how_a_file_is_indented() {
    // The parsing and the walk up the tree, against real files. What is
    // checked is the width a tab is *drawn* at, which is the setting having
    // an effect rather than merely being stored.
    let fixture = Fixture::new("editorconfig");
    let root = fixture.path();
    std::fs::write(
        root.join(".editorconfig"),
        "root = true\n\
         \n\
         [*]\n\
         indent_style = tab\n\
         indent_size = 8\n\
         \n\
         [*.txt]\n\
         indent_size = 3\n",
    )
    .unwrap();
    std::fs::write(root.join("narrow.txt"), "\tX\n").unwrap();
    std::fs::write(root.join("wide.md"), "\tX\n").unwrap();
    // Neither 3 nor 8: whichever width shows up came from the project.
    std::fs::write(
        root.join("config.kdl"),
        "set tab-width=4\nset nerd-font-icons=#false\nset line-numbers=#false\n",
    )
    .unwrap();

    let mut session = Session::start(root, &["--config", "config.kdl", "narrow.txt"]);
    assert!(
        wait_for(&mut session, "maxgus started in", 60),
        "no startup"
    );

    let column_of_x = |session: &mut Session| -> usize {
        session
            .screen()
            .iter()
            .find(|line| line.trim() == "X")
            .and_then(|line| line.find('X'))
            .unwrap_or_else(|| panic!("no `X` on screen:\n{:#?}", session.screen()))
    };
    assert_eq!(
        column_of_x(&mut session),
        3,
        "`[*.txt] indent_size = 3` did not take"
    );

    // The other file falls under `[*]`, which asks for eight.
    session.send(b"\x18\x06wide.md\r"); // C-x C-f wide.md RET
    assert!(
        wait_for(&mut session, "wide.md", 60),
        "the second file did not open:\n{:#?}",
        session.screen()
    );
    assert_eq!(
        column_of_x(&mut session),
        8,
        "`[*] indent_size = 8` did not take"
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn a_session_brings_back_the_files_that_were_open() {
    // The whole round trip through a real state directory: what was open,
    // where point was in it, and which one was showing.
    let fixture = Fixture::new("session");
    let root = fixture.path();
    std::fs::write(root.join("one.txt"), "first\nsecond\nthird\n").unwrap();
    std::fs::write(root.join("two.txt"), "alpha\nbeta\n").unwrap();
    std::fs::write(
        root.join("config.kdl"),
        "set session=#true\nset nerd-font-icons=#false\n",
    )
    .unwrap();
    // Its own state directory, so this test cannot read or write the real
    // one and cannot be affected by whatever is in it.
    let state = root.join("state");
    std::fs::create_dir_all(&state).unwrap();

    {
        let mut session = Session::start_with_env(
            root,
            &["--config", "config.kdl", "one.txt"],
            &[("XDG_DATA_HOME", state.to_string_lossy().as_ref())],
        );
        assert!(
            wait_for(&mut session, "maxgus started in", 60),
            "no startup"
        );
        // Open the second file and move down two lines in it.
        session.send(b"\x18\x06two.txt\r"); // C-x C-f two.txt RET
        assert!(
            wait_for(&mut session, "alpha", 60),
            "the second file did not open"
        );
        session.send(b"\x0e"); // C-n
        session.settle();
        assert_eq!(session.quit(), 0);
    }

    // Started again in the same project, with no file named.
    let mut session = Session::start_with_env(
        root,
        &["--config", "config.kdl"],
        &[("XDG_DATA_HOME", state.to_string_lossy().as_ref())],
    );
    assert!(
        wait_for(&mut session, "alpha", 100),
        "the session did not come back:\n{:#?}",
        session.screen()
    );
    // Both files, and point where it was left.
    session.send(b"\x18\x02"); // C-x C-b
    assert!(
        wait_for(&mut session, "one.txt", 60),
        "the other file was not restored:\n{:#?}",
        session.screen()
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn naming_a_file_means_that_file_rather_than_the_last_session() {
    let fixture = Fixture::new("sessionarg");
    let root = fixture.path();
    std::fs::write(root.join("wanted.txt"), "the one asked for\n").unwrap();
    std::fs::write(root.join("other.txt"), "not this one\n").unwrap();
    std::fs::write(
        root.join("config.kdl"),
        "set session=#true\nset nerd-font-icons=#false\n",
    )
    .unwrap();
    let state = root.join("state");
    std::fs::create_dir_all(&state).unwrap();
    let env = [("XDG_DATA_HOME", state.to_string_lossy().to_string())];
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();

    {
        let mut session =
            Session::start_with_env(root, &["--config", "config.kdl", "other.txt"], &env);
        assert!(wait_for(&mut session, "not this one", 60), "no first run");
        assert_eq!(session.quit(), 0);
    }

    let mut session =
        Session::start_with_env(root, &["--config", "config.kdl", "wanted.txt"], &env);
    assert!(
        wait_for(&mut session, "the one asked for", 60),
        "the named file did not open:\n{:#?}",
        session.screen()
    );
    assert!(
        !session.shows("not this one"),
        "the session was restored over the file that was asked for:\n{:#?}",
        session.screen()
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn a_snippet_file_is_found_and_expanded() {
    // The loading from disk, in the layout yasnippet uses: a directory per
    // mode, a file per snippet, with the header the same shape.
    let fixture = Fixture::new("snippets");
    let root = fixture.path();
    let snippets = root.join("snippets/rust-mode");
    std::fs::create_dir_all(&snippets).unwrap();
    std::fs::write(
        snippets.join("forloop"),
        "# -*- mode: snippet -*-\n\
         # name: a for loop\n\
         # key: fori\n\
         # --\n\
         for ${1:item} in ${2:items} {\n    $0\n}",
    )
    .unwrap();
    // One for every mode, directly in `snippets/`.
    std::fs::write(root.join("snippets/note"), "# key: nb\n# --\nNOTE($1): ").unwrap();
    std::fs::write(
        root.join("config.kdl"),
        "set nerd-font-icons=#false\nset line-numbers=#false\n",
    )
    .unwrap();
    std::fs::write(root.join("main.rs"), "").unwrap();

    let mut session = Session::start(root, &["--config", "config.kdl", "main.rs"]);
    assert!(
        wait_for(&mut session, "maxgus started in", 60),
        "no startup"
    );

    session.send(b"fori\t");
    assert!(
        wait_for(&mut session, "for item in items", 60),
        "the snippet did not expand:\n{:#?}",
        session.screen()
    );
    // The first field is selected, so typing replaces it.
    session.send(b"line");
    assert!(
        wait_for(&mut session, "for line in items", 60),
        "the default was not replaced:\n{:#?}",
        session.screen()
    );
    // And the second field is a tab away.
    session.send(b"\tlines");
    assert!(
        wait_for(&mut session, "for line in lines", 60),
        "the second field was not reached:\n{:#?}",
        session.screen()
    );
    // The buffer has been typed into; saving it is the simplest way to be
    // able to leave, and the fixture is thrown away afterwards anyway.
    session.send(b"\x18\x13"); // C-x C-s
    assert!(
        wait_for(&mut session, "Wrote", 60),
        "the save did not happen"
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn dired_lists_a_real_directory_and_acts_on_what_is_marked() {
    // The listing, the marks, and a deletion that really happens.
    let fixture = Fixture::new("dired");
    let root = fixture.path();
    // A directory of its own, so what is in it is exactly what this test put
    // there and counting lines means something.
    let work = root.join("work");
    std::fs::create_dir_all(work.join("subdir")).unwrap();
    for name in ["a-gone.txt", "b-gone.txt", "c-keep.txt"] {
        std::fs::write(work.join(name), "x\n").unwrap();
    }
    std::fs::write(
        root.join("config.kdl"),
        "set nerd-font-icons=#false\nset line-numbers=#false\n",
    )
    .unwrap();

    let mut session = Session::start(root, &["--config", "config.kdl", "keep.txt"]);
    assert!(
        wait_for(&mut session, "maxgus started in", 60),
        "no startup"
    );

    session.send(b"\x18d"); // C-x d
    session.settle();
    session.send(b"work\r");
    assert!(
        wait_for(&mut session, "a-gone.txt", 60),
        "the directory did not list:\n{:#?}",
        session.screen()
    );
    let screen = session.screen();
    assert!(
        screen.iter().any(|line| line.contains("subdir/")),
        "no directory in the listing:\n{screen:#?}"
    );
    assert!(
        screen.iter().any(|line| line.contains("rw")),
        "no permissions in the listing:\n{screen:#?}"
    );

    // The listing is `subdir/`, then the three files in order. Point starts
    // on `subdir`, so one `n` reaches the first file.
    session.send(b"n");
    session.settle();
    session.send(b"mm"); // mark a-gone.txt and b-gone.txt
    session.settle();
    session.send(b"D");
    session.settle();
    session.send(b"yes\r");
    assert!(
        wait_for(&mut session, "Deleted 2 item(s)", 100),
        "the deletion did not happen:\n{:#?}",
        session.screen()
    );

    assert!(
        !work.join("a-gone.txt").exists(),
        "a-gone.txt is still there"
    );
    assert!(
        !work.join("b-gone.txt").exists(),
        "b-gone.txt is still there"
    );
    assert!(
        work.join("c-keep.txt").exists(),
        "it deleted the wrong thing"
    );
    assert!(work.join("subdir").exists(), "it deleted the directory");
    assert_eq!(session.quit(), 0);
}

#[cfg(feature = "full")]
#[test]
fn a_script_beside_the_configuration_defines_a_real_command() {
    // The file being found, loaded, offered by `M-x` and run.
    let fixture = Fixture::new("script");
    let root = fixture.path();
    std::fs::write(
        root.join("config.kdl"),
        "set nerd-font-icons=#false\nset line-numbers=#false\n",
    )
    .unwrap();
    std::fs::write(
        root.join("init.rhai"),
        r#"
        fn surround(ctx) {
            insert("<<");
            run("end-of-line");
        }
        define("surround-line", "Put marks around this line.", surround);

        fn say_where(ctx) {
            message(`line ${ctx.line + 1} of ${ctx.buffer}`);
        }
        define("say-where", "Say where point is.", say_where);
        "#,
    )
    .unwrap();
    std::fs::write(root.join("notes.txt"), "first line\nsecond line\n").unwrap();

    let mut session = Session::start(root, &["--config", "config.kdl", "notes.txt"]);
    assert!(
        wait_for(&mut session, "maxgus started in", 100),
        "no startup"
    );

    // A command the script defined, run by name. Whether the load *said*
    // anything is a race with the startup greeting; whether the command
    // exists is not.
    session.send(b"\x1bxsay-where\r");
    assert!(
        wait_for(&mut session, "line 1 of notes.txt", 60),
        "the script command said nothing:\n{:#?}",
        session.screen()
    );

    // And one that edits, and hands on to a built-in command.
    session.send(b"\x1bxsurround-line\r");
    assert!(
        wait_for(&mut session, "<<first line", 60),
        "the script command did not edit:\n{:#?}",
        session.screen()
    );

    // `M-x` offers it, which is what makes it a command rather than a
    // function nobody can reach.
    session.send(b"\x1bxsurround");
    assert!(
        wait_for(&mut session, "Put marks around this line", 60),
        "`M-x` does not offer it with its documentation:\n{:#?}",
        session.screen()
    );
    session.send(b"\x07"); // C-g
    session.settle();
    session.send(b"\x18\x13"); // C-x C-s
    assert!(
        wait_for(&mut session, "Wrote", 60),
        "the save did not happen"
    );
    assert_eq!(session.quit(), 0);
}

#[test]
fn the_version_says_which_build_this_is() {
    // Three binaries that look identical and are not.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_maxgus"))
        .arg("--version")
        .output()
        .expect("it runs");
    let said = String::from_utf8_lossy(&output.stdout);
    assert!(said.starts_with("maxgus "), "got {said:?}");
    let expected = if cfg!(feature = "gui") {
        "(gui)"
    } else if cfg!(feature = "full") {
        "(full)"
    } else {
        "(minimal)"
    };
    assert!(
        said.contains(expected),
        "`--version` says {said:?}, and this build is {expected}"
    );
}
