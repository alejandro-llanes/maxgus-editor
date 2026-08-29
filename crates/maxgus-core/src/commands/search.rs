//! Search and replace.
//!
//! `isearch` runs as a minor mode: starting it pushes a keymap in which
//! printing characters extend the search string rather than inserting
//! themselves, and every keystroke moves point to the current match. Aborting
//! puts point back where it started, which is the property that makes
//! searching safe to explore with.

use crate::{
    MinibufferKind, Result, command,
    command::{Args, Registry},
    editor::Editor,
};
use maxgus_text::{Match, Motion, Range, SearchDirection, SearchKind, SearchQuery};

/// The state of an incremental search.
#[derive(Debug, Clone, PartialEq)]
pub struct Isearch {
    pub query: String,
    pub kind: SearchKind,
    pub direction: SearchDirection,
    /// Point when the search began, restored on abort.
    pub origin: usize,
    /// Where the current match sits, if the search is succeeding.
    pub current: Option<Range>,
    /// True once the search has run off the end and come back round.
    pub wrapped: bool,
    /// True when the query matches nothing.
    pub failing: bool,
    /// One entry per keystroke that extended the search, so `DEL` can step
    /// back through exactly the states the user passed through.
    pub history: Vec<(String, Option<Range>, bool)>,
}

impl Isearch {
    /// A search showing `current` as its match, for tests and for restoring a
    /// search from saved state.
    pub fn at(
        query: &str,
        kind: SearchKind,
        direction: SearchDirection,
        origin: usize,
        current: Option<Range>,
    ) -> Isearch {
        Isearch {
            query: query.to_string(),
            kind,
            direction,
            origin,
            current,
            wrapped: false,
            failing: current.is_none(),
            history: Vec::new(),
        }
    }
}

impl Isearch {
    fn new(kind: SearchKind, direction: SearchDirection, origin: usize) -> Isearch {
        Isearch {
            query: String::new(),
            kind,
            direction,
            origin,
            current: None,
            wrapped: false,
            failing: false,
            history: Vec::new(),
        }
    }

    /// The prompt shown in the echo area, in Emacs' wording.
    pub fn prompt(&self) -> String {
        let failing = if self.failing { "failing " } else { "" };
        let wrapped = if self.wrapped { "wrapped " } else { "" };
        let regexp = if self.kind == SearchKind::Regexp { " regexp" } else { "" };
        let direction = match self.direction {
            SearchDirection::Forward => "I-search",
            SearchDirection::Backward => "I-search backward",
        };
        format!("{failing}{wrapped}{direction}{regexp}: {}", self.query)
    }
}

/// The state of a `query-replace`.
#[derive(Debug, Clone)]
pub struct QueryReplace {
    pub query: SearchQuery,
    pub replacement: String,
    /// The match awaiting an answer.
    pub current: Option<Match>,
    pub replaced: usize,
    /// True once the user answered `!`, replacing the rest without asking.
    pub replace_all: bool,
}

/// The name of the buffer `occur` builds.
pub const OCCUR_NAME: &str = "*Occur*";

/// Registers the search commands.
pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!("isearch-forward", "Search incrementally forward.", isearch_forward),
        command!("isearch-backward", "Search incrementally backward.", isearch_backward),
        command!("isearch-forward-regexp", "Search incrementally forward for a regexp.", isearch_forward_regexp),
        command!("isearch-backward-regexp", "Search incrementally backward for a regexp.", isearch_backward_regexp),
        command!("isearch-printing-char", "Add the typed character to the search string.", printing_char, non_interactive),
        command!("isearch-repeat-forward", "Move to the next match.", repeat_forward, non_interactive),
        command!("isearch-repeat-backward", "Move to the previous match.", repeat_backward, non_interactive),
        command!("isearch-exit", "Finish the search, leaving point at the match.", isearch_exit, non_interactive),
        command!("isearch-abort", "Abandon the search and go back where it started.", isearch_abort, non_interactive),
        command!("isearch-delete-char", "Undo the last character of the search string.", delete_char, non_interactive),
        command!("isearch-yank-word", "Add the next word to the search string.", yank_word, non_interactive),
        command!("isearch-yank-line", "Add the rest of the line to the search string.", yank_line, non_interactive),
        command!("isearch-yank-char", "Add the next character to the search string.", yank_char, non_interactive),
        command!("isearch-yank-kill", "Add the most recent kill to the search string.", yank_kill, non_interactive),
        command!("query-replace", "Replace occurrences, asking about each.", query_replace),
        command!("query-replace-regexp", "Replace regexp matches, asking about each.", query_replace_regexp),
        command!("query-replace-answer", "Answer the query-replace prompt.", query_replace_answer, non_interactive),
        command!("occur", "List every line matching a regexp.", occur),
    ]);
}

// ---- incremental search -------------------------------------------------

