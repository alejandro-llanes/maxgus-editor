//! Converting between buffer offsets and language-server positions.
//!
//! The protocol counts lines and columns; the editor counts characters. The
//! obvious conversion — render the buffer to a string and walk it — is correct
//! but costs the whole buffer on every call, which is ruinous when redisplay
//! does it per diagnostic per line. These use the rope's own line index
//! instead, so a conversion costs a line lookup and the length of that line.

use maxgus_lsp::{LspPosition, PositionEncoding};
use maxgus_text::Buffer;

/// The character offset of `position`, clamped into the buffer.
pub fn offset_of_position(
    buffer: &Buffer,
    position: LspPosition,
    encoding: PositionEncoding,
) -> usize {
    let line = position.line as usize;
    if line >= buffer.len_lines() {
        return buffer.len_chars();
    }
    let start = buffer.line_start(line);
    let text = buffer.line_text(line);
    start + maxgus_lsp::position::char_offset_of(&text, position.character, encoding)
}

/// The position of `offset`, in the units `encoding` counts.
pub fn position_of_offset(
    buffer: &Buffer,
    offset: usize,
    encoding: PositionEncoding,
) -> LspPosition {
    let offset = offset.min(buffer.len_chars());
    let line = buffer.line_of(offset);
    let start = buffer.line_start(line);
    let text = buffer.line_text(line);
    let character = maxgus_lsp::position::character_of(&text, offset - start, encoding);
    LspPosition::new(line as u32, character)
}

#[cfg(test)]
mod tests {
    use super::*;
    use maxgus_text::BufferId;

    fn buffer(text: &str) -> Buffer {
        Buffer::from_str(BufferId(1), "test", text)
    }

    #[test]
    fn positions_and_offsets_round_trip() {
        let b = buffer("one\ntwo\nthree\n");
        let encoding = PositionEncoding::Utf16;
        for offset in 0..=b.len_chars() {
            let position = position_of_offset(&b, offset, encoding);
            assert_eq!(offset_of_position(&b, position, encoding), offset, "offset {offset}");
        }
    }

    #[test]
    fn they_agree_with_the_whole_text_conversion() {
        // The slow, obviously-correct conversion is the reference.
        let b = buffer("ascii\né漢🎉 tail\nlast line\n");
        let text = b.text();
        let encoding = PositionEncoding::Utf16;
        for offset in 0..=b.len_chars() {
            let fast = position_of_offset(&b, offset, encoding);
            let slow = maxgus_lsp::position::offset_to_position(&text, offset, encoding);
            assert_eq!(fast, slow, "offset {offset}");
            assert_eq!(
                offset_of_position(&b, fast, encoding),
                maxgus_lsp::position::position_to_offset(&text, slow, encoding),
                "position {fast:?}"
            );
        }
    }

    #[test]
    fn a_line_past_the_end_clamps_to_the_end_of_the_buffer() {
        let b = buffer("one\ntwo");
        let encoding = PositionEncoding::Utf16;
        assert_eq!(offset_of_position(&b, LspPosition::new(99, 0), encoding), b.len_chars());
        assert_eq!(position_of_offset(&b, 999, encoding), LspPosition::new(1, 3));
    }

    #[test]
    fn a_character_past_the_end_of_a_line_clamps_to_it() {
        let b = buffer("one\ntwo\n");
        let encoding = PositionEncoding::Utf16;
        // Column 99 of line 0 is the end of `one`, not the next line.
        assert_eq!(offset_of_position(&b, LspPosition::new(0, 99), encoding), 3);
    }

    #[test]
    fn an_empty_buffer_has_one_position() {
        let b = buffer("");
        let encoding = PositionEncoding::Utf16;
        assert_eq!(position_of_offset(&b, 0, encoding), LspPosition::ZERO);
        assert_eq!(offset_of_position(&b, LspPosition::ZERO, encoding), 0);
    }
}
