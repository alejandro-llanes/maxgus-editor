//! The markdown a language server sends, as something worth looking at.
//!
//! Hover replies arrive as markdown — clangd and rust-analyzer both send a
//! heading, a rule, a bulleted list of parameters, prose, and the signature
//! in a fenced code block. Drawn as it arrives, that reads as
//! ```text
//! ### function `add`
//! ---
//! → `int`
//! ```
//! which is worse than plain prose, because now the punctuation is in the
//! way as well.
//!
//! This turns it into runs of text with a face each, wrapped to a width. It
//! is deliberately a *subset*: headings, rules, fenced blocks, inline code,
//! bold, italic and bullets. Anything else is left as the text it is, which
//! is the right failure — a document with syntax nobody here understands
//! still reads as what it says.

/// One run of text, and how to draw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    /// A face name from [`maxgus_faces::names`].
    pub face: &'static str,
    pub bold: bool,
    pub italic: bool,
}

impl Span {
    fn plain(text: impl Into<String>) -> Span {
        Span {
            text: text.into(),
            face: "default",
            bold: false,
            italic: false,
        }
    }

    fn faced(text: impl Into<String>, face: &'static str) -> Span {
        Span {
            text: text.into(),
            face,
            bold: false,
            italic: false,
        }
    }

    fn width(&self) -> usize {
        self.text.chars().count()
    }
}

/// One line of a rendered document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// Runs of text.
    Text(Vec<Span>),
    /// A `---`, drawn as a rule across the box rather than three hyphens.
    Rule,
}

impl Line {
    /// How wide this line wants to be.
    fn width(&self) -> usize {
        match self {
            Line::Rule => 0,
            Line::Text(spans) => spans.iter().map(Span::width).sum(),
        }
    }
}

/// The width `markdown` would like, ignoring wrapping.
///
/// Used to size the box before rendering into it: a signature that fits on
/// one line should get one line, and a paragraph should not make the box as
/// wide as the paragraph.
pub fn natural_width(markdown: &str) -> usize {
    render(markdown, usize::MAX)
        .iter()
        .map(Line::width)
        .max()
        .unwrap_or(0)
}

/// Renders `markdown` into lines no wider than `width`.
pub fn render(markdown: &str, width: usize) -> Vec<Line> {
    let mut out = Vec::new();
    let mut in_code = false;
    for raw in markdown.lines() {
        let line = raw.trim_end();

        // ``` opens and closes a code block. The language after it is the
        // server saying what the block is; there is nothing to do with that
        // here beyond not printing it.
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            // Code is not wrapped on spaces — a signature broken at a space
            // is a signature nobody can read. It is cut instead, which at
            // least keeps the shape.
            for chunk in hard_wrap(line, width) {
                out.push(Line::Text(vec![Span::faced(chunk, "doc-code")]));
            }
            continue;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            out.push(Line::Text(Vec::new()));
            continue;
        }
        // A rule: three or more of `-`, `_` or `*` and nothing else.
        if trimmed.len() >= 3
            && (trimmed.chars().all(|c| c == '-')
                || trimmed.chars().all(|c| c == '_')
                || trimmed.chars().all(|c| c == '*'))
        {
            out.push(Line::Rule);
            continue;
        }
        // A heading. The `#`s go; what is left is the heading.
        if let Some(rest) = trimmed.strip_prefix('#') {
            let text = rest.trim_start_matches('#').trim();
            let mut spans = inline(text);
            for span in &mut spans {
                span.bold = true;
                if span.face == "default" {
                    span.face = "font-lock-heading";
                }
            }
            out.extend(wrap(spans, width));
            continue;
        }
        // A bullet, with a real bullet rather than a hyphen.
        if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "))
        {
            let mut spans = vec![Span::faced("• ", "font-lock-punctuation")];
            spans.extend(inline(rest));
            out.extend(wrap(spans, width));
            continue;
        }
        out.extend(wrap(inline(line), width));
    }
    // A document that ends in blank lines is a box with empty rows at the
    // bottom of it.
    while matches!(out.last(), Some(Line::Text(spans)) if spans.is_empty()) {
        out.pop();
    }
    out
}