fn start(editor: &mut Editor, kind: SearchKind, direction: SearchDirection) -> Result<()> {
    // Repeating the start key while searching moves to the next match, which
    // is what makes `C-s C-s` step forward.
    if editor.isearch.is_some() {
        return repeat(editor, direction);
    }
    editor.sync_to_buffer();
    let origin = editor.current_buffer().point();
    let state = Isearch::new(kind, direction, origin);
    editor.minibuffer.show_message(state.prompt());
    editor.isearch = Some(state);
    editor.push_minor_map(
        crate::keymap::isearch_keymap().expect("the built-in isearch map is well formed"),
    );
    Ok(())
}

fn isearch_forward(editor: &mut Editor, _: &Args) -> Result<()> {
    start(editor, SearchKind::Literal, SearchDirection::Forward)
}

fn isearch_backward(editor: &mut Editor, _: &Args) -> Result<()> {
    start(editor, SearchKind::Literal, SearchDirection::Backward)
}

fn isearch_forward_regexp(editor: &mut Editor, _: &Args) -> Result<()> {
    start(editor, SearchKind::Regexp, SearchDirection::Forward)
}

fn isearch_backward_regexp(editor: &mut Editor, _: &Args) -> Result<()> {
    start(editor, SearchKind::Regexp, SearchDirection::Backward)
}

/// Re-runs the search from `from` and moves point to what it finds.
///
/// `advance` is true when the search should step past the current match, as
/// repeating does; false when it should re-examine from the same place, as
/// extending the query does.
fn search_from(editor: &mut Editor, from: usize, advance: bool) -> Result<()> {
    let Some(state) = editor.isearch.as_ref() else { return Ok(()) };
    if state.query.is_empty() {
        return Ok(());
    }
    let (query_text, kind, direction) = (state.query.clone(), state.kind, state.direction);
    let case_fold = editor.settings.case_fold_search;

    let query = match SearchQuery::new(&query_text, kind, case_fold) {
        Ok(query) => query,
        Err(_) => {
            // A half-typed regexp is not an error, just not matching yet.
            if let Some(state) = editor.isearch.as_mut() {
                state.failing = true;
                state.current = None;
            }
            let prompt = editor.isearch.as_ref().expect("checked above").prompt();
            editor.minibuffer.show_message(prompt);
            return Ok(());
        }
    };

    let start = match (direction, advance) {
        (SearchDirection::Forward, true) => from + 1,
        (SearchDirection::Forward, false) => from,
        (SearchDirection::Backward, true) => from.saturating_sub(1),
        (SearchDirection::Backward, false) => from,
    };

    // The text is cached per revision, so typing a search string renders the
    // buffer once rather than once per character.
    let buffer = editor.current_buffer_id();
    editor.refresh_text_cache(buffer);
    let plain = {
        let text = editor.cached_text();
        let rope = editor.current_buffer().rope();
        match (direction, advance) {
            (SearchDirection::Forward, _) => query.search_forward_in(text, rope, start),
            // Stepping to the previous match: it must end before where we are.
            (SearchDirection::Backward, true) => query.search_backward_in(text, rope, start),
            // Growing the query: the match extends rightward from where it
            // already starts, so the limit applies to its start.
            (SearchDirection::Backward, false) => {
                query.search_backward_from_in(text, rope, start)
            }
        }
    };
    // Only wrap when a straight search found nothing, so the indicator is
    // accurate rather than always on.
    let (found, wrapped) = match plain {
        Some(m) => (Some(m), false),
        None => {
            let rope = editor.current_buffer().rope();
            (query.search_wrapping(rope, start, direction), true)
        }
    };

    match found {
        Some(m) => {
            let range = m.range;
            if let Some(state) = editor.isearch.as_mut() {
                state.current = Some(range);
                state.failing = false;
                state.wrapped = state.wrapped || wrapped;
            }
            // Point lands at the far end of the match, as Emacs leaves it.
            let point = match direction {
                SearchDirection::Forward => range.end,
                SearchDirection::Backward => range.start,
            };
            editor.with_current_buffer(|b| b.set_point(point));
            editor.follow_point();
        }
        None => {
            if let Some(state) = editor.isearch.as_mut() {
                state.failing = true;
                state.current = None;
            }
        }
    }
    let prompt = editor.isearch.as_ref().expect("checked above").prompt();
    editor.minibuffer.show_message(prompt);
    Ok(())
}

/// Records the state before a change, so `DEL` can step back to it.
fn push_history(editor: &mut Editor) {
    if let Some(state) = editor.isearch.as_mut() {
        let snapshot = (state.query.clone(), state.current, state.failing);
        state.history.push(snapshot);
    }
}

fn printing_char(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(c) = args.key.and_then(|k| k.as_char()) else { return Ok(()) };
    push_history(editor);
    let from = match editor.isearch.as_mut() {
        Some(state) => {
            state.query.push(c);
            // Extending re-searches from the start of the current match, so a
            // longer query can still match where the shorter one did.
            state.current.map_or_else(|| state.origin, |r| r.start)
        }
        None => return Ok(()),
    };
    search_from(editor, from, false)
}

