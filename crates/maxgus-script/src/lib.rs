//! Extending the editor.
//!
//! Configuration says what the editor should be; a script says what it should
//! *do*. This is the difference between a file of settings and a file that
//! defines a command — and the reason the configuration language is not the
//! extension language.
//!
//! A script does not get the editor. It gets a description of what is on
//! screen and asks for a list of changes, which the editor then applies. That
//! is a deliberate limit and a useful one: it can be tested without an
//! editor, a script cannot leave the editor half-modified by failing in the
//! middle, and nothing here needs to reach into editor state across a
//! foreign-language boundary.

use rhai::{AST, Dynamic, Engine, EvalAltResult, FnPtr, Map, Scope};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    #[error("{0}")]
    Parse(String),
    #[error("{0}")]
    Run(String),
    #[error("no command called `{0}` was defined")]
    NoSuchCommand(String),
}

pub type Result<T> = std::result::Result<T, ScriptError>;

/// Something a script asked the editor to do.
///
/// Deliberately small: everything else a script might want is reachable
/// through `Run`, which is any command the editor already has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Put text in at point.
    Insert(String),
    /// Take `count` characters out, forwards from point.
    Delete(usize),
    /// Move point to a character offset.
    Goto(usize),
    /// Run one of the editor's own commands.
    Run(String),
    /// Say something in the echo area.
    Message(String),
    /// Report a problem, which stops the rest of the script's actions being
    /// applied — a script that noticed something wrong should not have its
    /// earlier edits kept.
    Fail(String),
}

/// What a script is told about the editor when it runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Context {
    pub text: String,
    pub point: usize,
    pub line: usize,
    pub column: usize,
    /// The buffer's name, as the mode line shows it.
    pub buffer: String,
    /// The file it is visiting, if it is visiting one.
    pub path: Option<String>,
    pub mode: Option<String>,
    /// The selected text, when there is a region.
    pub region: Option<String>,
    /// Where the region starts and ends, as character offsets like `point`:
    /// what a command that replaces the region has to `goto_char` and `delete`.
    pub region_start: Option<usize>,
    pub region_end: Option<usize>,
}

impl Context {
    fn to_map(&self) -> Map {
        let mut map = Map::new();
        map.insert("text".into(), self.text.clone().into());
        map.insert("point".into(), (self.point as i64).into());
        map.insert("line".into(), (self.line as i64).into());
        map.insert("column".into(), (self.column as i64).into());
        map.insert("buffer".into(), self.buffer.clone().into());
        map.insert(
            "path".into(),
            match &self.path {
                Some(path) => path.clone().into(),
                None => Dynamic::UNIT,
            },
        );
        map.insert(
            "mode".into(),
            match &self.mode {
                Some(mode) => mode.clone().into(),
                None => Dynamic::UNIT,
            },
        );
        map.insert(
            "region".into(),
            match &self.region {
                Some(region) => region.clone().into(),
                None => Dynamic::UNIT,
            },
        );
        for (key, offset) in [
            ("region_start", self.region_start),
            ("region_end", self.region_end),
        ] {
            map.insert(
                key.into(),
                match offset {
                    Some(offset) => (offset as i64).into(),
                    None => Dynamic::UNIT,
                },
            );
        }
        map
    }
}

/// A command a script defined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptCommand {
    pub name: String,
    pub doc: String,
    /// The script function to call, by name.
    pub function: String,
}

/// A loaded script: the commands it defined, and the code to run them.
pub struct Script {
    engine: Engine,
    ast: AST,
    commands: Vec<ScriptCommand>,
    actions: Arc<Mutex<Vec<Action>>>,
}

impl std::fmt::Debug for Script {
    /// The engine and the syntax tree have nothing a reader wants; what the
    /// script *defined* is the whole of what one is.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Script")
            .field("commands", &self.commands)
            .finish()
    }
}

impl Script {
    /// Loads a script, running its top level so it can define its commands.
    pub fn load(source: &str) -> Result<Script> {
        Script::load_in(source, None)
    }

