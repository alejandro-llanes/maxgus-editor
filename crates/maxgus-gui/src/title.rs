//! The window's title.
//!
//! A taskbar full of windows called `maxgus` names nothing, and one that
//! says only `main.rs` names too many things: most projects have one. The
//! title says what the window holds, whose it is, and whether it has been
//! written, from a format the user can change.

use std::path::Path;

/// What there is to say about a window.
pub struct Subject<'a> {
    /// The buffer's name, which is the file's when it has one.
    pub buffer: &'a str,
    /// The file it holds, if it holds one.
    pub file: Option<&'a Path>,
    /// The project's root.
    pub project: &'a Path,
    /// Whether the buffer differs from what is on disk.
    pub modified: bool,
    /// The program's name.
    pub program: &'a str,
    /// The user's home, so a path can be shortened to `~`.
    pub home: Option<&'a Path>,
}

/// The mark a modified buffer gets in front of its name: the one a
/// document's title bar carries on every desktop.
pub const MODIFIED: &str = "• ";

/// Fills `format` in for `subject`. `%b` is the buffer's name, `%f` the
/// file's path with the home directory as `~` (the buffer's name when
/// there is no file), `%p` the project's name, `%P` its path, `%m` the
/// mark of a modified buffer or nothing, `%a` the program's name, and
/// `%%` a per-cent sign. Anything else after a `%` is left as written,
/// so a mistyped format still says something.
pub fn render(format: &str, subject: &Subject) -> String {
    let mut out = String::with_capacity(format.len() + 32);
    let mut characters = format.chars();
    while let Some(c) = characters.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match characters.next() {
            Some('b') => out.push_str(subject.buffer),
            Some('f') => match subject.file {
                Some(file) => out.push_str(&abbreviate(file, subject.home)),
                None => out.push_str(subject.buffer),
            },
            Some('p') => out.push_str(&name_of(subject.project)),
            Some('P') => out.push_str(&abbreviate(subject.project, subject.home)),
            Some('m') => {
                if subject.modified {
                    out.push_str(MODIFIED);
                }
            }
            Some('a') => out.push_str(subject.program),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

/// A directory's own name; the whole path for the root, which has none.
fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// The path with the home directory written as `~`, which is how people
/// say it and a third of the length of how the system does.
fn abbreviate(path: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home.filter(|home| !home.as_os_str().is_empty())
        && let Ok(rest) = path.strip_prefix(home)
    {
        return match rest.as_os_str().is_empty() {
            true => "~".into(),
            false => format!("~/{}", rest.display()),
        };
    }
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject<'a>(modified: bool) -> Subject<'a> {
        Subject {
            buffer: "main.rs",
            file: Some(Path::new("/home/m/code/thing/src/main.rs")),
            project: Path::new("/home/m/code/thing"),
            modified,
            program: "maxgus",
            home: Some(Path::new("/home/m")),
        }
    }

    #[test]
    fn the_default_names_the_file_the_project_and_the_program() {
        let format = maxgus_config::Settings::default().gui_title_format;
        assert_eq!(render(&format, &subject(false)), "main.rs — thing — maxgus");
        assert_eq!(
            render(&format, &subject(true)),
            "• main.rs — thing — maxgus"
        );
    }

    #[test]
    fn every_field_has_a_letter() {
        let s = subject(true);
        assert_eq!(render("%b", &s), "main.rs");
        assert_eq!(render("%f", &s), "~/code/thing/src/main.rs");
        assert_eq!(render("%p", &s), "thing");
        assert_eq!(render("%P", &s), "~/code/thing");
        assert_eq!(render("%m", &s), "• ");
        assert_eq!(render("%a", &s), "maxgus");
        assert_eq!(render("100%%", &s), "100%");
    }

    #[test]
    fn a_buffer_with_no_file_is_named_and_a_home_that_is_not_there_is_left() {
        let s = Subject {
            buffer: "*scratch*",
            file: None,
            project: Path::new("/srv/site"),
            modified: false,
            program: "maxgus",
            home: None,
        };
        assert_eq!(render("%f in %P", &s), "*scratch* in /srv/site");
        assert_eq!(render("%m%b", &s), "*scratch*");
    }

    #[test]
    fn what_is_not_a_letter_is_left_as_written() {
        let s = subject(false);
        assert_eq!(render("%z %", &s), "%z %");
        assert_eq!(render("plain", &s), "plain");
    }

    #[test]
    fn the_root_is_its_own_name_and_home_is_a_tilde() {
        let s = Subject {
            buffer: "x",
            file: Some(Path::new("/home/m")),
            project: Path::new("/"),
            modified: false,
            program: "maxgus",
            home: Some(Path::new("/home/m")),
        };
        assert_eq!(render("%p", &s), "/");
        assert_eq!(render("%f", &s), "~");
    }
}