/// Adds `text` to the search string, as the yank commands do.
fn extend(editor: &mut Editor, text: &str) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    push_history(editor);
    let from = match editor.isearch.as_mut() {
        Some(state) => {
            // Literal searches take the text as it stands; a regexp search
            // must escape it, or yanked punctuation would change the pattern.
            let addition = match state.kind {
                SearchKind::Literal => text.to_string(),
                SearchKind::Regexp => regex_escape(text),
            };
            state.query.push_str(&addition);
            state.current.map_or_else(|| state.origin, |r| r.start)
        }
        None => return Ok(()),
    };
    search_from(editor, from, false)
}

/// Escapes the characters that mean something in a regexp.
fn regex_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if r".^$*+?()[]{}|\".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn repeat(editor: &mut Editor, direction: SearchDirection) -> Result<()> {
    let Some(state) = editor.isearch.as_mut() else { return Ok(()) };
    state.direction = direction;
    // `C-s` with an empty search string recalls the last one, as Emacs does.
    if state.query.is_empty() {
        let previous = editor.minibuffer.history(MinibufferKind::Search).first().cloned();
        if let Some(previous) = previous {
            editor.isearch.as_mut().expect("checked above").query = previous;
            let from = editor.current_buffer().point();
            return search_from(editor, from, false);
        }
        let prompt = editor.isearch.as_ref().expect("checked above").prompt();
        editor.minibuffer.show_message(prompt);
        return Ok(());
    }
    let from = editor.current_buffer().point();
    search_from(editor, from, true)
}

fn repeat_forward(editor: &mut Editor, _: &Args) -> Result<()> {
    repeat(editor, SearchDirection::Forward)
}

fn repeat_backward(editor: &mut Editor, _: &Args) -> Result<()> {
    repeat(editor, SearchDirection::Backward)
}

/// Leaves isearch, taking the minor map with it.
fn finish(editor: &mut Editor) -> Option<Isearch> {
    let state = editor.isearch.take();
    editor.remove_minor_map("isearch-mode");
    state
}

fn isearch_exit(editor: &mut Editor, _: &Args) -> Result<()> {
    let Some(state) = finish(editor) else { return Ok(()) };
    // The starting position goes on the mark ring, so `C-u C-SPC` comes back.
    let origin = state.origin;
    editor.with_current_buffer(|b| b.push_mark(origin));
    if !state.query.is_empty() {
        // Remember the search string for the next `C-s C-s`.
        editor.minibuffer.activate(MinibufferKind::Search, "");
        for c in state.query.chars() {
            editor.minibuffer.insert_char(c);
        }
        editor.minibuffer.accept();
    }
    editor.message("Mark saved where search started");
    Ok(())
}

fn isearch_abort(editor: &mut Editor, _: &Args) -> Result<()> {
    let Some(state) = finish(editor) else { return Ok(()) };
    editor.with_current_buffer(|b| b.set_point(state.origin));
    editor.follow_point();
    editor.message("Quit");
    Ok(())
}

fn delete_char(editor: &mut Editor, _: &Args) -> Result<()> {
    let restored = match editor.isearch.as_mut() {
        Some(state) => state.history.pop(),
        None => return Ok(()),
    };
    let Some((query, current, failing)) = restored else {
        // Nothing left to undo; `DEL` at the start abandons the search.
        return isearch_abort(editor, &Args::default());
    };
    let point = {
        let state = editor.isearch.as_mut().expect("checked above");
        state.query = query;
        state.current = current;
        state.failing = failing;
        current.map_or(state.origin, |r| match state.direction {
            SearchDirection::Forward => r.end,
            SearchDirection::Backward => r.start,
        })
    };
    editor.with_current_buffer(|b| b.set_point(point));
    editor.follow_point();
    let prompt = editor.isearch.as_ref().expect("checked above").prompt();
    editor.minibuffer.show_message(prompt);
    Ok(())
}

/// The text after point that a yank command would take.
fn text_after_point(editor: &Editor, extent: impl Fn(&maxgus_text::Buffer, usize) -> usize) -> String {
    let buffer = editor.current_buffer();
    let from = buffer.point();
    let to = extent(buffer, from);
    buffer.slice(Range::new(from.min(to), to.max(from)))
}

fn yank_word(editor: &mut Editor, _: &Args) -> Result<()> {
    let text = text_after_point(editor, |b, at| Motion::forward_word(b.rope(), at, 1));
    extend(editor, &text)
}

fn yank_char(editor: &mut Editor, _: &Args) -> Result<()> {
    let text = text_after_point(editor, |b, at| (at + 1).min(b.point_max()));
    extend(editor, &text)
}

