//! Cursor motions.
//!
//! These are pure functions over a rope: they take an offset and return one.
//! Buffer-level commands in `maxgus-core` layer point handling, prefix arguments
//! and undo on top of them.

use ropey::Rope;

/// Emacs' syntax classes, reduced to what the console editor needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharClass {
    Whitespace,
    /// Constituents of a word: `forward-word` stops at their boundaries.
    Word,
    /// Symbol constituents such as `_` and `-` that join words in identifiers.
    Symbol,
    Open,
    Close,
    Quote,
    Punctuation,
}

impl CharClass {
    pub fn of(c: char) -> CharClass {
        match c {
            '(' | '[' | '{' => CharClass::Open,
            ')' | ']' | '}' => CharClass::Close,
            '"' | '\'' | '`' => CharClass::Quote,
            '_' | '-' => CharClass::Symbol,
            c if c.is_whitespace() => CharClass::Whitespace,
            c if c.is_alphanumeric() => CharClass::Word,
            _ => CharClass::Punctuation,
        }
    }

    /// True for characters `forward-word` treats as part of a word.
    pub fn is_word(self) -> bool {
        matches!(self, CharClass::Word)
    }

    /// True for characters that may appear inside a symbol, which is what
    /// `forward-sexp` scans over when not starting on a delimiter.
    pub fn is_symbol(self) -> bool {
        matches!(self, CharClass::Word | CharClass::Symbol)
    }
}

/// The matching closer for an opening delimiter.
fn closer_for(open: char) -> Option<char> {
    match open {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        _ => None,
    }
}

/// The matching opener for a closing delimiter.
fn opener_for(close: char) -> Option<char> {
    match close {
        ')' => Some('('),
        ']' => Some('['),
        '}' => Some('{'),
        _ => None,
    }
}

/// Namespace for motion functions over a rope.
pub struct Motion;

impl Motion {
    fn char_at(rope: &Rope, at: usize) -> Option<char> {
        (at < rope.len_chars()).then(|| rope.char(at))
    }

    /// `forward-char`, clamped to the end of the buffer.
    pub fn forward_char(rope: &Rope, at: usize, n: usize) -> usize {
        (at + n).min(rope.len_chars())
    }

    /// `backward-char`, clamped to the start of the buffer.
    pub fn backward_char(_rope: &Rope, at: usize, n: usize) -> usize {
        at.saturating_sub(n)
    }

    /// `move-beginning-of-line`.
    pub fn line_start(rope: &Rope, at: usize) -> usize {
        let line = rope.char_to_line(at.min(rope.len_chars()));
        rope.line_to_char(line)
    }

    /// `move-end-of-line`: the offset of the newline, or of the buffer end on
    /// the final line.
    pub fn line_end(rope: &Rope, at: usize) -> usize {
        let at = at.min(rope.len_chars());
        let line = rope.char_to_line(at);
        let start = rope.line_to_char(line);
        let slice = rope.line(line);
        let mut len = slice.len_chars();
        // Strip the line terminator so point lands before it.
        if len > 0 && slice.char(len - 1) == '\n' {
            len -= 1;
            if len > 0 && slice.char(len - 1) == '\r' {
                len -= 1;
            }
        }
        start + len
    }

    /// `back-to-indentation`: first non-whitespace character of the line.
    pub fn back_to_indentation(rope: &Rope, at: usize) -> usize {
        let start = Self::line_start(rope, at);
        let end = Self::line_end(rope, at);
        let mut i = start;
        while i < end && rope.char(i).is_whitespace() {
            i += 1;
        }
        i
    }

    /// `forward-word` applied `n` times: skips non-word characters, then
    /// consumes the word.
    pub fn forward_word(rope: &Rope, mut at: usize, n: usize) -> usize {
        let len = rope.len_chars();
        for _ in 0..n {
            while at < len && !CharClass::of(rope.char(at)).is_word() {
                at += 1;
            }
            while at < len && CharClass::of(rope.char(at)).is_word() {
                at += 1;
            }
        }
        at
    }

    /// `backward-word` applied `n` times.
    pub fn backward_word(rope: &Rope, mut at: usize, n: usize) -> usize {
        for _ in 0..n {
            while at > 0 && !CharClass::of(rope.char(at - 1)).is_word() {
                at -= 1;
            }
            while at > 0 && CharClass::of(rope.char(at - 1)).is_word() {
                at -= 1;
            }
        }
        at
    }