    /// Loads a script that lives in `directory`, which is where its
    /// `import`s are looked for.
    ///
    /// Not where the editor happens to have been started: that is whichever
    /// project is open, and an `import "helpers"` would have run a
    /// `helpers.rhai` the project had brought with it.
    pub fn load_in(source: &str, directory: Option<&Path>) -> Result<Script> {
        let actions: Arc<Mutex<Vec<Action>>> = Arc::new(Mutex::new(Vec::new()));
        let defined: Arc<Mutex<Vec<ScriptCommand>>> = Arc::new(Mutex::new(Vec::new()));
        let mut engine = Engine::new();
        engine.set_module_resolver(match directory {
            Some(directory) => rhai::module_resolvers::FileModuleResolver::new_with_path(directory),
            None => rhai::module_resolvers::FileModuleResolver::new(),
        });

        // A script is code the user wrote, not code from the network, but a
        // runaway loop in one should still not take the editor with it.
        engine.set_max_operations(5_000_000);
        engine.set_max_call_levels(64);
        engine.set_max_string_size(10_000_000);

        let register = defined.clone();
        engine.register_fn("define", move |name: &str, doc: &str, function: FnPtr| {
            let mut defined = register.lock().expect("not poisoned");
            // Defined again, the later definition is the one that stands, as
            // a function defined twice in a file is.
            defined.retain(|command| command.name != name);
            defined.push(ScriptCommand {
                name: name.to_string(),
                doc: doc.to_string(),
                function: function.fn_name().to_string(),
            });
        });

        let record = |actions: Arc<Mutex<Vec<Action>>>| {
            move |action: Action| {
                actions.lock().expect("not poisoned").push(action);
            }
        };
        let push = record(actions.clone());
        engine.register_fn("insert", move |text: &str| {
            push(Action::Insert(text.into()))
        });
        let push = record(actions.clone());
        engine.register_fn("delete", move |count: i64| {
            push(Action::Delete(count.max(0) as usize))
        });
        // `goto_char`, as Emacs spells it, because `goto` is one of the
        // words Rhai keeps for itself: registered as `goto`, it was a name no
        // script could call, and one that tried would not load at all.
        let push = record(actions.clone());
        engine.register_fn("goto_char", move |offset: i64| {
            push(Action::Goto(offset.max(0) as usize))
        });
        let push = record(actions.clone());
        engine.register_fn("run", move |command: &str| {
            push(Action::Run(command.into()))
        });
        let push = record(actions.clone());
        engine.register_fn("message", move |text: &str| {
            push(Action::Message(text.into()))
        });
        // `fail` stops the script where it is. It only recorded the failure,
        // so a script went on to the next line — and a check such as
        // `if ctx.region == () { fail(..) }` was followed by code that used
        // the region it had just found was not there.
        let push = record(actions.clone());
        engine.register_fn(
            "fail",
            move |text: &str| -> std::result::Result<(), Box<EvalAltResult>> {
                push(Action::Fail(text.into()));
                Err(text.into())
            },
        );
        // Rhai prints to standard output, which under a terminal front end
        // is the screen the editor is drawing, written over.
        let push = record(actions.clone());
        engine.on_print(move |text| push(Action::Message(text.into())));
        let push = record(actions.clone());
        engine.on_debug(move |text, _, _| push(Action::Message(text.into())));

        let ast = engine
            .compile(source)
            .map_err(|error| ScriptError::Parse(error.to_string()))?;
        // The top level runs once, which is where `define` is called from.
        engine
            .run_ast(&ast)
            .map_err(|error| ScriptError::Run(describe(&error)))?;
        // Anything the top level asked the editor to do is not a command and
        // has nowhere to be applied.
        actions.lock().expect("not poisoned").clear();
        let commands = defined.lock().expect("not poisoned").clone();
        Ok(Script {
            engine,
            ast,
            commands,
            actions,
        })
    }

    pub fn commands(&self) -> &[ScriptCommand] {
        &self.commands
    }

    /// Runs the command called `name`, returning what it asked for.
    pub fn call(&self, name: &str, context: &Context) -> Result<Vec<Action>> {
        let command = self
            .commands
            .iter()
            .find(|command| command.name == name)
            .ok_or_else(|| ScriptError::NoSuchCommand(name.to_string()))?;
        self.actions.lock().expect("not poisoned").clear();
        let mut scope = Scope::new();
        // `eval_ast: false`, because the script's top level has already run
        // once at load. Running it again on every command would repeat
        // whatever it did — and a script that types at the top level would
        // type on every keystroke that reached one of its commands.
        let options = rhai::CallFnOptions::new().eval_ast(false);
        let outcome: std::result::Result<Dynamic, _> = self.engine.call_fn_with_options(
            options,
            &mut scope,
            &self.ast,
            &command.function,
            (Dynamic::from(context.to_map()),),
        );
        let recorded = std::mem::take(&mut *self.actions.lock().expect("not poisoned"));
        match outcome {
            Ok(_) => Ok(recorded),
            // What it did before it failed is dropped: a script that stopped
            // half way should not leave half an edit behind. A `fail` is the
            // script's own reason, said as it was written rather than as the
            // runtime error it was carried out by.
            Err(error) => match recorded.into_iter().find(|a| matches!(a, Action::Fail(_))) {
                Some(failure) => Ok(vec![failure]),
                None => Err(ScriptError::Run(describe(&error))),
            },
        }
    }
}