fn yank_line(editor: &mut Editor, _: &Args) -> Result<()> {
    let text = text_after_point(editor, |b, at| Motion::line_end(b.rope(), at));
    extend(editor, &text)
}

fn yank_kill(editor: &mut Editor, _: &Args) -> Result<()> {
    let Some(text) = editor.kill_ring.front().map(str::to_string) else {
        return Err(crate::CoreError::Message("Kill ring is empty".into()));
    };
    extend(editor, &text)
}

// ---- query-replace ------------------------------------------------------

/// Both `query-replace` variants collect two answers: what to look for, then
/// what to put in its place.
fn begin_replace(editor: &mut Editor, args: &Args, kind: SearchKind) -> Result<()> {
    let command = match kind {
        SearchKind::Literal => "query-replace",
        SearchKind::Regexp => "query-replace-regexp",
    };
    let Some(input) = args.input.clone() else {
        let verb = if kind == SearchKind::Regexp { "Query replace regexp" } else { "Query replace" };
        editor.prompt_for(command, MinibufferKind::Search, format!("{verb}: "), "", Vec::new());
        return Ok(());
    };

    // The two answers arrive one after the other; the first is stashed in the
    // replacement field until the second comes back.
    match editor.query_replace.take() {
        None => {
            if input.is_empty() {
                return Err(crate::CoreError::Message("Nothing to replace".into()));
            }
            let query = SearchQuery::new(&input, kind, editor.settings.case_fold_search)
                .map_err(|e| crate::CoreError::Message(format!("Invalid pattern: {e}")))?;
            editor.query_replace = Some(QueryReplace {
                query,
                replacement: String::new(),
                current: None,
                replaced: 0,
                replace_all: false,
            });
            editor.prompt_for(
                command,
                MinibufferKind::Replace,
                format!("Replace `{input}` with: "),
                "",
                Vec::new(),
            );
            Ok(())
        }
        Some(mut state) => {
            state.replacement = input;
            editor.query_replace = Some(state);
            advance_replace(editor, false)
        }
    }
}

fn query_replace(editor: &mut Editor, args: &Args) -> Result<()> {
    begin_replace(editor, args, SearchKind::Literal)
}

fn query_replace_regexp(editor: &mut Editor, args: &Args) -> Result<()> {
    begin_replace(editor, args, SearchKind::Regexp)
}

/// Finds the next match and asks about it, or finishes when there are none.
///
/// `from_current` is true when the search should resume after a replacement.
fn advance_replace(editor: &mut Editor, _from_current: bool) -> Result<()> {
    let Some(state) = editor.query_replace.as_ref() else { return Ok(()) };
    let (query, replace_all) = (state.query.clone(), state.replace_all);
    let rope = editor.current_buffer().rope().clone();
    let from = editor.current_buffer().point();

    let Some(found) = query.search_forward(&rope, from) else {
        return finish_replace(editor);
    };

    if replace_all {
        apply_replacement(editor, &found)?;
        return advance_replace(editor, true);
    }

    if let Some(state) = editor.query_replace.as_mut() {
        state.current = Some(found.clone());
    }
    editor.with_current_buffer(|b| b.set_point(found.range.start));
    editor.follow_point();
    editor.prompt_for(
        "query-replace-answer",
        MinibufferKind::Char,
        "Query replacing (y, n, !, q, .): ",
        "",
        Vec::new(),
    );
    Ok(())
}

/// Replaces one match and leaves point after the replacement.
fn apply_replacement(editor: &mut Editor, found: &Match) -> Result<()> {
    let Some(state) = editor.query_replace.as_ref() else { return Ok(()) };
    let text = state.query.expand_replacement(&state.replacement, found);
    let range = found.range;
    editor.with_current_buffer(|b| b.replace(range, &text))?;
    editor.with_current_buffer(|b| b.set_point(range.start + text.chars().count()));
    if let Some(state) = editor.query_replace.as_mut() {
        state.replaced += 1;
    }
    Ok(())
}

fn finish_replace(editor: &mut Editor) -> Result<()> {
    let replaced = editor.query_replace.take().map_or(0, |s| s.replaced);
    editor.message(match replaced {
        0 => "Replaced 0 occurrences".to_string(),
        1 => "Replaced 1 occurrence".to_string(),
        n => format!("Replaced {n} occurrences"),
    });
    Ok(())
}

