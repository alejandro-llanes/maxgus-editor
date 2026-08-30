//! The command registry.
//!
//! A command is a named function over editor state. Every key binding, every
//! `M-x` completion and every entry in `describe-bindings` resolves through
//! this table, so a command that is bound but not registered is caught by a
//! test rather than at run time.

use crate::{Result, editor::Editor, prefix::Prefix};
use maxgus_keys::Key;
use std::collections::BTreeMap;

/// What a command is told about how it was invoked.
#[derive(Debug, Clone, Default)]
pub struct Args {
    /// The prefix argument in effect.
    pub prefix: Prefix,
    /// The key that triggered it, which `self-insert-command` needs and
    /// `describe-key` reports.
    pub key: Option<Key>,
    /// Set when the command was re-entered with the character it asked for.
    /// `zap-to-char` and `quoted-insert` read their argument this way, as
    /// Emacs' `read-char` does.
    pub read_char: Option<Key>,
    /// Set when the command was re-entered with the text a prompt collected.
    /// `find-file` and `switch-to-buffer` read their argument this way.
    pub input: Option<String>,
}

impl Args {
    pub fn new(prefix: Prefix, key: Option<Key>) -> Args {
        Args {
            prefix,
            key,
            read_char: None,
            input: None,
        }
    }

    /// The same arguments, carrying the character a command asked for.
    pub fn with_read_char(prefix: Prefix, key: Key) -> Args {
        Args {
            prefix,
            key: Some(key),
            read_char: Some(key),
            input: None,
        }
    }

    /// The same arguments, carrying the text a prompt collected.
    pub fn with_input(prefix: Prefix, input: String) -> Args {
        Args {
            prefix,
            key: None,
            read_char: None,
            input: Some(input),
        }
    }

    /// A repeat count of at least one.
    pub fn count(&self) -> usize {
        self.prefix.positive_count().max(1)
    }

    /// The signed count, for commands that move in either direction.
    pub fn signed_count(&self) -> i32 {
        self.prefix.count()
    }
}

/// A registered command.
#[derive(Clone, Copy)]
pub struct Command {
    pub name: &'static str,
    /// The first line is what `M-x` shows as an annotation; the whole string
    /// is what `describe-function` prints.
    pub doc: &'static str,
    pub handler: fn(&mut Editor, &Args) -> Result<()>,
    /// False for commands that exist only as key bindings and should not be
    /// offered by `M-x` — the prefix-argument commands, mostly.
    pub interactive: bool,
}

impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Command").field("name", &self.name).finish()
    }
}

impl Command {
    /// The one-line summary shown beside the name in completion.
    pub fn summary(&self) -> &str {
        self.doc.lines().next().unwrap_or_default()
    }
}

/// Every command the editor knows, by name.
#[derive(Debug, Default, Clone)]
pub struct Registry {
    commands: BTreeMap<&'static str, Command>,
}

impl Registry {
    pub fn new() -> Registry {
        Registry::default()
    }

    /// Adds a command, replacing any of the same name.
    pub fn register(&mut self, command: Command) {
        self.commands.insert(command.name, command);
    }

    /// Adds every command in `commands`.
    pub fn register_all(&mut self, commands: &[Command]) {
        for command in commands {
            self.register(*command);
        }
    }

