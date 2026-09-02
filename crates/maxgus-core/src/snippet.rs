//! Snippets: a short name that becomes a shape you fill in.
//!
//! The body is written the way yasnippet and the language-server protocol
//! both write it — `$1`, `${2:default}`, `$0` for where to end up — so a
//! snippet copied from either works, and so a completion that arrives from a
//! server as a snippet can be inserted as one instead of being flattened into
//! its literal text.
//!
//! Parsing produces the text to insert and the fields as offsets into it. The
//! editor turns those into positions in the buffer; nothing here knows what a
//! buffer is.

/// One place the reader is meant to type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// The number after the `$`. `0` is where to leave point at the end.
    pub number: u32,
    /// Where the field is in the expanded text, in characters.
    pub start: usize,
    pub end: usize,
    /// The blanks a line of nothing but this field had before it. They are
    /// left out of the text and put back when the field is reached, so a
    /// line waiting to be typed on is not a line of trailing whitespace
    /// until then.
    pub indent: String,
}

impl Field {
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// A parsed snippet: what to insert, and where the fields are in it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Expansion {
    pub text: String,
    /// In the order they are visited: `$1`, `$2`, … and `$0` last.
    pub fields: Vec<Field>,
}

impl Expansion {
    /// Where point goes when there is nothing to fill in.
    pub fn end(&self) -> usize {
        self.text.chars().count()
    }
}

/// Parses a snippet body.
///
/// Anything that is not a field is text, including a `$` that is not followed
/// by a number or a brace — a snippet full of shell variables should not have
/// to escape every one of them. `\$` is a literal dollar for when it does.
pub fn parse(body: &str) -> Expansion {
    let chars: Vec<char> = body.chars().collect();
    let mut text = String::new();
    let mut length = 0usize;
    let mut fields: Vec<Field> = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            '\\' if i + 1 < chars.len() && chars[i + 1] == '$' => {
                text.push('$');
                length += 1;
                i += 2;
            }
            '$' if i + 1 < chars.len() && chars[i + 1] == '{' => match placeholder(&chars, i) {
                Some((number, default, next)) => {
                    let start = length;
                    text.push_str(&default);
                    length += default.chars().count();
                    fields.push(Field {
                        number,
                        start,
                        end: length,
                        indent: String::new(),
                    });
                    i = next;
                }
                None => {
                    text.push('$');
                    length += 1;
                    i += 1;
                }
            },
            '$' if i + 1 < chars.len() && chars[i + 1].is_ascii_digit() => {
                let mut j = i + 1;
                let mut number = 0u32;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    number = number.saturating_mul(10) + chars[j].to_digit(10).unwrap_or(0);
                    j += 1;
                }
                fields.push(Field {
                    number,
                    start: length,
                    end: length,
                    indent: String::new(),
                });
                i = j;
            }
            other => {
                text.push(other);
                length += 1;
                i += 1;
            }
        }
    }

    // The order they are visited in: by number, with `$0` last because it is
    // where the reader is left rather than something to fill in.
    fields.sort_by_key(|field| match field.number {
        0 => (1, 0),
        n => (0, n),
    });
    let text = hold_back_indentation(text, &mut fields);
    Expansion { text, fields }
}

/// Takes the blanks out of every line that is nothing but blanks and an
/// empty field, giving them to the field to put back when it is reached.
///
/// `    $0` on a line of its own is how most snippets say where the body
/// goes, and inserted as written it is a line of trailing whitespace —
/// marked as such — until the reader gets there.
fn hold_back_indentation(text: String, fields: &mut [Field]) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut kept = String::with_capacity(text.len());
    // Where each old offset lands in the shortened text; one more than the
    // characters, for a field at the very end.
    let mut moved: Vec<usize> = Vec::with_capacity(chars.len() + 1);
    let mut line_start = 0;
    loop {
        let line_end = chars[line_start..]
            .iter()
            .position(|&c| c == '\n')
            .map_or(chars.len(), |n| line_start + n);
        let line = &chars[line_start..line_end];
        let blank = !line.is_empty() && line.iter().all(|&c| c == ' ' || c == '\t');
        let field = fields
            .iter()
            .position(|f| f.start == line_end && f.end == line_end)
            .filter(|_| blank);
        match field {
            Some(index) => {
                fields[index].indent = line.iter().collect();
                // The whole line lands where it began, with nothing in it.
                moved.extend(std::iter::repeat_n(kept.chars().count(), line.len()));
            }
            None => {
                let at = kept.chars().count();
                moved.extend(at..at + line.len());
                kept.extend(line.iter());
            }
        }
        if line_end == chars.len() {
            moved.push(kept.chars().count());
            break;
        }
        moved.push(kept.chars().count());
        kept.push('\n');
        line_start = line_end + 1;
    }
    for field in fields.iter_mut() {
        field.start = moved[field.start];
        field.end = moved[field.end];
    }
    kept
}

