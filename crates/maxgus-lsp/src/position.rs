//! Position arithmetic.
//!
//! LSP positions are a line number plus a character offset measured in the
//! negotiated encoding — UTF-16 code units unless the server agrees otherwise.
//! `maxgus` works in Unicode scalar values, so every position crossing the wire
//! is converted. Getting this wrong shows up as diagnostics landing one column
//! off on any line containing a non-ASCII character, so it is tested closely.

use serde::{Deserialize, Serialize};

/// How a server counts the `character` field of a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PositionEncoding {
    /// Bytes. Offered by servers that advertise `utf-8`.
    Utf8,
    /// UTF-16 code units. The protocol default, and the only one every server
    /// is required to support.
    #[default]
    Utf16,
    /// Unicode scalar values, which is what the editor uses internally.
    Utf32,
}

impl PositionEncoding {
    /// The name used in `general.positionEncodings` during initialisation.
    pub fn wire_name(self) -> &'static str {
        match self {
            PositionEncoding::Utf8 => "utf-8",
            PositionEncoding::Utf16 => "utf-16",
            PositionEncoding::Utf32 => "utf-32",
        }
    }

    /// Parses the encoding a server picked, defaulting to UTF-16.
    pub fn from_wire_name(name: &str) -> PositionEncoding {
        match name {
            "utf-8" => PositionEncoding::Utf8,
            "utf-32" => PositionEncoding::Utf32,
            _ => PositionEncoding::Utf16,
        }
    }

    /// The width of `c` in this encoding.
    fn width_of(self, c: char) -> usize {
        match self {
            PositionEncoding::Utf8 => c.len_utf8(),
            PositionEncoding::Utf16 => c.len_utf16(),
            PositionEncoding::Utf32 => 1,
        }
    }
}

/// A zero-based line and encoding-dependent character offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}

impl LspPosition {
    pub const ZERO: LspPosition = LspPosition {
        line: 0,
        character: 0,
    };

    pub fn new(line: u32, character: u32) -> LspPosition {
        LspPosition { line, character }
    }
}

/// A half-open range of positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}

impl LspRange {
    pub fn new(start: LspPosition, end: LspPosition) -> LspRange {
        LspRange { start, end }
    }