/// What went wrong, in words for the echo area.
fn describe(error: &EvalAltResult) -> String {
    match error {
        // Rhai's "Too many operations" is true and says nothing about why.
        EvalAltResult::ErrorTooManyOperations(position) => {
            format!("stopped for running too long, as a loop that never ends does ({position})")
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> Context {
        Context {
            text: "hello world".into(),
            point: 5,
            line: 0,
            column: 5,
            buffer: "main.rs".into(),
            path: Some("/project/main.rs".into()),
            mode: Some("rust-mode".into()),
            region: None,
            region_start: None,
            region_end: None,
        }
    }

    #[test]
    fn a_script_defines_commands() {
        let script = Script::load(
            r#"
            fn shout(ctx) { insert("!"); }
            define("shout", "Add an exclamation mark.", shout);
            "#,
        )
        .expect("it loads");
        assert_eq!(script.commands().len(), 1);
        assert_eq!(script.commands()[0].name, "shout");
        assert_eq!(script.commands()[0].doc, "Add an exclamation mark.");
    }

    #[test]
    fn a_command_asks_for_what_it_wants_done() {
        let script = Script::load(
            r#"
            fn shout(ctx) { insert("!!"); message("done"); }
            define("shout", "…", shout);
            "#,
        )
        .unwrap();
        let actions = script.call("shout", &context()).unwrap();
        assert_eq!(
            actions,
            vec![Action::Insert("!!".into()), Action::Message("done".into())]
        );
    }

    #[test]
    fn a_command_can_read_where_it_was_called() {
        let script = Script::load(
            r#"
            fn describe(ctx) {
                message(`${ctx.buffer} at ${ctx.point} in ${ctx.mode}`);
            }
            define("describe", "…", describe);
            "#,
        )
        .unwrap();
        let actions = script.call("describe", &context()).unwrap();
        assert_eq!(
            actions,
            vec![Action::Message("main.rs at 5 in rust-mode".into())]
        );
    }

    #[test]
    fn a_command_can_read_the_text_and_act_on_it() {
        let script = Script::load(
            r#"
            fn count_words(ctx) {
                let words = ctx.text.split(" ").len();
                message(`${words} words`);
            }
            define("count-words", "…", count_words);
            "#,
        )
        .unwrap();
        let actions = script.call("count-words", &context()).unwrap();
        assert_eq!(actions, vec![Action::Message("2 words".into())]);
    }

    #[test]
    fn a_command_can_run_the_editors_own_commands() {
        let script = Script::load(
            r#"
            fn save_and_say(ctx) { run("save-buffer"); message("saved"); }
            define("save-and-say", "…", save_and_say);
            "#,
        )
        .unwrap();
        assert_eq!(
            script.call("save-and-say", &context()).unwrap(),
            vec![
                Action::Run("save-buffer".into()),
                Action::Message("saved".into())
            ]
        );
    }

    #[test]
    fn a_script_that_will_not_parse_says_why() {
        let error = match Script::load("fn broken( {") {
            Err(error) => error,
            Ok(_) => panic!("a script that will not parse was accepted"),
        };
        assert!(matches!(error, ScriptError::Parse(_)), "{error}");
    }

    #[test]
    fn a_command_that_fails_keeps_none_of_what_it_did() {
        let script = Script::load(
            r#"
            fn half(ctx) { insert("a"); throw "no"; }
            define("half", "…", half);
            "#,
        )
        .unwrap();
        assert!(script.call("half", &context()).is_err());
        // And the next call starts clean rather than with the leftovers.
        let script2 = Script::load(
            r#"
            fn fine(ctx) { insert("b"); }
            define("fine", "…", fine);
            "#,
        )
        .unwrap();
        assert_eq!(
            script2.call("fine", &context()).unwrap(),
            vec![Action::Insert("b".into())]
        );
    }

    #[test]
    fn a_command_that_is_not_there_is_refused() {
        let script = Script::load("").unwrap();
        let error = script.call("nothing", &context()).unwrap_err();
        assert!(matches!(error, ScriptError::NoSuchCommand(_)), "{error}");
    }

    #[test]
    fn two_calls_do_not_see_each_others_actions() {
        let script = Script::load(
            r#"
            fn once(ctx) { insert("x"); }
            define("once", "…", once);
            "#,
        )
        .unwrap();
        let first = script.call("once", &context()).unwrap();
        let second = script.call("once", &context()).unwrap();
        assert_eq!(first, second, "the second call inherited the first's");
        assert_eq!(second.len(), 1);
    }

    #[test]
    fn a_runaway_script_is_stopped_rather_than_taking_the_editor_with_it() {
        let script = Script::load(
            r#"
            fn forever(ctx) { let n = 0; loop { n += 1; } }
            define("forever", "…", forever);
            "#,
        )
        .unwrap();
        let error = script.call("forever", &context()).unwrap_err();
        assert!(error.to_string().contains("running too long"), "{error}");
    }

    #[test]
    fn the_top_levels_own_actions_are_not_kept() {
        // A script that types at load time would type into whatever buffer
        // happened to be showing when the editor started.
        let script = Script::load(
            r#"
            insert("this should go nowhere");
            fn nothing(ctx) { }
            define("nothing", "…", nothing);
            "#,
        )
        .unwrap();
        assert!(script.call("nothing", &context()).unwrap().is_empty());
    }

    #[test]
    fn every_action_can_be_called_by_its_name() {
        // A name Rhai reserves cannot be called, and a script calling it
        // does not load, so each one is called here rather than assumed.
        let script = Script::load(
            r#"
            fn everything(ctx) {
                insert("a");
                delete(1);
                goto_char(3);
                run("save-buffer");
                message("said");
            }
            define("everything", "…", everything);
            "#,
        )
        .expect("every action's name is one a script can use");
        assert_eq!(
            script.call("everything", &context()).unwrap(),
            vec![
                Action::Insert("a".into()),
                Action::Delete(1),
                Action::Goto(3),
                Action::Run("save-buffer".into()),
                Action::Message("said".into()),
            ]
        );
    }

    #[test]
    fn fail_stops_the_script_and_says_only_why() {
        let script = Script::load(
            r#"
            fn wrap(ctx) {
                if ctx.region == () { fail("Select something first"); }
                insert(ctx.region.len().to_string());
            }
            define("wrap", "…", wrap);
            "#,
        )
        .unwrap();
        assert_eq!(
            script.call("wrap", &context()).unwrap(),
            vec![Action::Fail("Select something first".into())],
            "it carried on past the `fail`"
        );
    }

    #[test]
    fn print_is_a_message_rather_than_writing_over_the_screen() {
        let script = Script::load(
            r#"
            fn say(ctx) { print("hello"); debug("there"); }
            define("say", "…", say);
            "#,
        )
        .unwrap();
        assert_eq!(
            script.call("say", &context()).unwrap(),
            vec![
                Action::Message("hello".into()),
                Action::Message("\"there\"".into())
            ]
        );
    }

    #[test]
    fn a_command_defined_twice_is_the_later_one() {
        let script = Script::load(
            r#"
            fn one(ctx) { insert("1"); }
            fn two(ctx) { insert("2"); }
            define("number", "The first.", one);
            define("number", "The second.", two);
            "#,
        )
        .unwrap();
        assert_eq!(script.commands().len(), 1);
        assert_eq!(script.commands()[0].doc, "The second.");
        assert_eq!(
            script.call("number", &context()).unwrap(),
            vec![Action::Insert("2".into())]
        );
    }

    #[test]
    fn imports_are_looked_for_beside_the_script() {
        let directory = std::env::temp_dir().join(format!("maxgus-script-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("helpers.rhai"), "fn twice(s) { s + s }").unwrap();
        // Inside the function: the top level ran once, at load, and what it
        // imported went with it.
        let script = Script::load_in(
            r#"
            fn double(ctx) {
                import "helpers" as helpers;
                insert(helpers::twice("ab"));
            }
            define("double", "…", double);
            "#,
            Some(&directory),
        )
        .unwrap();
        assert_eq!(
            script.call("double", &context()).unwrap(),
            vec![Action::Insert("abab".into())]
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_region_is_offered_when_there_is_one() {
        let script = Script::load(
            r#"
            fn wrap(ctx) {
                if ctx.region == () { fail("no region"); }
                else { insert(`(${ctx.region})`); }
            }
            define("wrap", "…", wrap);
            "#,
        )
        .unwrap();
        assert_eq!(
            script.call("wrap", &context()).unwrap(),
            vec![Action::Fail("no region".into())]
        );
        let selected = Context {
            region: Some("world".into()),
            region_start: Some(6),
            region_end: Some(11),
            ..context()
        };
        assert_eq!(
            script.call("wrap", &selected).unwrap(),
            vec![Action::Insert("(world)".into())]
        );
    }
}