/// Reads `${1:default}` from `chars` at `at`, returning the number, the
/// default text and where the placeholder ends.
fn placeholder(chars: &[char], at: usize) -> Option<(u32, String, usize)> {
    let mut i = at + 2;
    let mut number = 0u32;
    let mut digits = 0;
    while i < chars.len() && chars[i].is_ascii_digit() {
        number = number.saturating_mul(10) + chars[i].to_digit(10)?;
        digits += 1;
        i += 1;
    }
    if digits == 0 {
        return None;
    }
    let mut default = String::new();
    match chars.get(i) {
        Some('}') => return Some((number, default, i + 1)),
        Some(':') => i += 1,
        _ => return None,
    }
    // Nesting is not supported and does not need to be: a default containing
    // another field is rare, and reading the braces as text is closer to what
    // was meant than refusing the whole snippet.
    while i < chars.len() && chars[i] != '}' {
        default.push(chars[i]);
        i += 1;
    }
    match chars.get(i) {
        Some('}') => Some((number, default, i + 1)),
        _ => None,
    }
}

/// One snippet, as a file or the configuration defines it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    /// What is typed to get it.
    pub key: String,
    /// What it is called, for the list `M-x insert-snippet` offers.
    pub name: String,
    /// The mode it belongs to, or `None` for every mode.
    pub mode: Option<String>,
    pub body: String,
}