    /// The bounds of the word around `at`, or `None` when point is not on one.
    pub fn word_bounds(rope: &Rope, at: usize) -> Option<(usize, usize)> {
        let len = rope.len_chars();
        let on_word = |i: usize| i < len && CharClass::of(rope.char(i)).is_word();
        // Emacs' `thing-at-point` also accepts point sitting just after a word.
        let anchor = if on_word(at) {
            at
        } else if at > 0 && on_word(at - 1) {
            at - 1
        } else {
            return None;
        };
        let mut start = anchor;
        while start > 0 && CharClass::of(rope.char(start - 1)).is_word() {
            start -= 1;
        }
        let mut end = anchor;
        while end < len && CharClass::of(rope.char(end)).is_word() {
            end += 1;
        }
        Some((start, end))
    }

    /// True when the line containing `at` holds only whitespace.
    fn line_is_blank(rope: &Rope, line: usize) -> bool {
        rope.line(line).chars().all(char::is_whitespace)
    }

    /// `forward-paragraph`: moves to the start of the next blank line that
    /// separates paragraphs, or to the end of the buffer.
    pub fn forward_paragraph(rope: &Rope, at: usize, n: usize) -> usize {
        let last_line = rope.len_lines().saturating_sub(1);
        let mut line = rope.char_to_line(at.min(rope.len_chars()));
        for _ in 0..n {
            // Skip the blank run we may currently be sitting in.
            while line < last_line && Self::line_is_blank(rope, line) {
                line += 1;
            }
            while line < last_line && !Self::line_is_blank(rope, line) {
                line += 1;
            }
            if line >= last_line {
                return rope.len_chars();
            }
        }
        rope.line_to_char(line)
    }

    /// `backward-paragraph`.
    pub fn backward_paragraph(rope: &Rope, at: usize, n: usize) -> usize {
        let mut line = rope.char_to_line(at.min(rope.len_chars()));
        for _ in 0..n {
            while line > 0 && Self::line_is_blank(rope, line.saturating_sub(1)) {
                line -= 1;
            }
            while line > 0 && !Self::line_is_blank(rope, line.saturating_sub(1)) {
                line -= 1;
            }
            if line == 0 {
                return 0;
            }
            // Emacs parks point on the blank line that precedes the paragraph.
            line -= 1;
        }
        rope.line_to_char(line)
    }

    /// `forward-sentence`: stops after `.`, `?` or `!`, or at a paragraph end.
    pub fn forward_sentence(rope: &Rope, mut at: usize, n: usize) -> usize {
        let len = rope.len_chars();
        for _ in 0..n {
            let mut i = at;
            while i < len {
                let c = rope.char(i);
                i += 1;
                if matches!(c, '.' | '?' | '!') {
                    // Consume the closing quotes and whitespace that follow.
                    while i < len && matches!(rope.char(i), '"' | '\'' | ')' | ']') {
                        i += 1;
                    }
                    break;
                }
            }
            at = i.min(len);
        }
        at
    }

    /// `backward-sentence`.
    pub fn backward_sentence(rope: &Rope, mut at: usize, n: usize) -> usize {
        for _ in 0..n {
            let from = at;
            let mut i = at;
            // Step off any terminator we are already sitting on.
            while i > 0
                && (rope.char(i - 1).is_whitespace()
                    || matches!(rope.char(i - 1), '"' | '\'' | ')' | ']'))
            {
                i -= 1;
            }
            while i > 0 && matches!(rope.char(i - 1), '.' | '?' | '!') {
                i -= 1;
            }
            while i > 0 && !matches!(rope.char(i - 1), '.' | '?' | '!') {
                i -= 1;
            }
            // Leave point after the whitespace separating the sentences.
            let len = rope.len_chars();
            let mut after_gap = i;
            while after_gap < len && rope.char(after_gap).is_whitespace() {
                after_gap += 1;
            }
            // Starting *inside* that whitespace, the scan back reaches the
            // buffer start and stepping over it again would carry point
            // forward to where it began — a backward motion must never do
            // that, and it made a backwards range out of
            // `backward-kill-sentence`. Keep the skip only when it still ends
            // up behind where we started.
            at = if after_gap < from { after_gap } else { i };
        }
        at
    }