/// Handles one answer at the `query-replace` prompt.
fn query_replace_answer(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(answer) = args.input.as_ref().and_then(|s| s.chars().next()) else {
        return finish_replace(editor);
    };
    let Some(found) = editor.query_replace.as_ref().and_then(|s| s.current.clone()) else {
        return finish_replace(editor);
    };

    match answer {
        // Replace this one and move on.
        'y' | ' ' => {
            apply_replacement(editor, &found)?;
            advance_replace(editor, true)
        }
        // Skip it.
        'n' => {
            editor.with_current_buffer(|b| b.set_point(found.range.end));
            advance_replace(editor, true)
        }
        // Replace this one and every one after it without asking.
        '!' => {
            if let Some(state) = editor.query_replace.as_mut() {
                state.replace_all = true;
            }
            apply_replacement(editor, &found)?;
            advance_replace(editor, true)
        }
        // Replace this one and stop.
        '.' => {
            apply_replacement(editor, &found)?;
            finish_replace(editor)
        }
        // Stop without replacing.
        'q' => finish_replace(editor),
        // Anything else re-asks rather than guessing.
        _ => {
            editor.prompt_for(
                "query-replace-answer",
                MinibufferKind::Char,
                "Please answer y, n, !, q or .: ",
                "",
                Vec::new(),
            );
            Ok(())
        }
    }
}

// ---- occur --------------------------------------------------------------