impl Snippet {
    /// Reads a snippet file: a header of `# key: …` lines, then the body.
    ///
    /// The same shape yasnippet uses, so a snippet can be copied from an
    /// Emacs configuration without being rewritten.
    pub fn parse_file(source: &str, fallback_name: &str, mode: Option<String>) -> Snippet {
        let mut key = String::new();
        let mut name = fallback_name.to_string();
        let mut body = String::new();
        let mut in_body = false;
        for line in source.lines() {
            if in_body {
                body.push_str(line);
                body.push('\n');
                continue;
            }
            let trimmed = line.trim();
            if trimmed == "# --" || trimmed == "#--" {
                in_body = true;
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("# key:") {
                key = rest.trim().to_string();
            } else if let Some(rest) = trimmed.strip_prefix("# name:") {
                name = rest.trim().to_string();
            } else if !trimmed.starts_with('#') && !trimmed.is_empty() {
                // A file with no header at all is all body, which is the
                // shortest thing a snippet can be.
                in_body = true;
                body.push_str(line);
                body.push('\n');
            }
        }
        // The trailing newline the file ends with is the file's, not the
        // snippet's: a snippet that wanted one says so with a blank line.
        while body.ends_with('\n') {
            body.pop();
        }
        if key.is_empty() {
            key = fallback_name.to_string();
        }
        Snippet {
            key,
            name,
            mode,
            body,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_with_no_fields_is_itself() {
        let expanded = parse("hello, world");
        assert_eq!(expanded.text, "hello, world");
        assert!(expanded.fields.is_empty());
    }

    #[test]
    fn a_numbered_field_is_an_empty_place_to_type() {
        let expanded = parse("fn $1() {}");
        assert_eq!(expanded.text, "fn () {}");
        assert_eq!(expanded.fields.len(), 1);
        assert_eq!(expanded.fields[0].number, 1);
        assert_eq!((expanded.fields[0].start, expanded.fields[0].end), (3, 3));
    }

    #[test]
    fn a_placeholder_puts_its_default_in_and_selects_it() {
        let expanded = parse("for ${1:item} in ${2:items} {}");
        assert_eq!(expanded.text, "for item in items {}");
        assert_eq!(expanded.fields.len(), 2);
        assert_eq!(
            &expanded.text[expanded.fields[0].start..expanded.fields[0].end],
            "item"
        );
        assert_eq!(
            &expanded.text[expanded.fields[1].start..expanded.fields[1].end],
            "items"
        );
    }

    #[test]
    fn fields_are_visited_in_order_with_the_last_stop_last() {
        let expanded = parse("$2 $0 $1");
        let order: Vec<u32> = expanded.fields.iter().map(|f| f.number).collect();
        assert_eq!(order, vec![1, 2, 0], "`$0` is where to end up, not first");
    }

    #[test]
    fn a_placeholder_with_no_default_is_still_a_field() {
        let expanded = parse("${1}x");
        assert_eq!(expanded.text, "x");
        assert_eq!(expanded.fields[0].number, 1);
        assert!(expanded.fields[0].is_empty());
    }

    #[test]
    fn a_dollar_that_is_not_a_field_is_a_dollar() {
        // A snippet full of shell variables should not have to escape them.
        let expanded = parse("echo $HOME and $ and ${}");
        assert_eq!(expanded.text, "echo $HOME and $ and ${}");
        assert!(expanded.fields.is_empty());
    }

    #[test]
    fn an_escaped_dollar_is_a_literal_one() {
        let expanded = parse(r"cost: \$$1");
        assert_eq!(expanded.text, "cost: $");
        assert_eq!(expanded.fields.len(), 1);
    }

    #[test]
    fn a_two_digit_field_is_one_field() {
        let expanded = parse("$10 $2");
        assert_eq!(expanded.fields.len(), 2);
        let order: Vec<u32> = expanded.fields.iter().map(|f| f.number).collect();
        assert_eq!(order, vec![2, 10]);
    }

    #[test]
    fn an_unclosed_placeholder_is_read_as_text() {
        let expanded = parse("${1:never closed");
        assert_eq!(expanded.text, "${1:never closed");
        assert!(expanded.fields.is_empty());
    }

    #[test]
    fn fields_are_counted_in_characters_rather_than_bytes() {
        let expanded = parse("café ${1:x}");
        assert_eq!(expanded.fields[0].start, 5, "it counted bytes");
    }

    #[test]
    fn the_blanks_before_a_field_alone_on_a_line_wait_for_it() {
        let expanded = parse("fn ${1:name}() {\n    $0\n}");
        assert_eq!(
            expanded.text, "fn name() {\n\n}",
            "the blank line kept its blanks"
        );
        let last = expanded.fields.last().expect("$0");
        assert_eq!(last.number, 0);
        assert_eq!(
            (last.start, last.end),
            (12, 12),
            "it is at the start of its line"
        );
        assert_eq!(last.indent, "    ");
        assert_eq!(
            expanded.fields[0].indent, "",
            "a field with text before it has nothing held"
        );
    }

    #[test]
    fn fields_after_a_held_back_line_move_up_with_it() {
        let expanded = parse("a\n\t$1\nb $2 c\n  $0");
        assert_eq!(expanded.text, "a\n\nb  c\n");
        let at: Vec<(u32, usize, usize, &str)> = expanded
            .fields
            .iter()
            .map(|f| (f.number, f.start, f.end, f.indent.as_str()))
            .collect();
        assert_eq!(at, vec![(1, 2, 2, "\t"), (2, 5, 5, ""), (0, 8, 8, "  ")]);
    }

    #[test]
    fn a_blank_line_with_no_field_on_it_is_left_as_written() {
        let expanded = parse("a\n    \nb$1");
        assert_eq!(expanded.text, "a\n    \nb");
        assert_eq!(expanded.fields[0].start, 8);
    }

    // ---- snippet files ---------------------------------------------------

    #[test]
    fn a_file_with_a_header_is_read_the_way_yasnippet_writes_one() {
        let snippet = Snippet::parse_file(
            "# -*- mode: snippet -*-\n\
             # name: a for loop\n\
             # key: for\n\
             # --\n\
             for ${1:item} in ${2:items} {\n    $0\n}\n",
            "for-loop",
            Some("rust-mode".into()),
        );
        assert_eq!(snippet.key, "for");
        assert_eq!(snippet.name, "a for loop");
        assert_eq!(snippet.mode.as_deref(), Some("rust-mode"));
        assert!(snippet.body.starts_with("for ${1:item}"));
        assert!(
            snippet.body.ends_with('}'),
            "the body kept a trailing newline"
        );
    }

    #[test]
    fn a_file_with_no_header_is_all_body() {
        let snippet = Snippet::parse_file("just text\n", "plain", None);
        assert_eq!(snippet.body, "just text");
        assert_eq!(snippet.key, "plain", "the filename is the key");
        assert_eq!(snippet.name, "plain");
    }

    #[test]
    fn a_body_can_contain_a_line_that_looks_like_a_header() {
        let snippet = Snippet::parse_file("# key: k\n# --\n# name: not a header\n", "f", None);
        assert_eq!(snippet.body, "# name: not a header");
    }
}