    /// Scans forward over one balanced expression, as `forward-sexp` does.
    /// Returns `None` when the delimiters are unbalanced.
    pub fn forward_sexp(rope: &Rope, at: usize, n: usize) -> Option<usize> {
        let len = rope.len_chars();
        let mut i = at;
        for _ in 0..n {
            while i < len && CharClass::of(rope.char(i)) == CharClass::Whitespace {
                i += 1;
            }
            if i >= len {
                return Some(len);
            }
            let c = rope.char(i);
            match CharClass::of(c) {
                CharClass::Open => i = Self::matching_forward(rope, i)? + 1,
                CharClass::Quote => i = Self::skip_string_forward(rope, i)?,
                CharClass::Close => return None,
                _ => {
                    while i < len && CharClass::of(rope.char(i)).is_symbol() {
                        i += 1;
                    }
                    // A lone punctuation character still counts as progress.
                    if i == at {
                        i += 1;
                    }
                }
            }
        }
        Some(i)
    }

    /// Scans backward over one balanced expression, as `backward-sexp` does.
    pub fn backward_sexp(rope: &Rope, at: usize, n: usize) -> Option<usize> {
        let mut i = at;
        for _ in 0..n {
            while i > 0 && CharClass::of(rope.char(i - 1)) == CharClass::Whitespace {
                i -= 1;
            }
            if i == 0 {
                return Some(0);
            }
            let c = rope.char(i - 1);
            match CharClass::of(c) {
                CharClass::Close => i = Self::matching_backward(rope, i - 1)?,
                CharClass::Quote => i = Self::skip_string_backward(rope, i - 1)?,
                CharClass::Open => return None,
                _ => {
                    let before = i;
                    while i > 0 && CharClass::of(rope.char(i - 1)).is_symbol() {
                        i -= 1;
                    }
                    if i == before {
                        i -= 1;
                    }
                }
            }
        }
        Some(i)
    }