    pub fn get(&self, name: &str) -> Option<&Command> {
        self.commands.get(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.commands.contains_key(name)
    }

    pub fn len(&self) -> usize {
        self.commands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Every command name, sorted.
    pub fn names(&self) -> Vec<String> {
        self.commands.keys().map(|n| n.to_string()).collect()
    }

    /// Names `M-x` offers, sorted.
    pub fn interactive_names(&self) -> Vec<String> {
        self.commands
            .values()
            .filter(|c| c.interactive)
            .map(|c| c.name.to_string())
            .collect()
    }

    /// Interactive names starting with `prefix`, for completion.
    pub fn complete(&self, prefix: &str) -> Vec<String> {
        self.interactive_names()
            .into_iter()
            .filter(|n| n.starts_with(prefix))
            .collect()
    }

    /// Runs `name`, reporting an unknown command the way Emacs does.
    pub fn execute(&self, editor: &mut Editor, name: &str, args: &Args) -> Result<()> {
        let Some(command) = self.get(name) else {
            // A name the editor does not know may be one a script defined.
            // Scripts are looked at last, so nothing a script defines can
            // take a built-in command's name out from under it.
            #[cfg(feature = "full")]
            if editor.has_script_command(name) {
                return crate::commands::script::run(editor, name);
            }
            return Err(crate::CoreError::UnknownCommand(name.to_string()));
        };
        (command.handler)(editor, args)
    }

    /// Every registered command, sorted by name.
    pub fn iter(&self) -> impl Iterator<Item = &Command> {
        self.commands.values()
    }
}

/// Declares a command with its documentation and handler.
///
/// The macro exists so the name appears exactly once: as the string the
/// registry is keyed by and as the identifier the handler is written against,
/// which is where a typo would otherwise hide.
#[macro_export]
macro_rules! command {
    ($name:literal, $doc:literal, $handler:expr) => {
        $crate::command::Command {
            name: $name,
            doc: $doc,
            handler: $handler,
            interactive: true,
        }
    };
    ($name:literal, $doc:literal, $handler:expr, non_interactive) => {
        $crate::command::Command {
            name: $name,
            doc: $doc,
            handler: $handler,
            interactive: false,
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use maxgus_config::Settings;
    use maxgus_faces::defaults;
    use maxgus_tui::Rect;

    fn editor() -> Editor {
        Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 80, 24),
        )
    }

    fn noop(_: &mut Editor, _: &Args) -> Result<()> {
        Ok(())
    }

    fn shout(editor: &mut Editor, args: &Args) -> Result<()> {
        editor.message(format!("ran {} times", args.count()));
        Ok(())
    }

    fn failing(_: &mut Editor, _: &Args) -> Result<()> {
        Err(crate::CoreError::Message("deliberate failure".into()))
    }

    fn registry() -> Registry {
        let mut r = Registry::new();
        r.register(command!("do-nothing", "Do nothing at all.", noop));
        r.register(command!(
            "shout",
            "Say how many times.\nA second line.",
            shout
        ));
        r.register(command!("fail", "Always fail.", failing));
        r.register(command!(
            "universal-argument",
            "Prefix.",
            noop,
            non_interactive
        ));
        r
    }

    #[test]
    fn a_registered_command_can_be_found_and_run() {
        let r = registry();
        let mut e = editor();
        assert!(r.contains("do-nothing"));
        r.execute(&mut e, "do-nothing", &Args::default()).unwrap();
    }

    #[test]
    fn an_unknown_command_is_reported_by_name() {
        let r = registry();
        let mut e = editor();
        let err = r
            .execute(&mut e, "no-such-command", &Args::default())
            .unwrap_err();
        assert!(err.to_string().contains("no-such-command"), "got `{err}`");
    }

    #[test]
    fn a_failing_command_propagates_its_error() {
        let r = registry();
        let mut e = editor();
        let err = r.execute(&mut e, "fail", &Args::default()).unwrap_err();
        assert!(err.to_string().contains("deliberate failure"));
    }

    #[test]
    fn a_command_sees_its_prefix_argument() {
        let r = registry();
        let mut e = editor();
        r.execute(&mut e, "shout", &Args::new(Prefix::Numeric(7), None))
            .unwrap();
        assert_eq!(e.minibuffer.display(), "ran 7 times");
    }

    #[test]
    fn the_count_is_at_least_one_even_for_a_negative_argument() {
        assert_eq!(Args::new(Prefix::None, None).count(), 1);
        assert_eq!(Args::new(Prefix::Numeric(-5), None).count(), 1);
        assert_eq!(Args::new(Prefix::Numeric(-5), None).signed_count(), -5);
        assert_eq!(Args::new(Prefix::Universal(1), None).count(), 4);
    }

    #[test]
    fn registering_the_same_name_twice_replaces_it() {
        let mut r = registry();
        assert_eq!(r.len(), 4);
        r.register(command!("do-nothing", "Replaced.", noop));
        assert_eq!(r.len(), 4);
        assert_eq!(r.get("do-nothing").unwrap().doc, "Replaced.");
    }

    #[test]
    fn names_come_back_sorted() {
        let r = registry();
        let names = r.names();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn non_interactive_commands_are_hidden_from_m_x() {
        let r = registry();
        assert!(r.names().contains(&"universal-argument".to_string()));
        assert!(
            !r.interactive_names()
                .contains(&"universal-argument".to_string()),
            "prefix commands are not offered by M-x"
        );
    }

    #[test]
    fn completion_filters_by_prefix() {
        let mut r = registry();
        r.register(command!("do-something", "Something.", noop));
        assert_eq!(r.complete("do-"), vec!["do-nothing", "do-something"]);
        assert_eq!(r.complete("sh"), vec!["shout"]);
        assert!(r.complete("zzz").is_empty());
    }

    #[test]
    fn the_summary_is_the_first_line_of_the_documentation() {
        let r = registry();
        assert_eq!(r.get("shout").unwrap().summary(), "Say how many times.");
        assert_eq!(r.get("do-nothing").unwrap().summary(), "Do nothing at all.");
    }

    #[test]
    fn an_empty_registry_reports_itself_as_empty() {
        let r = Registry::new();
        assert!(r.is_empty());
        assert!(r.names().is_empty());
        assert!(r.get("anything").is_none());
    }

    #[test]
    fn commands_can_be_registered_in_bulk() {
        let mut r = Registry::new();
        r.register_all(&[command!("a", "A.", noop), command!("b", "B.", noop)]);
        assert_eq!(r.len(), 2);
        assert_eq!(r.iter().count(), 2);
    }
}
