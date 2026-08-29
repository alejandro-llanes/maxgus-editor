//! The minibuffer and echo area.
//!
//! One line at the bottom of the frame serves three purposes, as in Emacs: it
//! shows messages, it echoes a half-typed key sequence, and it hosts prompts
//! such as `M-x`, `C-x C-f` and `C-s`. Only one prompt is active at a time.

use std::collections::HashMap;

/// What the minibuffer is currently asking for. The kind selects which history
/// ring is used and how completion behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MinibufferKind {
    /// `M-x`.
    Command,
    /// `C-x C-f` and friends.
    File,
    /// `C-x b`.
    Buffer,
    /// `C-s` / `C-r`.
    Search,
    /// The replacement half of `query-replace`.
    Replace,
    /// `M-!`.
    Shell,
    /// A prompt with no completion, such as a new file name in the tree.
    Text,
    /// A prompt completing over a fixed set of answers, such as a theme name
    /// or a coding system.
    Choice,
    /// A `yes`/`no` question.
    YesNo,
    /// A single-character answer, as `query-replace` and registers use.
    Char,
}

impl MinibufferKind {
    /// True when TAB should try to complete.
    pub fn completes(self) -> bool {
        matches!(
            self,
            MinibufferKind::Command
                | MinibufferKind::File
                | MinibufferKind::Buffer
                | MinibufferKind::Choice
        )
    }

    /// True when `RET` answers with the highlighted candidate rather than
    /// with what was literally typed, given whether anything has been typed.
    ///
    /// `M-x` always does: an empty command name answers nothing, so running
    /// what is highlighted is the only useful reading of `RET` there.
    ///
    /// A buffer or a fixed set of answers does so only once something is
    /// typed, because those prompts name a default in their prompt text and
    /// an empty answer takes it.
    ///
    /// A file name never does. `C-x C-f` has to be able to name a file that
    /// is not there yet, and a new name is very often a subsequence of one
    /// that is — `notes` of `notes-2024.md`. Answering with the match would
    /// visit the old file and leave no way to create the new one. The list is
    /// still there to be chosen from with `TAB` or the arrows.
    pub fn takes_the_candidate(self, queried: bool) -> bool {
        match self {
            MinibufferKind::Command => true,
            MinibufferKind::Buffer | MinibufferKind::Choice => queried,
            _ => false,
        }
    }

    /// True when the prompt takes one key rather than a line of text.
    pub fn is_single_key(self) -> bool {
        matches!(self, MinibufferKind::Char)
    }

    /// True when the input should be remembered in a history ring.
    pub fn has_history(self) -> bool {
        !matches!(self, MinibufferKind::YesNo | MinibufferKind::Char)
    }
}

/// Candidate completions for what has been typed so far.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Completion {
    pub candidates: Vec<String>,
    /// How many candidates the prompt is completing over, before filtering.
    /// The popup shows `matching/total`, which is how the user can tell a
    /// query that narrowed things down from one that matched nothing.
    pub total: usize,
    /// Index into `candidates` when the user is cycling through them.
    pub selected: Option<usize>,
    /// True once the candidate list should be shown.
    pub visible: bool,
    /// True once the user has moved the highlight themselves, which is what
    /// makes `RET` take the highlighted row rather than the prompt's default.
    pub moved: bool,
    /// True when TAB is what put the list up, which is when TAB starts
    /// cycling through it instead of completing.
    ///
    /// A prompt that lists its options from the moment it opens is visible
    /// without being cyclable: the first TAB must still complete to the
    /// common prefix, as it always has.
    pub cycling: bool,
}

impl Completion {
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }

    pub fn len(&self) -> usize {
        self.candidates.len()
    }

    /// The candidate currently selected while cycling.
    pub fn current(&self) -> Option<&str> {
        self.selected.and_then(|i| self.candidates.get(i)).map(String::as_str)
    }

    fn clear(&mut self) {
        self.candidates.clear();
        self.total = 0;
        self.selected = None;
        self.visible = false;
        self.cycling = false;
        self.moved = false;
    }
}