    /// Given the offset of an opening delimiter, finds its partner.
    pub fn matching_forward(rope: &Rope, open_at: usize) -> Option<usize> {
        let open = Self::char_at(rope, open_at)?;
        let close = closer_for(open)?;
        let len = rope.len_chars();
        let mut depth = 0usize;
        let mut i = open_at;
        while i < len {
            let c = rope.char(i);
            if c == open {
                depth += 1;
            } else if c == close {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            i += 1;
        }
        None
    }

    /// Given the offset of a closing delimiter, finds its partner.
    pub fn matching_backward(rope: &Rope, close_at: usize) -> Option<usize> {
        let close = Self::char_at(rope, close_at)?;
        let open = opener_for(close)?;
        let mut depth = 0usize;
        let mut i = close_at + 1;
        while i > 0 {
            i -= 1;
            let c = rope.char(i);
            if c == close {
                depth += 1;
            } else if c == open {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
        }
        None
    }

    /// The partner of whichever delimiter sits at `at`, in either direction.
    pub fn matching_delimiter(rope: &Rope, at: usize) -> Option<usize> {
        let c = Self::char_at(rope, at)?;
        match CharClass::of(c) {
            CharClass::Open => Self::matching_forward(rope, at),
            CharClass::Close => Self::matching_backward(rope, at),
            _ => None,
        }
    }

    fn skip_string_forward(rope: &Rope, quote_at: usize) -> Option<usize> {
        let quote = rope.char(quote_at);
        let len = rope.len_chars();
        let mut i = quote_at + 1;
        while i < len {
            let c = rope.char(i);
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == quote {
                return Some(i + 1);
            }
            i += 1;
        }
        None
    }

    fn skip_string_backward(rope: &Rope, quote_at: usize) -> Option<usize> {
        let quote = rope.char(quote_at);
        let mut i = quote_at;
        while i > 0 {
            i -= 1;
            if rope.char(i) == quote && (i == 0 || rope.char(i - 1) != '\\') {
                return Some(i);
            }
        }
        None
    }

    /// `beginning-of-defun`: the previous line that starts in column zero with
    /// a non-whitespace, non-closing character.
    pub fn beginning_of_defun(rope: &Rope, at: usize) -> usize {
        let mut line = rope.char_to_line(at.min(rope.len_chars()));
        while line > 0 {
            line -= 1;
            let start = rope.line_to_char(line);
            if let Some(c) = Self::char_at(rope, start)
                && !c.is_whitespace()
                && CharClass::of(c) != CharClass::Close
            {
                return start;
            }
        }
        0
    }

    /// `end-of-defun`: the start of the next top-level definition, or the end
    /// of the buffer.
    pub fn end_of_defun(rope: &Rope, at: usize) -> usize {
        let last = rope.len_lines().saturating_sub(1);
        let mut line = rope.char_to_line(at.min(rope.len_chars())) + 1;
        while line < last {
            let start = rope.line_to_char(line);
            if let Some(c) = Self::char_at(rope, start)
                && !c.is_whitespace()
                && CharClass::of(c) != CharClass::Close
            {
                return start;
            }
            line += 1;
        }
        rope.len_chars()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn backward_sentence_never_moves_point_forward() {
        // Starting inside leading whitespace, the scan back reaches the buffer
        // start and the step that skips the separating whitespace used to
        // carry point past where it began. `backward-kill-sentence` turned
        // that into a backwards range and panicked.
        let rope = Rope::from_str("   hello world. and more.");
        for at in 0..=rope.len_chars() {
            let to = Motion::backward_sentence(&rope, at, 1);
            assert!(to <= at, "from {at} it moved forward to {to}");
        }
    }

    #[test]
    fn backward_sentence_still_lands_on_the_sentence_start() {
        // The clamp must not cost the ordinary case its behaviour: from inside
        // the second sentence, point goes to the first character of it, not
        // back onto the whitespace before it.
        let rope = Rope::from_str("One. Two three.");
        let at = rope.len_chars() - 1;
        assert_eq!(
            Motion::backward_sentence(&rope, at, 1),
            5,
            "the `T` of `Two`"
        );
    }

    use super::*;

    fn rope(s: &str) -> Rope {
        Rope::from_str(s)
    }

    #[test]
    fn char_classes_match_emacs_syntax_intuition() {
        assert_eq!(CharClass::of('a'), CharClass::Word);
        assert_eq!(CharClass::of('7'), CharClass::Word);
        assert_eq!(CharClass::of('_'), CharClass::Symbol);
        assert_eq!(CharClass::of(' '), CharClass::Whitespace);
        assert_eq!(CharClass::of('('), CharClass::Open);
        assert_eq!(CharClass::of('}'), CharClass::Close);
        assert_eq!(CharClass::of('"'), CharClass::Quote);
        assert_eq!(CharClass::of('+'), CharClass::Punctuation);
        assert!(
            CharClass::of('é').is_word(),
            "non-ASCII letters are word constituents"
        );
    }

    #[test]
    fn forward_and_backward_word_skip_separators() {
        let r = rope("foo  bar-baz");
        assert_eq!(Motion::forward_word(&r, 0, 1), 3);
        assert_eq!(Motion::forward_word(&r, 0, 2), 8, "stops at the hyphen");
        assert_eq!(Motion::forward_word(&r, 0, 3), 12);
        assert_eq!(
            Motion::forward_word(&r, 12, 1),
            12,
            "clamps at end of buffer"
        );
        assert_eq!(Motion::backward_word(&r, 12, 1), 9);
        assert_eq!(Motion::backward_word(&r, 12, 3), 0);
        assert_eq!(Motion::backward_word(&r, 0, 1), 0);
    }

    #[test]
    fn line_start_and_end_ignore_terminators() {
        let r = rope("alpha\nbeta\r\ngamma");
        assert_eq!(Motion::line_start(&r, 3), 0);
        assert_eq!(Motion::line_end(&r, 3), 5);
        assert_eq!(Motion::line_start(&r, 7), 6);
        assert_eq!(Motion::line_end(&r, 7), 10, "CRLF is stripped");
        assert_eq!(
            Motion::line_end(&r, 14),
            r.len_chars(),
            "final line has no terminator"
        );
    }

    #[test]
    fn back_to_indentation_finds_first_non_blank() {
        let r = rope("    indented\n\t\ttabs");
        assert_eq!(Motion::back_to_indentation(&r, 0), 4);
        assert_eq!(Motion::back_to_indentation(&r, 14), 15);
    }

    #[test]
    fn word_bounds_accept_point_just_after_a_word() {
        let r = rope("one two");
        assert_eq!(Motion::word_bounds(&r, 5), Some((4, 7)));
        assert_eq!(
            Motion::word_bounds(&r, 3),
            Some((0, 3)),
            "point after `one`"
        );
        assert_eq!(Motion::word_bounds(&rope("  "), 1), None);
    }

    #[test]
    fn paragraph_motion_stops_at_blank_lines() {
        let r = rope("a\nb\n\nc\nd\n\ne\n");
        let p = Motion::forward_paragraph(&r, 0, 1);
        assert_eq!(r.char_to_line(p), 2);
        let p2 = Motion::forward_paragraph(&r, p, 1);
        assert_eq!(r.char_to_line(p2), 5);
        assert_eq!(Motion::backward_paragraph(&r, p2, 1), r.line_to_char(2));
        assert_eq!(Motion::backward_paragraph(&r, 2, 1), 0);
    }

    #[test]
    fn sentence_motion_stops_after_terminators() {
        let r = rope("One two. Three four! Five.");
        assert_eq!(Motion::forward_sentence(&r, 0, 1), 8);
        assert_eq!(Motion::forward_sentence(&r, 0, 2), 20);
        assert_eq!(
            Motion::forward_sentence(&r, 0, 9),
            r.len_chars(),
            "clamps at the end"
        );
        assert_eq!(Motion::backward_sentence(&r, 26, 1), 21);
    }

    #[test]
    fn forward_sexp_traverses_nested_delimiters() {
        let r = rope("(a (b c) d) rest");
        assert_eq!(Motion::forward_sexp(&r, 0, 1), Some(11));
        assert_eq!(Motion::forward_sexp(&r, 11, 1), Some(16));
        assert_eq!(Motion::forward_sexp(&r, 3, 1), Some(8));
    }

    #[test]
    fn backward_sexp_mirrors_forward_sexp() {
        let r = rope("(a (b c) d) rest");
        assert_eq!(Motion::backward_sexp(&r, 11, 1), Some(0));
        assert_eq!(Motion::backward_sexp(&r, 16, 1), Some(12));
        assert_eq!(Motion::backward_sexp(&r, 8, 1), Some(3));
    }

    #[test]
    fn sexp_motion_reports_unbalanced_input() {
        let r = rope("(a b");
        assert_eq!(Motion::forward_sexp(&r, 0, 1), None);
        let r = rope("a)");
        assert_eq!(Motion::forward_sexp(&r, 1, 1), None);
    }

    #[test]
    fn sexp_motion_treats_strings_as_one_unit() {
        let r = rope(r#""a b c" tail"#);
        assert_eq!(Motion::forward_sexp(&r, 0, 1), Some(7));
        assert_eq!(Motion::backward_sexp(&r, 7, 1), Some(0));
        let escaped = rope(r#""a\"b" x"#);
        assert_eq!(
            Motion::forward_sexp(&escaped, 0, 1),
            Some(6),
            "escaped quotes do not terminate"
        );
    }

    #[test]
    fn matching_delimiter_works_in_both_directions() {
        let r = rope("fn f() { g([1]) }");
        assert_eq!(Motion::matching_delimiter(&r, 7), Some(16));
        assert_eq!(Motion::matching_delimiter(&r, 16), Some(7));
        assert_eq!(Motion::matching_delimiter(&r, 11), Some(13));
        assert_eq!(Motion::matching_delimiter(&r, 0), None, "not a delimiter");
        assert_eq!(
            Motion::matching_delimiter(&rope("("), 0),
            None,
            "unbalanced"
        );
    }

    #[test]
    fn defun_motion_anchors_on_column_zero() {
        let r = rope("fn a() {\n    body\n}\n\nfn b() {\n    body\n}\n");
        let inside_b = r.line_to_char(5) + 2;
        assert_eq!(Motion::beginning_of_defun(&r, inside_b), r.line_to_char(4));
        let inside_a = r.line_to_char(1) + 2;
        assert_eq!(Motion::end_of_defun(&r, inside_a), r.line_to_char(4));
        assert_eq!(
            Motion::beginning_of_defun(&r, 3),
            0,
            "clamps to buffer start"
        );
    }

    #[test]
    fn char_motion_clamps_at_both_ends() {
        let r = rope("abc");
        assert_eq!(Motion::forward_char(&r, 2, 5), 3);
        assert_eq!(Motion::backward_char(&r, 1, 5), 0);
    }
}