/// The inline markup within one line: `code`, **bold**, *italic*.
fn inline(text: &str) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    let mut plain = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    let flush = |plain: &mut String, spans: &mut Vec<Span>| {
        if !plain.is_empty() {
            spans.push(Span::plain(std::mem::take(plain)));
        }
    };

    while i < chars.len() {
        match chars[i] {
            // `code`
            '`' => match closing(&chars, i + 1, '`', 1) {
                Some(end) => {
                    flush(&mut plain, &mut spans);
                    spans.push(Span::faced(
                        chars[i + 1..end].iter().collect::<String>(),
                        "doc-code",
                    ));
                    i = end + 1;
                }
                None => {
                    plain.push('`');
                    i += 1;
                }
            },
            // **bold**
            '*' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                match closing(&chars, i + 2, '*', 2) {
                    Some(end) => {
                        flush(&mut plain, &mut spans);
                        for mut span in inline(&chars[i + 2..end].iter().collect::<String>()) {
                            span.bold = true;
                            spans.push(span);
                        }
                        i = end + 2;
                    }
                    None => {
                        plain.push('*');
                        i += 1;
                    }
                }
            }
            // *italic* and _italic_
            '*' | '_' => {
                let marker = chars[i];
                // An underscore inside a word is part of the word.
                // `snake_case_name` is not three italics, and hover text is
                // made of identifiers — this is CommonMark's rule and the
                // reason it exists.
                let intraword = marker == '_' && i > 0 && chars[i - 1].is_alphanumeric();
                match closing(&chars, i + 1, marker, 1).filter(|end| {
                    !intraword
                        && !(marker == '_'
                            && chars.get(end + 1).is_some_and(|c| c.is_alphanumeric()))
                }) {
                    Some(end) => {
                        flush(&mut plain, &mut spans);
                        for mut span in inline(&chars[i + 1..end].iter().collect::<String>()) {
                            span.italic = true;
                            spans.push(span);
                        }
                        i = end + 1;
                    }
                    None => {
                        plain.push(marker);
                        i += 1;
                    }
                }
            }
            other => {
                plain.push(other);
                i += 1;
            }
        }
    }
    flush(&mut plain, &mut spans);
    spans
}

/// Where a run of `count` `marker`s closes, searching from `from`.
///
/// `None` when it never does, which is what makes a stray asterisk in prose
/// an asterisk rather than the start of something.
fn closing(chars: &[char], from: usize, marker: char, count: usize) -> Option<usize> {
    let mut i = from;
    while i + count <= chars.len() {
        if chars[i..i + count].iter().all(|c| *c == marker) {
            // An empty `` or **** is not emphasis.
            return (i > from).then_some(i);
        }
        i += 1;
    }
    None
}

/// Breaks a line of spans at spaces so none is wider than `width`.
fn wrap(spans: Vec<Span>, width: usize) -> Vec<Line> {
    if width == 0 {
        return Vec::new();
    }
    if spans.iter().map(Span::width).sum::<usize>() <= width {
        return vec![Line::Text(spans)];
    }
    let mut out: Vec<Line> = Vec::new();
    let mut line: Vec<Span> = Vec::new();
    let mut used = 0usize;

    for span in spans {
        // A span is broken word by word, keeping its face on every piece.
        for word in split_keeping_spaces(&span.text) {
            let w = word.chars().count();
            let is_space = word.chars().all(char::is_whitespace);
            if used + w > width && !line.is_empty() {
                out.push(Line::Text(std::mem::take(&mut line)));
                used = 0;
                // A wrapped line does not begin with the space it broke on.
                if is_space {
                    continue;
                }
            }
            // A single word longer than the whole width has to be cut.
            if w > width {
                for chunk in hard_wrap(&word, width) {
                    if !line.is_empty() {
                        out.push(Line::Text(std::mem::take(&mut line)));
                    }
                    let cut = chunk.chars().count();
                    line.push(Span {
                        text: chunk,
                        ..span.clone()
                    });
                    used = cut;
                }
                continue;
            }
            match line.last_mut() {
                Some(last) if last.face == span.face && last.bold == span.bold => {
                    last.text.push_str(&word)
                }
                _ => line.push(Span {
                    text: word,
                    ..span.clone()
                }),
            }
            used += w;
        }
    }
    if !line.is_empty() {
        out.push(Line::Text(line));
    }
    out
}