/// The state of the minibuffer line.
#[derive(Debug, Clone, Default)]
pub struct Minibuffer {
    active: Option<MinibufferKind>,
    prompt: String,
    input: String,
    /// Point within `input`, in characters.
    point: usize,
    completion: Completion,
    /// The message shown when no prompt is active.
    message: Option<String>,
    /// True when the message is an error, so it is drawn in the error face.
    message_is_error: bool,
    histories: HashMap<MinibufferKind, Vec<String>>,
    /// Position in the history ring while walking it; `None` means the user is
    /// editing fresh input.
    history_index: Option<usize>,
    /// The input as it was before history walking began, so `M-n` past the
    /// newest entry restores it.
    saved_input: String,
}

/// The maximum entries kept per history ring.
const HISTORY_MAX: usize = 100;

impl Minibuffer {
    pub fn new() -> Minibuffer {
        Minibuffer::default()
    }

    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    pub fn kind(&self) -> Option<MinibufferKind> {
        self.active
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn point(&self) -> usize {
        self.point
    }

    pub fn completion(&self) -> &Completion {
        &self.completion
    }

    /// The message shown when no prompt is active.
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn message_is_error(&self) -> bool {
        self.message_is_error
    }

    /// The whole line as displayed: prompt followed by input, or the message.
    pub fn display(&self) -> String {
        if self.is_active() {
            format!("{}{}", self.prompt, self.input)
        } else {
            self.message.clone().unwrap_or_default()
        }
    }

    /// The column point sits at on the displayed line.
    pub fn cursor_column(&self) -> usize {
        self.prompt.chars().count() + self.point
    }

    // ---- messages ------------------------------------------------------

    /// `message`: shows text in the echo area.
    pub fn show_message(&mut self, text: impl Into<String>) {
        self.message = Some(text.into());
        self.message_is_error = false;
    }

    /// Shows an error, which is drawn in the error face.
    pub fn show_error(&mut self, text: impl Into<String>) {
        self.message = Some(text.into());
        self.message_is_error = true;
    }

    pub fn clear_message(&mut self) {
        self.message = None;
        self.message_is_error = false;
    }

    // ---- prompting -----------------------------------------------------

    /// Opens a prompt. Any message on screen is cleared.
    pub fn activate(&mut self, kind: MinibufferKind, prompt: impl Into<String>) {
        self.activate_with(kind, prompt, "");
    }

    /// Opens a prompt pre-filled with `initial`, with point at its end — how
    /// `C-x C-f` offers the current directory.
    pub fn activate_with(
        &mut self,
        kind: MinibufferKind,
        prompt: impl Into<String>,
        initial: &str,
    ) {
        self.active = Some(kind);
        self.prompt = prompt.into();
        self.input = initial.to_string();
        self.point = self.input.chars().count();
        self.completion.clear();
        self.history_index = None;
        self.saved_input.clear();
        self.clear_message();
    }

    /// `C-g`: abandons the prompt without recording history.
    pub fn abort(&mut self) {
        self.active = None;
        self.input.clear();
        self.point = 0;
        self.completion.clear();
        self.history_index = None;
        self.show_message("Quit");
    }

    /// `RET`: closes the prompt and returns what was typed, recording it in the
    /// history ring.
    pub fn accept(&mut self) -> Option<String> {
        // What the list is pointing at, which with fuzzy matching is a real
        // candidate where the typed query usually is not. Read before the
        // prompt is closed, because which of the two it is depends on what
        // kind of prompt this is.
        let text = self.chosen();
        let kind = self.active.take()?;
        self.input.clear();
        self.point = 0;
        self.completion.clear();
        self.history_index = None;
        if kind.has_history() && !text.is_empty() {
            let ring = self.histories.entry(kind).or_default();
            // Repeating an entry moves it to the front rather than duplicating.
            ring.retain(|e| e != &text);
            ring.insert(0, text.clone());
            ring.truncate(HISTORY_MAX);
        }
        Some(text)
    }

    // ---- editing -------------------------------------------------------

    fn byte_index(&self, char_index: usize) -> usize {
        self.input
            .char_indices()
            .nth(char_index)
            .map_or(self.input.len(), |(byte, _)| byte)
    }

    fn len_chars(&self) -> usize {
        self.input.chars().count()
    }

    /// Inserts text at point. Editing invalidates any completion in progress.
    pub fn insert(&mut self, text: &str) {
        if !self.is_active() {
            return;
        }
        let at = self.byte_index(self.point);
        self.input.insert_str(at, text);
        self.point += text.chars().count();
        self.completion.clear();
        self.history_index = None;
    }

    pub fn insert_char(&mut self, c: char) {
        self.insert(&c.to_string());
    }

    /// `DEL`.
    pub fn delete_backward(&mut self) -> bool {
        if !self.is_active() || self.point == 0 {
            return false;
        }
        let at = self.byte_index(self.point - 1);
        self.input.remove(at);
        self.point -= 1;
        self.completion.clear();
        true
    }

    /// `C-d`.
    pub fn delete_forward(&mut self) -> bool {
        if !self.is_active() || self.point >= self.len_chars() {
            return false;
        }
        let at = self.byte_index(self.point);
        self.input.remove(at);
        self.completion.clear();
        true
    }

    /// `C-k`.
    pub fn kill_to_end(&mut self) -> String {
        let at = self.byte_index(self.point);
        let killed = self.input.split_off(at);
        self.completion.clear();
        killed
    }

    /// `C-u` in the minibuffer, and what `C-a C-k` amounts to.
    pub fn kill_whole(&mut self) -> String {
        self.point = 0;
        let killed = std::mem::take(&mut self.input);
        self.completion.clear();
        killed
    }

    /// `M-DEL`: deletes the word before point.
    pub fn delete_word_backward(&mut self) -> bool {
        if self.point == 0 {
            return false;
        }
        let chars: Vec<char> = self.input.chars().collect();
        let mut start = self.point;
        while start > 0 && !chars[start - 1].is_alphanumeric() {
            start -= 1;
        }
        while start > 0 && chars[start - 1].is_alphanumeric() {
            start -= 1;
        }
        let from = self.byte_index(start);
        let to = self.byte_index(self.point);
        self.input.replace_range(from..to, "");
        self.point = start;
        self.completion.clear();
        true
    }

    pub fn move_start(&mut self) {
        self.point = 0;
    }

    pub fn move_end(&mut self) {
        self.point = self.len_chars();
    }

    pub fn move_left(&mut self) {
        self.point = self.point.saturating_sub(1);
    }

    pub fn move_right(&mut self) {
        self.point = (self.point + 1).min(self.len_chars());
    }

    // ---- completion ----------------------------------------------------

    /// TAB: completes as far as the candidates agree.
    ///
    /// Returns true when the input grew. When it did not, the candidate list
    /// becomes visible instead — Emacs' behaviour of showing `*Completions*`
    /// only once completion has stopped making progress.
    pub fn complete(&mut self, candidates: &[String]) -> bool {
        let matching: Vec<String> =
            candidates.iter().filter(|c| c.starts_with(&self.input)).cloned().collect();

        if matching.is_empty() {
            self.completion.clear();
            return false;
        }
        let common = longest_common_prefix(&matching);
        let grew = common.chars().count() > self.len_chars();
        if grew {
            self.input = common;
            self.point = self.len_chars();
        }
        let exact_single = matching.len() == 1 && matching[0] == self.input;
        let settled = !grew && !exact_single;
        self.completion = Completion {
            total: candidates.len(),
            visible: settled,
            cycling: settled,
            candidates: matching,
            selected: None,
            moved: false,
        };
        grew
    }

    /// Shows the candidates matching what has been typed, without touching the
    /// input.
    ///
    /// This is what makes `M-x` and `C-x b` list what is on offer the moment
    /// they open and narrow as the user types, rather than staying blank until
    /// TAB. Unlike [`Minibuffer::complete`] it never grows the input, and it
    /// leaves the list un-cyclable so the first TAB still completes.
    pub fn filter_completions(&mut self, candidates: &[String]) {
        if !self.active.is_some_and(MinibufferKind::completes) {
            return;
        }
        let moved = self.completion.moved;
        let matching = crate::fuzzy::matches(&self.input, candidates.iter());
        self.completion = Completion {
            moved,
            total: candidates.len(),
            // The best match is selected from the start, so `RET` takes what
            // the list is pointing at rather than the letters typed so far —
            // which with fuzzy matching are usually not a command name.
            selected: (!matching.is_empty()).then_some(0),
            visible: !matching.is_empty(),
            cycling: false,
            candidates: matching,
        };
    }

    /// Moves the highlight through the candidate list by `delta` rows.
    ///
    /// The input is left exactly as typed: with fuzzy matching it is a query
    /// rather than a half-finished name, and replacing it — which is what
    /// `cycle_completion` does for TAB — would throw the query away.
    pub fn move_selection(&mut self, delta: isize) -> bool {
        if self.completion.is_empty() {
            return false;
        }
        self.completion.moved = true;
        let len = self.completion.len() as isize;
        let from = self.completion.selected.unwrap_or(0) as isize;
        // Wrapping, so holding a key at either end walks round rather than
        // stopping without saying why.
        self.completion.selected = Some(((from + delta).rem_euclid(len)) as usize);
        self.completion.visible = true;
        true
    }

    /// What `RET` should accept: the highlighted candidate, or the raw input
    /// when nothing is highlighted.
    pub fn chosen(&self) -> String {
        // Moving the highlight always points at the list. Short of that, a
        // query points at it only for the prompts that answer with one of
        // their candidates, and an untouched prompt answers with nothing at
        // all, which is how a command that offers a default gets to use it.
        let queried = !self.input.is_empty();
        let points_at_it =
            self.active.is_some_and(|kind| kind.takes_the_candidate(queried));
        if !self.completion.moved && !points_at_it {
            return self.input.clone();
        }
        self.completion
            .current()
            .map(str::to_string)
            .unwrap_or_else(|| self.input.clone())
    }

    /// Cycles forward through the candidate list, replacing the input with the
    /// selected candidate.
    pub fn cycle_completion(&mut self, forward: bool) -> bool {
        if self.completion.is_empty() {
            return false;
        }
        let len = self.completion.len();
        let next = match (self.completion.selected, forward) {
            (None, true) => 0,
            (None, false) => len - 1,
            (Some(i), true) => (i + 1) % len,
            (Some(i), false) => (i + len - 1) % len,
        };
        self.completion.selected = Some(next);
        self.completion.visible = true;
        self.input = self.completion.candidates[next].clone();
        self.point = self.len_chars();
        true
    }

    /// True when the input exactly matches one candidate and no other.
    pub fn is_unique_match(&self, candidates: &[String]) -> bool {
        candidates.iter().filter(|c| c.starts_with(&self.input)).count() == 1
    }

    // ---- history -------------------------------------------------------

    /// The history ring for `kind`, newest first.
    pub fn history(&self, kind: MinibufferKind) -> &[String] {
        self.histories.get(&kind).map_or(&[], Vec::as_slice)
    }

    /// `M-p`: walks back through history.
    pub fn history_previous(&mut self) -> bool {
        let Some(kind) = self.active else { return false };
        let len = self.histories.get(&kind).map_or(0, Vec::len);
        if len == 0 {
            return false;
        }
        let next = match self.history_index {
            None => {
                // Remember what was typed so `M-n` can come back to it.
                self.saved_input = self.input.clone();
                0
            }
            Some(i) if i + 1 < len => i + 1,
            Some(_) => return false,
        };
        self.history_index = Some(next);
        self.input = self.histories[&kind][next].clone();
        self.point = self.len_chars();
        true
    }

    /// `M-n`: walks forward, restoring the original input past the newest
    /// entry.
    pub fn history_next(&mut self) -> bool {
        let Some(kind) = self.active else { return false };
        let Some(index) = self.history_index else { return false };
        if index == 0 {
            self.history_index = None;
            self.input = std::mem::take(&mut self.saved_input);
            self.point = self.len_chars();
            return true;
        }
        let next = index - 1;
        self.history_index = Some(next);
        self.input = self.histories[&kind][next].clone();
        self.point = self.len_chars();
        true
    }
}

/// The longest prefix every string in `items` shares.
fn longest_common_prefix(items: &[String]) -> String {
    let Some(first) = items.first() else { return String::new() };
    let mut prefix: Vec<char> = first.chars().collect();
    for item in &items[1..] {
        let shared = prefix
            .iter()
            .zip(item.chars())
            .take_while(|(a, b)| **a == *b)
            .count();
        prefix.truncate(shared);
        if prefix.is_empty() {
            break;
        }
    }
    prefix.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn prompting() -> Minibuffer {
        let mut m = Minibuffer::new();
        m.activate(MinibufferKind::Command, "M-x ");
        m
    }

    #[test]
    fn a_fresh_minibuffer_is_inactive_and_silent() {
        let m = Minibuffer::new();
        assert!(!m.is_active());
        assert_eq!(m.message(), None);
        assert_eq!(m.display(), "");
    }

    #[test]
    fn messages_show_when_no_prompt_is_active() {
        let mut m = Minibuffer::new();
        m.show_message("Wrote file");
        assert_eq!(m.display(), "Wrote file");
        assert!(!m.message_is_error());
        m.show_error("No such file");
        assert!(m.message_is_error());
        m.clear_message();
        assert_eq!(m.display(), "");
    }

    #[test]
    fn activating_a_prompt_clears_the_message() {
        let mut m = Minibuffer::new();
        m.show_message("stale");
        m.activate(MinibufferKind::Command, "M-x ");
        assert!(m.is_active());
        assert_eq!(m.message(), None);
        assert_eq!(m.display(), "M-x ");
    }

    #[test]
    fn a_prompt_can_start_pre_filled_with_point_at_the_end() {
        let mut m = Minibuffer::new();
        m.activate_with(MinibufferKind::File, "Find file: ", "/home/user/");
        assert_eq!(m.input(), "/home/user/");
        assert_eq!(m.point(), 11);
        assert_eq!(m.display(), "Find file: /home/user/");
        assert_eq!(m.cursor_column(), 22);
    }

    #[test]
    fn typing_inserts_at_point() {
        let mut m = prompting();
        m.insert("find-file");
        assert_eq!(m.input(), "find-file");
        m.move_start();
        m.insert_char('X');
        assert_eq!(m.input(), "Xfind-file");
        assert_eq!(m.point(), 1);
    }

    #[test]
    fn typing_into_an_inactive_minibuffer_does_nothing() {
        let mut m = Minibuffer::new();
        m.insert("ignored");
        assert_eq!(m.input(), "");
    }

    #[test]
    fn deletion_works_in_both_directions_and_stops_at_the_edges() {
        let mut m = prompting();
        m.insert("abc");
        assert!(m.delete_backward());
        assert_eq!(m.input(), "ab");
        m.move_start();
        assert!(!m.delete_backward(), "nothing before point");
        assert!(m.delete_forward());
        assert_eq!(m.input(), "b");
        m.move_end();
        assert!(!m.delete_forward(), "nothing after point");
    }

    #[test]
    fn killing_to_the_end_returns_what_it_removed() {
        let mut m = prompting();
        m.insert("hello world");
        m.move_start();
        m.move_right();
        m.move_right();
        m.move_right();
        m.move_right();
        m.move_right();
        assert_eq!(m.kill_to_end(), " world");
        assert_eq!(m.input(), "hello");
    }

    #[test]
    fn killing_the_whole_line_empties_it() {
        let mut m = prompting();
        m.insert("discard me");
        assert_eq!(m.kill_whole(), "discard me");
        assert_eq!(m.input(), "");
        assert_eq!(m.point(), 0);
    }

    #[test]
    fn deleting_a_word_backward_stops_at_the_separator() {
        let mut m = prompting();
        m.insert("/home/user/file.rs");
        assert!(m.delete_word_backward());
        assert_eq!(m.input(), "/home/user/file.");
        assert!(m.delete_word_backward());
        assert_eq!(m.input(), "/home/user/");
        assert!(m.delete_word_backward());
        assert_eq!(m.input(), "/home/");
    }

    #[test]
    fn point_motion_clamps_at_both_ends() {
        let mut m = prompting();
        m.insert("abc");
        m.move_start();
        m.move_left();
        assert_eq!(m.point(), 0);
        m.move_end();
        m.move_right();
        assert_eq!(m.point(), 3);
    }

    #[test]
    fn editing_handles_multibyte_input() {
        let mut m = prompting();
        m.insert("héllo");
        assert_eq!(m.point(), 5);
        m.delete_backward();
        assert_eq!(m.input(), "héll");
        m.move_start();
        m.move_right();
        m.delete_forward();
        assert_eq!(m.input(), "hll", "the accented character was one deletion");
    }

    #[test]
    fn accepting_returns_the_input_and_closes_the_prompt() {
        let mut m = prompting();
        m.insert("save-buffer");
        assert_eq!(m.accept(), Some("save-buffer".into()));
        assert!(!m.is_active());
        assert_eq!(m.input(), "");
    }

    #[test]
    fn accepting_an_inactive_minibuffer_yields_nothing() {
        let mut m = Minibuffer::new();
        assert_eq!(m.accept(), None);
    }

    #[test]
    fn aborting_discards_the_input_and_says_quit() {
        let mut m = prompting();
        m.insert("half-typed");
        m.abort();
        assert!(!m.is_active());
        assert_eq!(m.message(), Some("Quit"));
        assert!(m.history(MinibufferKind::Command).is_empty(), "aborts are not recorded");
    }

    #[test]
    fn accepted_input_goes_into_the_history_ring_newest_first() {
        let mut m = Minibuffer::new();
        for command in ["first", "second", "third"] {
            m.activate(MinibufferKind::Command, "M-x ");
            m.insert(command);
            m.accept();
        }
        assert_eq!(m.history(MinibufferKind::Command), ["third", "second", "first"]);
    }

    #[test]
    fn repeating_an_entry_moves_it_to_the_front_rather_than_duplicating() {
        let mut m = Minibuffer::new();
        for command in ["a", "b", "a"] {
            m.activate(MinibufferKind::Command, "M-x ");
            m.insert(command);
            m.accept();
        }
        assert_eq!(m.history(MinibufferKind::Command), ["a", "b"]);
    }

    #[test]
    fn empty_input_is_not_recorded() {
        let mut m = prompting();
        m.accept();
        assert!(m.history(MinibufferKind::Command).is_empty());
    }

    #[test]
    fn history_rings_are_kept_separate_by_kind() {
        let mut m = Minibuffer::new();
        m.activate(MinibufferKind::Command, "M-x ");
        m.insert("a-command");
        m.accept();
        m.activate(MinibufferKind::File, "Find file: ");
        m.insert("/a/file");
        m.accept();
        assert_eq!(m.history(MinibufferKind::Command), ["a-command"]);
        assert_eq!(m.history(MinibufferKind::File), ["/a/file"]);
    }

    #[test]
    fn yes_no_answers_are_not_remembered() {
        let mut m = Minibuffer::new();
        m.activate(MinibufferKind::YesNo, "Really? ");
        m.insert("yes");
        m.accept();
        assert!(m.history(MinibufferKind::YesNo).is_empty());
        assert!(!MinibufferKind::YesNo.has_history());
    }

    #[test]
    fn history_walks_backwards_and_forwards() {
        let mut m = Minibuffer::new();
        for command in ["one", "two"] {
            m.activate(MinibufferKind::Command, "M-x ");
            m.insert(command);
            m.accept();
        }
        m.activate(MinibufferKind::Command, "M-x ");
        assert!(m.history_previous());
        assert_eq!(m.input(), "two");
        assert!(m.history_previous());
        assert_eq!(m.input(), "one");
        assert!(!m.history_previous(), "the ring ends");
        assert!(m.history_next());
        assert_eq!(m.input(), "two");
    }

    #[test]
    fn walking_past_the_newest_entry_restores_what_was_typed() {
        let mut m = Minibuffer::new();
        m.activate(MinibufferKind::Command, "M-x ");
        m.insert("remembered");
        m.accept();
        m.activate(MinibufferKind::Command, "M-x ");
        m.insert("half-typed");
        m.history_previous();
        assert_eq!(m.input(), "remembered");
        assert!(m.history_next());
        assert_eq!(m.input(), "half-typed");
        assert!(!m.history_next(), "already back at the input");
    }

    #[test]
    fn history_on_an_empty_ring_does_nothing() {
        let mut m = prompting();
        assert!(!m.history_previous());
        assert!(!m.history_next());
    }

    #[test]
    fn typing_after_walking_history_leaves_the_ring() {
        let mut m = Minibuffer::new();
        m.activate(MinibufferKind::Command, "M-x ");
        m.insert("entry");
        m.accept();
        m.activate(MinibufferKind::Command, "M-x ");
        m.history_previous();
        m.insert("!");
        assert!(!m.history_next(), "editing detached from the ring");
        assert_eq!(m.input(), "entry!");
    }

    // ---- completion ----

    #[test]
    fn completion_extends_to_the_common_prefix() {
        let mut m = prompting();
        m.insert("save");
        assert!(m.complete(&candidates(&["save-buffer", "save-some-buffers", "switch-to-buffer"])));
        assert_eq!(m.input(), "save-", "as far as the two matches agree");
        assert_eq!(m.point(), 5);
        assert_eq!(m.completion().len(), 2);
    }

    #[test]
    fn a_unique_candidate_completes_in_full() {
        let mut m = prompting();
        m.insert("sav");
        assert!(m.complete(&candidates(&["save-buffer", "find-file"])));
        assert_eq!(m.input(), "save-buffer");
        assert!(m.is_unique_match(&candidates(&["save-buffer", "find-file"])));
    }

    #[test]
    fn a_second_completion_that_cannot_grow_shows_the_candidates() {
        let mut m = prompting();
        m.insert("save-");
        let all = candidates(&["save-buffer", "save-some-buffers"]);
        assert!(!m.complete(&all), "already at the common prefix");
        assert!(m.completion().visible, "the list is offered instead");
        assert_eq!(m.completion().len(), 2);
    }

    #[test]
    fn completing_an_exact_unique_match_does_not_pop_up_a_list() {
        let mut m = prompting();
        m.insert("save-buffer");
        assert!(!m.complete(&candidates(&["save-buffer", "find-file"])));
        assert!(!m.completion().visible);
    }

    #[test]
    fn no_matching_candidate_leaves_the_input_alone() {
        let mut m = prompting();
        m.insert("zzz");
        assert!(!m.complete(&candidates(&["save-buffer"])));
        assert_eq!(m.input(), "zzz");
        assert!(m.completion().is_empty());
    }

    #[test]
    fn completion_cycles_through_the_candidates_and_wraps() {
        let mut m = prompting();
        m.insert("save-");
        let all = candidates(&["save-buffer", "save-some-buffers"]);
        m.complete(&all);
        assert!(m.cycle_completion(true));
        assert_eq!(m.input(), "save-buffer");
        m.cycle_completion(true);
        assert_eq!(m.input(), "save-some-buffers");
        m.cycle_completion(true);
        assert_eq!(m.input(), "save-buffer", "wraps around");
        m.cycle_completion(false);
        assert_eq!(m.input(), "save-some-buffers", "and backwards");
    }

    #[test]
    fn cycling_with_no_candidates_does_nothing() {
        let mut m = prompting();
        assert!(!m.cycle_completion(true));
    }

    #[test]
    fn editing_discards_the_completion_state() {
        let mut m = prompting();
        m.insert("save-");
        m.complete(&candidates(&["save-buffer", "save-some-buffers"]));
        assert!(!m.completion().is_empty());
        m.insert("b");
        assert!(m.completion().is_empty());
    }

    #[test]
    fn the_longest_common_prefix_handles_the_edge_cases() {
        assert_eq!(longest_common_prefix(&[]), "");
        assert_eq!(longest_common_prefix(&candidates(&["only"])), "only");
        assert_eq!(longest_common_prefix(&candidates(&["abc", "abd"])), "ab");
        assert_eq!(longest_common_prefix(&candidates(&["abc", "xyz"])), "");
        assert_eq!(longest_common_prefix(&candidates(&["ab", "abc"])), "ab");
        assert_eq!(longest_common_prefix(&candidates(&["héllo", "hérisson"])), "hé");
    }

    #[test]
    fn each_kind_declares_how_it_behaves() {
        assert!(MinibufferKind::Command.completes());
        assert!(MinibufferKind::File.completes());
        assert!(!MinibufferKind::Search.completes());
        assert!(MinibufferKind::Char.is_single_key());
        assert!(!MinibufferKind::Text.is_single_key());
    }
}