fn occur(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(pattern) = args.input.clone() else {
        editor.prompt_for("occur", MinibufferKind::Search, "List lines matching regexp: ", "", Vec::new());
        return Ok(());
    };
    if pattern.is_empty() {
        return Err(crate::CoreError::Message("Nothing to search for".into()));
    }
    let query = SearchQuery::new(&pattern, SearchKind::Regexp, editor.settings.case_fold_search)
        .map_err(|e| crate::CoreError::Message(format!("Invalid regexp: {e}")))?;

    let (listing, count) = {
        let buffer = editor.current_buffer();
        let matches = query.find_all(buffer.rope());
        let mut listing = format!("{} matches for `{pattern}` in {}:\n", matches.len(), buffer.name());
        // One entry per matching line, not per match, as `occur` reports it.
        let mut last_line = None;
        let mut lines = 0usize;
        for m in &matches {
            let line = buffer.line_of(m.range.start);
            if last_line == Some(line) {
                continue;
            }
            last_line = Some(line);
            lines += 1;
            listing.push_str(&format!("{:>6}:{}\n", line + 1, buffer.line_text(line)));
        }
        (listing, lines)
    };

    let id = match editor.buffers.find_by_name(OCCUR_NAME) {
        Some(id) => {
            editor.replace_buffer_contents(id, &listing)?;
            id
        }
        None => editor.buffers.create_with_text(OCCUR_NAME, &listing),
    };
    editor.buffers.get_mut(id).expect("just created").set_read_only(true);
    editor.switch_to_buffer(id)?;
    editor.message(format!("{count} matching line(s)"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dispatch, Dispatcher};
    use maxgus_config::Settings;
    use maxgus_faces::defaults;
    use maxgus_tui::Rect;

    fn setup(text: &str) -> (Dispatcher, Editor) {
        let mut editor = Editor::new(
            Settings::default(),
            defaults::builtin("maxgus-dark").unwrap(),
            Rect::new(0, 0, 80, 24),
        );
        let id = editor.buffers.create_with_text("test", text);
        editor.switch_to_buffer(id).unwrap();
        editor.with_current_buffer(|b| b.set_point(0));

        let mut registry = Registry::new();
        register(&mut registry);
        super::super::minibuffer::register(&mut registry);
        super::super::motion::register(&mut registry);
        super::super::edit::register(&mut registry);
        super::super::buffer::register(&mut registry);
        (Dispatcher::new(registry), editor)
    }

    fn point(e: &Editor) -> usize {
        e.windows.current().point
    }

    fn typed(d: &mut Dispatcher, e: &mut Editor, keys: &str) {
        for key in keys.split_whitespace() {
            d.handle_keys(e, key);
        }
    }

    fn answer(d: &mut Dispatcher, e: &mut Editor, text: &str) {
        assert!(e.minibuffer.is_active(), "expected a prompt");
        e.minibuffer.kill_whole();
        for c in text.chars() {
            e.minibuffer.insert_char(c);
        }
        d.handle_keys(e, "RET");
    }

    #[test]
    fn every_search_binding_is_registered() {
        let mut registry = Registry::new();
        register(&mut registry);
        for (keys, command) in crate::keymap::ISEARCH_BINDINGS {
            assert!(registry.contains(command), "`{keys}` runs unregistered `{command}`");
        }
        assert!(registry.contains("isearch-printing-char"), "the fallback binding");
    }

    #[test]
    fn searching_moves_point_to_the_end_of_the_match() {
        let (mut d, mut e) = setup("alpha beta gamma");
        d.execute(&mut e, "isearch-forward", None);
        assert!(e.isearch.is_some());
        typed(&mut d, &mut e, "b e t a");
        assert_eq!(point(&e), 10, "just past `beta`");
        assert_eq!(e.isearch.as_ref().unwrap().current, Some(Range::new(6, 10)));
        assert!(e.minibuffer.display().starts_with("I-search: beta"));
    }

    #[test]
    fn printing_characters_do_not_reach_the_buffer_while_searching() {
        let (mut d, mut e) = setup("alpha beta");
        d.execute(&mut e, "isearch-forward", None);
        typed(&mut d, &mut e, "b");
        assert_eq!(e.current_buffer().text(), "alpha beta", "nothing was inserted");
    }

    #[test]
    fn repeating_steps_to_the_next_match_and_wraps() {
        let (mut d, mut e) = setup("one two one two one");
        d.execute(&mut e, "isearch-forward", None);
        typed(&mut d, &mut e, "o n e");
        assert_eq!(point(&e), 3, "the first match");

        d.handle_keys(&mut e, "C-s");
        assert_eq!(point(&e), 11, "the second");
        d.handle_keys(&mut e, "C-s");
        assert_eq!(point(&e), 19, "the third");

        d.handle_keys(&mut e, "C-s");
        assert_eq!(point(&e), 3, "wrapped to the first");
        assert!(e.isearch.as_ref().unwrap().wrapped);
        assert!(e.minibuffer.display().contains("wrapped"));
    }

    #[test]
    fn a_search_that_matches_nothing_says_it_is_failing() {
        let (mut d, mut e) = setup("alpha beta");
        d.execute(&mut e, "isearch-forward", None);
        typed(&mut d, &mut e, "z z z");
        assert!(e.isearch.as_ref().unwrap().failing);
        assert!(e.minibuffer.display().starts_with("failing I-search"));
    }

    #[test]
    fn backspace_steps_back_through_the_search_string() {
        let (mut d, mut e) = setup("alpha beta");
        d.execute(&mut e, "isearch-forward", None);
        typed(&mut d, &mut e, "b e t z");
        assert!(e.isearch.as_ref().unwrap().failing);

        d.handle_keys(&mut e, "DEL");
        assert_eq!(e.isearch.as_ref().unwrap().query, "bet");
        assert!(!e.isearch.as_ref().unwrap().failing, "back to a matching state");
        assert_eq!(point(&e), 9);
    }

    #[test]
    fn backspace_at_the_start_abandons_the_search() {
        let (mut d, mut e) = setup("alpha");
        e.with_current_buffer(|b| b.set_point(3));
        d.execute(&mut e, "isearch-forward", None);
        d.handle_keys(&mut e, "DEL");
        assert!(e.isearch.is_none());
        assert_eq!(point(&e), 3, "point went back where it started");
    }

    #[test]
    fn aborting_puts_point_back_where_the_search_began() {
        let (mut d, mut e) = setup("alpha beta gamma");
        e.with_current_buffer(|b| b.set_point(2));
        d.execute(&mut e, "isearch-forward", None);
        typed(&mut d, &mut e, "g a m m a");
        assert_eq!(point(&e), 16);

        d.handle_keys(&mut e, "C-g");
        assert!(e.isearch.is_none());
        assert_eq!(point(&e), 2);
        assert_eq!(e.minibuffer.display(), "Quit");
    }

    #[test]
    fn exiting_keeps_point_and_marks_where_the_search_started() {
        let (mut d, mut e) = setup("alpha beta");
        d.execute(&mut e, "isearch-forward", None);
        typed(&mut d, &mut e, "b e t a");
        d.handle_keys(&mut e, "RET");

        assert!(e.isearch.is_none());
        assert_eq!(point(&e), 10, "point stayed at the match");
        assert_eq!(e.current_buffer().mark(), Some(0), "the origin was marked");
    }

    #[test]
    fn the_global_map_comes_back_once_the_search_ends() {
        let (mut d, mut e) = setup("alpha");
        d.execute(&mut e, "isearch-forward", None);
        assert_eq!(d.handle_keys(&mut e, "a").command(), Some("isearch-printing-char"));
        d.handle_keys(&mut e, "RET");
        assert_eq!(d.handle_keys(&mut e, "C-f").command(), Some("forward-char"));
    }

    #[test]
    fn a_second_search_recalls_the_previous_string() {
        let (mut d, mut e) = setup("alpha beta alpha");
        d.execute(&mut e, "isearch-forward", None);
        typed(&mut d, &mut e, "a l p h a");
        d.handle_keys(&mut e, "RET");

        e.with_current_buffer(|b| b.set_point(0));
        d.execute(&mut e, "isearch-forward", None);
        d.handle_keys(&mut e, "C-s");
        assert_eq!(e.isearch.as_ref().unwrap().query, "alpha", "recalled");
    }

    #[test]
    fn searching_backward_lands_at_the_start_of_the_match() {
        let (mut d, mut e) = setup("alpha beta alpha");
        e.with_current_buffer(|b| b.set_point(16));
        d.execute(&mut e, "isearch-backward", None);
        typed(&mut d, &mut e, "a l p h a");
        assert_eq!(point(&e), 11);
        assert!(e.minibuffer.display().starts_with("I-search backward:"));
    }

    #[test]
    fn a_regexp_search_matches_a_pattern() {
        let (mut d, mut e) = setup("foo123bar");
        d.execute(&mut e, "isearch-forward-regexp", None);
        typed(&mut d, &mut e, "\\ d +");
        assert_eq!(e.isearch.as_ref().unwrap().current, Some(Range::new(3, 6)));
        assert!(e.minibuffer.display().contains("regexp"));
    }

    #[test]
    fn a_half_typed_regexp_is_not_an_error() {
        let (mut d, mut e) = setup("text (here)");
        d.execute(&mut e, "isearch-forward-regexp", None);
        typed(&mut d, &mut e, "(");
        assert!(e.isearch.as_ref().unwrap().failing, "not matching yet");
        assert!(e.isearch.is_some(), "the search is still running");
    }

    #[test]
    fn smart_case_folds_only_for_a_lowercase_query() {
        let (mut d, mut e) = setup("Alpha alpha");
        d.execute(&mut e, "isearch-forward", None);
        typed(&mut d, &mut e, "a l p h a");
        assert_eq!(e.isearch.as_ref().unwrap().current, Some(Range::new(0, 5)), "matched `Alpha`");

        d.handle_keys(&mut e, "C-g");
        e.with_current_buffer(|b| b.set_point(0));
        d.execute(&mut e, "isearch-forward", None);
        typed(&mut d, &mut e, "A l p h a");
        assert_eq!(e.isearch.as_ref().unwrap().current, Some(Range::new(0, 5)));
    }

    #[test]
    fn yanking_a_word_extends_the_search_string() {
        let (mut d, mut e) = setup("needle haystack needle");
        d.execute(&mut e, "isearch-forward", None);
        d.handle_keys(&mut e, "C-w");
        assert_eq!(e.isearch.as_ref().unwrap().query, "needle");
        assert_eq!(point(&e), 6);
    }

    #[test]
    fn yanking_a_character_and_a_line_also_work() {
        let (mut d, mut e) = setup("abc\nxyz");
        d.execute(&mut e, "isearch-forward", None);
        d.handle_keys(&mut e, "C-M-y");
        assert_eq!(e.isearch.as_ref().unwrap().query, "a");

        d.handle_keys(&mut e, "C-g");
        e.with_current_buffer(|b| b.set_point(0));
        d.execute(&mut e, "isearch-forward", None);
        d.handle_keys(&mut e, "C-y");
        assert_eq!(e.isearch.as_ref().unwrap().query, "abc");
    }

    #[test]
    fn yanking_into_a_regexp_search_escapes_the_text() {
        // `C-y` yanks the rest of the line, so the line is the whole pattern.
        let (mut d, mut e) = setup("a.c
abc");
        d.execute(&mut e, "isearch-forward-regexp", None);
        d.handle_keys(&mut e, "C-y");
        assert_eq!(e.isearch.as_ref().unwrap().query, r"a\.c", "the dot is literal");
        assert_eq!(regex_escape("a+b*c"), r"a\+b\*c");
    }

    #[test]
    fn yanking_the_kill_ring_extends_the_search() {
        let (mut d, mut e) = setup("hello world");
        e.kill_ring.kill_new("world");
        d.execute(&mut e, "isearch-forward", None);
        d.handle_keys(&mut e, "M-y");
        assert_eq!(e.isearch.as_ref().unwrap().query, "world");
        assert_eq!(point(&e), 11);
    }

    // ---- query-replace ----

    /// Starts a replacement and answers both prompts.
    fn start_replace(d: &mut Dispatcher, e: &mut Editor, command: &str, from: &str, to: &str) {
        d.execute(e, command, None);
        answer(d, e, from);
        answer(d, e, to);
    }

    #[test]
    fn query_replace_asks_about_each_occurrence() {
        let (mut d, mut e) = setup("one two one two one");
        start_replace(&mut d, &mut e, "query-replace", "one", "1");
        assert!(e.minibuffer.is_active(), "it asked about the first match");

        d.handle_keys(&mut e, "y");
        assert_eq!(e.current_buffer().text(), "1 two one two one");
        d.handle_keys(&mut e, "n");
        d.handle_keys(&mut e, "y");
        assert_eq!(e.current_buffer().text(), "1 two one two 1");
        assert!(e.minibuffer.display().contains("Replaced 2"));
    }

    #[test]
    fn answering_with_a_bang_replaces_the_rest_without_asking() {
        let (mut d, mut e) = setup("a a a a");
        start_replace(&mut d, &mut e, "query-replace", "a", "b");
        d.handle_keys(&mut e, "!");
        assert_eq!(e.current_buffer().text(), "b b b b");
        assert!(e.minibuffer.display().contains("Replaced 4"));
        assert!(e.query_replace.is_none(), "the replacement finished");
    }

    #[test]
    fn answering_with_q_stops_without_replacing() {
        let (mut d, mut e) = setup("a a a");
        start_replace(&mut d, &mut e, "query-replace", "a", "b");
        d.handle_keys(&mut e, "y");
        d.handle_keys(&mut e, "q");
        assert_eq!(e.current_buffer().text(), "b a a");
        assert!(e.minibuffer.display().contains("Replaced 1 occurrence"));
    }

    #[test]
    fn answering_with_a_full_stop_replaces_one_and_stops() {
        let (mut d, mut e) = setup("a a a");
        start_replace(&mut d, &mut e, "query-replace", "a", "b");
        d.handle_keys(&mut e, ".");
        assert_eq!(e.current_buffer().text(), "b a a");
        assert!(e.query_replace.is_none());
    }

    #[test]
    fn an_unrecognised_answer_asks_again() {
        let (mut d, mut e) = setup("a a");
        start_replace(&mut d, &mut e, "query-replace", "a", "b");
        d.handle_keys(&mut e, "z");
        assert!(e.minibuffer.is_active());
        assert!(e.minibuffer.prompt().contains("Please answer"));
        assert_eq!(e.current_buffer().text(), "a a", "nothing was replaced");
    }

    #[test]
    fn replacing_with_a_longer_string_does_not_rematch_itself() {
        let (mut d, mut e) = setup("a a");
        start_replace(&mut d, &mut e, "query-replace", "a", "aa");
        d.handle_keys(&mut e, "!");
        assert_eq!(e.current_buffer().text(), "aa aa", "it terminated");
    }

    #[test]
    fn a_pattern_that_matches_nothing_reports_zero() {
        let (mut d, mut e) = setup("alpha");
        start_replace(&mut d, &mut e, "query-replace", "zzz", "x");
        assert!(e.minibuffer.display().contains("Replaced 0"));
    }

    #[test]
    fn query_replace_regexp_expands_capture_groups() {
        let (mut d, mut e) = setup("key = value\nother = thing\n");
        start_replace(&mut d, &mut e, "query-replace-regexp", r"(\w+) = (\w+)", r"\2: \1");
        d.handle_keys(&mut e, "!");
        assert_eq!(e.current_buffer().text(), "value: key\nthing: other\n");
    }

    #[test]
    fn an_invalid_regexp_is_reported() {
        let (mut d, mut e) = setup("text");
        d.execute(&mut e, "query-replace-regexp", None);
        e.minibuffer.kill_whole();
        for c in "(unclosed".chars() {
            e.minibuffer.insert_char(c);
        }
        let out = d.handle_keys(&mut e, "RET");
        assert!(matches!(out, Dispatch::Failed { .. }));
    }

    #[test]
    fn replacing_only_looks_forward_from_point() {
        let (mut d, mut e) = setup("a a a");
        e.with_current_buffer(|b| b.set_point(2));
        start_replace(&mut d, &mut e, "query-replace", "a", "b");
        d.handle_keys(&mut e, "!");
        assert_eq!(e.current_buffer().text(), "a b b", "the first was behind point");
    }

    // ---- occur ----

    #[test]
    fn occur_lists_every_matching_line() {
        let (mut d, mut e) = setup("alpha\nbeta\nalpha again\ngamma\n");
        d.execute(&mut e, "occur", None);
        answer(&mut d, &mut e, "alpha");

        assert_eq!(e.current_buffer().name(), OCCUR_NAME);
        assert!(e.current_buffer().is_read_only());
        let text = e.current_buffer().text();
        assert!(text.contains("2 matches for `alpha`"), "got `{text}`");
        assert!(text.contains("     1:alpha"), "got `{text}`");
        assert!(text.contains("     3:alpha again"), "got `{text}`");
        assert!(!text.contains("beta"), "got `{text}`");
    }

    #[test]
    fn occur_counts_a_line_once_however_many_times_it_matches() {
        let (mut d, mut e) = setup("a a a\nb\n");
        d.execute(&mut e, "occur", None);
        answer(&mut d, &mut e, "a");
        assert!(e.minibuffer.display().contains("1 matching line"));
    }

    #[test]
    fn occur_reuses_its_buffer_rather_than_piling_them_up() {
        let (mut d, mut e) = setup("alpha\nbeta\n");
        d.execute(&mut e, "occur", None);
        answer(&mut d, &mut e, "alpha");
        let back = e.buffers.find_by_name("test").unwrap();
        e.switch_to_buffer(back).unwrap();
        d.execute(&mut e, "occur", None);
        answer(&mut d, &mut e, "beta");
        assert_eq!(e.buffers.iter().filter(|b| b.name() == OCCUR_NAME).count(), 1);
    }

    #[test]
    fn occur_with_an_invalid_regexp_reports_it() {
        let (mut d, mut e) = setup("text");
        d.execute(&mut e, "occur", None);
        e.minibuffer.kill_whole();
        for c in "(unclosed".chars() {
            e.minibuffer.insert_char(c);
        }
        let out = d.handle_keys(&mut e, "RET");
        assert!(matches!(out, Dispatch::Failed { .. }));
    }
}