    pub fn empty(at: LspPosition) -> LspRange {
        LspRange { start: at, end: at }
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// True when the range covers a single line.
    pub fn is_single_line(&self) -> bool {
        self.start.line == self.end.line
    }
}

/// Converts a character offset *within one line* into the LSP character field.
pub fn character_of(line: &str, char_offset: usize, encoding: PositionEncoding) -> u32 {
    line.chars()
        .take(char_offset)
        .map(|c| encoding.width_of(c))
        .sum::<usize>() as u32
}

/// The inverse: converts an LSP character field into a character offset within
/// the line.
///
/// A `character` past the end of the line clamps to the line length, as the
/// specification directs. A `character` landing inside a multi-unit character
/// resolves to that character's start, which is the only sane reading.
pub fn char_offset_of(line: &str, character: u32, encoding: PositionEncoding) -> usize {
    let target = character as usize;
    let mut units = 0usize;
    for (index, c) in line.chars().enumerate() {
        if units >= target {
            return index;
        }
        let width = encoding.width_of(c);
        // The target falls inside this character; its start is the answer.
        if units + width > target {
            return index;
        }
        units += width;
    }
    line.chars().count()
}

/// The length of `line` in `encoding`'s units.
pub fn line_length(line: &str, encoding: PositionEncoding) -> u32 {
    line.chars().map(|c| encoding.width_of(c)).sum::<usize>() as u32
}

/// Splits `text` into lines the way LSP counts them: on `\n`, with `\r`
/// stripped, and with a trailing newline producing a final empty line.
pub fn lines(text: &str) -> Vec<&str> {
    let mut out: Vec<&str> = text
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    if out.is_empty() {
        out.push("");
    }
    out
}

/// The position of a *byte* offset.
///
/// Text edits arrive from the editor as byte ranges, because that is what
/// comparing two strings produces; the protocol wants line and character. This
/// converts without first counting characters across the whole document.
pub fn byte_to_position(text: &str, byte: usize, encoding: PositionEncoding) -> LspPosition {
    let byte = byte.min(text.len());
    // Back off to a character boundary so the line scan cannot split one.
    let byte = (0..=byte)
        .rev()
        .find(|b| text.is_char_boundary(*b))
        .unwrap_or(0);
    let before = &text[..byte];
    let line = before.bytes().filter(|b| *b == b'\n').count();
    let line_start = before.rfind('\n').map_or(0, |at| at + 1);
    let within = &text[line_start..byte];
    LspPosition::new(line as u32, line_length(within, encoding))
}

/// Converts a document-wide character offset into an LSP position.
pub fn offset_to_position(
    text: &str,
    char_offset: usize,
    encoding: PositionEncoding,
) -> LspPosition {
    let mut consumed = 0usize;
    for (line_number, line) in lines(text).into_iter().enumerate() {
        let line_chars = line.chars().count();
        // `+ 1` accounts for the newline that separates this line from the next.
        if consumed + line_chars >= char_offset {
            let within = char_offset - consumed;
            return LspPosition::new(line_number as u32, character_of(line, within, encoding));
        }
        consumed += line_chars + 1;
    }
    // Past the end: clamp to the final position.
    let all = lines(text);
    let last = all.len().saturating_sub(1);
    LspPosition::new(last as u32, line_length(all[last], encoding))
}

/// Converts an LSP position into a document-wide character offset, clamping
/// out-of-range lines to the end of the document.
pub fn position_to_offset(text: &str, position: LspPosition, encoding: PositionEncoding) -> usize {
    let all = lines(text);
    let line_number = position.line as usize;
    if line_number >= all.len() {
        return text.chars().count();
    }
    let mut offset = 0usize;
    for line in &all[..line_number] {
        offset += line.chars().count() + 1;
    }
    offset + char_offset_of(all[line_number], position.character, encoding)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line with a two-byte char, a three-byte char and an astral char.
    const MIXED: &str = "aé漢🎉b";

    #[test]
    fn encodings_round_trip_through_their_wire_names() {
        for e in [
            PositionEncoding::Utf8,
            PositionEncoding::Utf16,
            PositionEncoding::Utf32,
        ] {
            assert_eq!(PositionEncoding::from_wire_name(e.wire_name()), e);
        }
        assert_eq!(
            PositionEncoding::from_wire_name("nonsense"),
            PositionEncoding::Utf16
        );
        assert_eq!(PositionEncoding::default(), PositionEncoding::Utf16);
    }

    #[test]
    fn ascii_is_the_same_in_every_encoding() {
        for e in [
            PositionEncoding::Utf8,
            PositionEncoding::Utf16,
            PositionEncoding::Utf32,
        ] {
            assert_eq!(character_of("hello", 3, e), 3);
            assert_eq!(char_offset_of("hello", 3, e), 3);
            assert_eq!(line_length("hello", e), 5);
        }
    }

    #[test]
    fn utf16_counts_surrogate_pairs_as_two() {
        // a=1, é=1, 漢=1, 🎉=2 code units.
        let e = PositionEncoding::Utf16;
        assert_eq!(character_of(MIXED, 0, e), 0);
        assert_eq!(character_of(MIXED, 1, e), 1, "after `a`");
        assert_eq!(character_of(MIXED, 2, e), 2, "after `é`");
        assert_eq!(character_of(MIXED, 3, e), 3, "after `漢`");
        assert_eq!(character_of(MIXED, 4, e), 5, "after the astral char");
        assert_eq!(line_length(MIXED, e), 6);
    }

    #[test]
    fn utf8_counts_bytes() {
        // a=1, é=2, 漢=3, 🎉=4 bytes.
        let e = PositionEncoding::Utf8;
        assert_eq!(character_of(MIXED, 1, e), 1);
        assert_eq!(character_of(MIXED, 2, e), 3);
        assert_eq!(character_of(MIXED, 3, e), 6);
        assert_eq!(character_of(MIXED, 4, e), 10);
        assert_eq!(line_length(MIXED, e), 11);
    }

    #[test]
    fn utf32_counts_scalar_values() {
        let e = PositionEncoding::Utf32;
        assert_eq!(character_of(MIXED, 4, e), 4);
        assert_eq!(line_length(MIXED, e), 5);
    }

    #[test]
    fn character_conversion_round_trips_in_every_encoding() {
        for e in [
            PositionEncoding::Utf8,
            PositionEncoding::Utf16,
            PositionEncoding::Utf32,
        ] {
            for offset in 0..=MIXED.chars().count() {
                let character = character_of(MIXED, offset, e);
                assert_eq!(
                    char_offset_of(MIXED, character, e),
                    offset,
                    "{e:?} at {offset}"
                );
            }
        }
    }

    #[test]
    fn a_character_inside_a_multi_unit_char_resolves_to_its_start() {
        // Character 4 lands in the middle of the surrogate pair for `🎉`.
        assert_eq!(char_offset_of(MIXED, 4, PositionEncoding::Utf16), 3);
    }

    #[test]
    fn a_character_past_the_end_clamps_to_the_line_length() {
        assert_eq!(char_offset_of("abc", 99, PositionEncoding::Utf16), 3);
        assert_eq!(char_offset_of("", 5, PositionEncoding::Utf16), 0);
    }

    #[test]
    fn lines_are_split_the_way_lsp_counts_them() {
        assert_eq!(lines("a\nb\nc"), vec!["a", "b", "c"]);
        assert_eq!(
            lines("a\nb\n"),
            vec!["a", "b", ""],
            "a trailing newline adds a line"
        );
        assert_eq!(
            lines("a\r\nb"),
            vec!["a", "b"],
            "carriage returns are stripped"
        );
        assert_eq!(lines(""), vec![""]);
    }

    #[test]
    fn document_offsets_convert_to_positions() {
        let text = "one\ntwo\nthree";
        let e = PositionEncoding::Utf16;
        assert_eq!(offset_to_position(text, 0, e), LspPosition::new(0, 0));
        assert_eq!(
            offset_to_position(text, 3, e),
            LspPosition::new(0, 3),
            "end of line 0"
        );
        assert_eq!(
            offset_to_position(text, 4, e),
            LspPosition::new(1, 0),
            "start of line 1"
        );
        assert_eq!(offset_to_position(text, 9, e), LspPosition::new(2, 1));
    }

    #[test]
    fn positions_convert_back_to_document_offsets() {
        let text = "one\ntwo\nthree";
        let e = PositionEncoding::Utf16;
        for offset in 0..=text.chars().count() {
            let position = offset_to_position(text, offset, e);
            assert_eq!(
                position_to_offset(text, position, e),
                offset,
                "offset {offset}"
            );
        }
    }

    #[test]
    fn document_conversion_handles_multibyte_lines() {
        let text = "ascii\né漢🎉 tail\nlast";
        let e = PositionEncoding::Utf16;
        // The character after the astral char on line 1.
        let offset = text.chars().position(|c| c == ' ').unwrap();
        let position = offset_to_position(text, offset, e);
        assert_eq!(position.line, 1);
        assert_eq!(position.character, 4, "é=1, 漢=1, 🎉=2");
        assert_eq!(position_to_offset(text, position, e), offset);
    }

    #[test]
    fn byte_offsets_convert_to_positions() {
        let text = "one\ntwo\nthree";
        let e = PositionEncoding::Utf16;
        assert_eq!(byte_to_position(text, 0, e), LspPosition::new(0, 0));
        assert_eq!(byte_to_position(text, 3, e), LspPosition::new(0, 3));
        assert_eq!(byte_to_position(text, 4, e), LspPosition::new(1, 0));
        assert_eq!(byte_to_position(text, 9, e), LspPosition::new(2, 1));
        assert_eq!(
            byte_to_position(text, 999, e),
            LspPosition::new(2, 5),
            "clamps"
        );
    }

    #[test]
    fn byte_conversion_agrees_with_the_character_one() {
        let text = "ascii\né漢🎉 tail\nlast";
        let e = PositionEncoding::Utf16;
        for (byte, _) in text
            .char_indices()
            .chain(std::iter::once((text.len(), ' ')))
        {
            let chars = text[..byte].chars().count();
            assert_eq!(
                byte_to_position(text, byte, e),
                offset_to_position(text, chars, e),
                "byte {byte}"
            );
        }
    }

    #[test]
    fn a_byte_inside_a_character_backs_off_to_its_start() {
        // The second byte of `é` is not a boundary.
        let text = "aéb";
        let e = PositionEncoding::Utf16;
        assert_eq!(byte_to_position(text, 2, e), byte_to_position(text, 1, e));
    }

    #[test]
    fn an_out_of_range_line_clamps_to_the_end_of_the_document() {
        let text = "a\nb";
        let e = PositionEncoding::Utf16;
        assert_eq!(position_to_offset(text, LspPosition::new(99, 0), e), 3);
        assert_eq!(offset_to_position(text, 999, e), LspPosition::new(1, 1));
    }

    #[test]
    fn an_empty_document_has_exactly_one_position() {
        let e = PositionEncoding::Utf16;
        assert_eq!(offset_to_position("", 0, e), LspPosition::ZERO);
        assert_eq!(position_to_offset("", LspPosition::ZERO, e), 0);
    }

    #[test]
    fn a_trailing_newline_leaves_a_final_empty_line() {
        let text = "a\n";
        let e = PositionEncoding::Utf16;
        assert_eq!(offset_to_position(text, 2, e), LspPosition::new(1, 0));
        assert_eq!(position_to_offset(text, LspPosition::new(1, 0), e), 2);
    }

    #[test]
    fn ranges_report_their_shape() {
        let a = LspPosition::new(1, 0);
        let b = LspPosition::new(1, 5);
        assert!(!LspRange::new(a, b).is_empty());
        assert!(LspRange::new(a, b).is_single_line());
        assert!(LspRange::empty(a).is_empty());
        assert!(!LspRange::new(a, LspPosition::new(3, 0)).is_single_line());
    }

    #[test]
    fn positions_order_by_line_then_character() {
        assert!(LspPosition::new(0, 9) < LspPosition::new(1, 0));
        assert!(LspPosition::new(1, 2) < LspPosition::new(1, 3));
    }
}