/// Words and the whitespace between them, in order.
fn split_keeping_spaces(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_space = None;
    for ch in text.chars() {
        let space = ch.is_whitespace();
        if in_space != Some(space) && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
        in_space = Some(space);
        current.push(ch);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Cuts a string into pieces of at most `width` characters.
fn hard_wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    if text.chars().count() <= width {
        return vec![text.to_string()];
    }
    let chars: Vec<char> = text.chars().collect();
    chars
        .chunks(width)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text of a rendered line, with the markup gone.
    fn text(line: &Line) -> String {
        match line {
            Line::Rule => "───".into(),
            Line::Text(spans) => spans.iter().map(|s| s.text.as_str()).collect(),
        }
    }

    fn faces(line: &Line) -> Vec<&'static str> {
        match line {
            Line::Rule => Vec::new(),
            Line::Text(spans) => spans.iter().map(|s| s.face).collect(),
        }
    }

    /// What clangd really sends, once the client asks for markdown.
    const CLANGD: &str = "### function `add`\n\n---\n→ `int`\n\nParameters:\n\n\
                          - `int a`\n- `int b`\n\nAdds two numbers together.\n\n\
                          ---\n```cpp\nstatic int add(int a, int b)\n```";

    #[test]
    fn a_real_hover_reply_loses_its_punctuation_and_keeps_its_shape() {
        let lines = render(CLANGD, 60);
        let shown: Vec<String> = lines.iter().map(text).collect();
        let joined = shown.join("\n");
        assert!(
            !joined.contains("###") && !joined.contains("```"),
            "the markup is still on screen:\n{joined}"
        );
        assert!(
            joined.contains("function add"),
            "the heading lost its words:\n{joined}"
        );
        assert!(
            shown.iter().any(|l| l.starts_with("• int a")),
            "a list item is not a bullet:\n{joined}"
        );
        assert!(
            joined.contains("static int add(int a, int b)"),
            "the signature is gone:\n{joined}"
        );
        assert!(lines.contains(&Line::Rule), "`---` did not become a rule");
    }

    #[test]
    fn the_signature_is_marked_as_code_and_the_heading_as_a_heading() {
        let lines = render(CLANGD, 60);
        let signature = lines
            .iter()
            .find(|l| text(l).contains("static int add"))
            .expect("the signature");
        assert_eq!(faces(signature), ["doc-code"]);

        let heading = lines
            .iter()
            .find(|l| text(l).contains("function add"))
            .expect("the heading");
        let Line::Text(spans) = heading else {
            panic!("a rule is not a heading")
        };
        assert!(spans.iter().all(|s| s.bold), "the heading is not bold");
        assert!(
            spans.iter().any(|s| s.face == "doc-code"),
            "`add` was in backticks and should still read as code: {spans:?}"
        );
    }

    #[test]
    fn inline_code_becomes_a_face_rather_than_backticks() {
        let lines = render("a `code` word", 40);
        let Line::Text(spans) = &lines[0] else {
            panic!("expected text")
        };
        assert_eq!(
            spans
                .iter()
                .map(|s| (s.text.as_str(), s.face))
                .collect::<Vec<_>>(),
            [
                ("a ", "default"),
                ("code", "doc-code"),
                (" word", "default")
            ]
        );
    }

    #[test]
    fn bold_and_italic_are_attributes_rather_than_asterisks() {
        let lines = render("**very** and *slightly*", 40);
        let Line::Text(spans) = &lines[0] else {
            panic!("expected text")
        };
        let bold = spans.iter().find(|s| s.text == "very").expect("bold");
        assert!(bold.bold && !bold.italic);
        let italic = spans.iter().find(|s| s.text == "slightly").expect("italic");
        assert!(italic.italic && !italic.bold);
        assert!(
            !spans.iter().any(|s| s.text.contains('*')),
            "asterisks survived: {spans:?}"
        );
    }

    #[test]
    fn punctuation_that_opens_nothing_stays_as_punctuation() {
        // Prose is full of asterisks and underscores that mean nothing.
        for source in ["2 * 3 = 6", "a_b_c", "an * on its own", "back ` tick"] {
            let lines = render(source, 40);
            assert_eq!(text(&lines[0]), source, "`{source}` was mangled");
        }
    }

    #[test]
    fn a_long_paragraph_wraps_at_spaces() {
        let lines = render("one two three four five six seven eight nine", 20);
        assert!(lines.len() > 1, "it did not wrap");
        for line in &lines {
            assert!(
                line.width() <= 20,
                "`{}` is {} wide",
                text(line),
                line.width()
            );
        }
        let joined: String = lines.iter().map(text).collect::<Vec<_>>().join(" ");
        assert!(joined.contains("eight nine"), "words were lost: {joined}");
    }

    #[test]
    fn a_wrapped_line_keeps_the_face_of_what_it_split() {
        let lines = render("`a very long piece of code indeed`", 12);
        assert!(lines.len() > 1, "it did not wrap");
        for line in &lines {
            assert!(
                faces(line).iter().all(|f| *f == "doc-code"),
                "a piece of the code lost its face: {:?}",
                faces(line)
            );
        }
    }

    #[test]
    fn code_is_cut_rather_than_broken_at_spaces() {
        // A signature wrapped on a space reads as two signatures.
        let lines = render("```\nfn a(b: i32, c: i32) -> i32\n```", 12);
        assert!(lines.len() > 1);
        assert_eq!(text(&lines[0]).chars().count(), 12, "it was not filled");
    }

    #[test]
    fn a_word_longer_than_the_box_is_cut_rather_than_lost() {
        let lines = render("short Supercalifragilisticexpialidocious end", 10);
        let joined: String = lines.iter().map(text).collect();
        assert!(
            joined.contains("Supercalifragilistic"),
            "the long word vanished: {joined}"
        );
        assert!(
            joined.contains("end"),
            "what followed it vanished: {joined}"
        );
        for line in &lines {
            assert!(line.width() <= 10, "`{}` is too wide", text(line));
        }
    }

    #[test]
    fn plain_text_is_left_alone() {
        // Servers that send no markdown still send something.
        let plain = "function add\n\n-> int\n\nAdds two numbers.";
        let lines = render(plain, 40);
        let joined: Vec<String> = lines.iter().map(text).collect();
        assert_eq!(joined[0], "function add");
        assert!(joined.iter().any(|l| l == "-> int"));
    }

    #[test]
    fn the_natural_width_is_the_widest_line_it_would_draw() {
        // The box is sized from this, so it has to be the rendered width
        // rather than the markdown's.
        let width = natural_width("### `add`\n\n---\n\nA short line.");
        assert_eq!(width, "A short line.".len(), "got {width}");
    }

    #[test]
    fn trailing_blank_lines_are_not_box_to_pay_for() {
        let lines = render("text\n\n\n\n", 40);
        assert_eq!(lines.len(), 1, "got {lines:?}");
    }
}
