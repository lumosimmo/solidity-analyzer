use std::ops::{Add, AddAssign, Sub, SubAssign};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TextSize {
    raw: u32,
}

impl TextSize {
    pub const fn new(raw: u32) -> Self {
        Self { raw }
    }

    pub const fn raw(self) -> u32 {
        self.raw
    }

    pub fn of(text: &str) -> Self {
        Self::new(text.len() as u32)
    }

    pub const fn checked_add(self, rhs: TextSize) -> Option<TextSize> {
        match self.raw.checked_add(rhs.raw) {
            Some(raw) => Some(TextSize { raw }),
            None => None,
        }
    }

    pub const fn checked_sub(self, rhs: TextSize) -> Option<TextSize> {
        match self.raw.checked_sub(rhs.raw) {
            Some(raw) => Some(TextSize { raw }),
            None => None,
        }
    }
}

impl From<u32> for TextSize {
    fn from(raw: u32) -> Self {
        Self { raw }
    }
}

impl From<TextSize> for u32 {
    fn from(value: TextSize) -> Self {
        value.raw
    }
}

impl TryFrom<usize> for TextSize {
    type Error = std::num::TryFromIntError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        u32::try_from(value).map(TextSize::from)
    }
}

impl From<TextSize> for usize {
    fn from(value: TextSize) -> Self {
        value.raw as usize
    }
}

impl Add for TextSize {
    type Output = TextSize;

    fn add(self, rhs: TextSize) -> TextSize {
        TextSize::new(self.raw + rhs.raw)
    }
}

impl AddAssign for TextSize {
    fn add_assign(&mut self, rhs: TextSize) {
        self.raw += rhs.raw;
    }
}

impl Sub for TextSize {
    type Output = TextSize;

    fn sub(self, rhs: TextSize) -> TextSize {
        TextSize::new(self.raw - rhs.raw)
    }
}

impl SubAssign for TextSize {
    fn sub_assign(&mut self, rhs: TextSize) {
        self.raw -= rhs.raw;
    }
}

pub fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextRange {
    start: TextSize,
    end: TextSize,
}

impl TextRange {
    pub const fn new(start: TextSize, end: TextSize) -> TextRange {
        assert!(start.raw <= end.raw);
        TextRange { start, end }
    }

    pub const fn at(start: TextSize, len: TextSize) -> TextRange {
        TextRange::new(start, TextSize::new(start.raw + len.raw))
    }

    pub const fn empty(offset: TextSize) -> TextRange {
        TextRange {
            start: offset,
            end: offset,
        }
    }

    pub const fn start(self) -> TextSize {
        self.start
    }

    pub const fn end(self) -> TextSize {
        self.end
    }

    pub const fn len(self) -> TextSize {
        TextSize::new(self.end.raw - self.start.raw)
    }

    pub const fn is_empty(self) -> bool {
        self.start.raw == self.end.raw
    }
}

pub fn range_contains(range: TextRange, offset: TextSize) -> bool {
    offset >= range.start() && offset < range.end()
}

pub mod lsp {
    use super::{TextRange, TextSize};

    pub use lsp_types::{Position as LspPosition, Range as LspRange};

    pub fn to_lsp_position(offset: TextSize, text: &str) -> LspPosition {
        let mut line = 0u32;
        let mut col = 0u32;
        let target = offset.raw as usize;

        for (idx, ch) in text.char_indices() {
            if idx >= target {
                break;
            }

            if ch == '\n' {
                line += 1;
                col = 0;
                continue;
            }

            col += ch.len_utf16() as u32;
        }

        LspPosition::new(line, col)
    }

    pub fn from_lsp_position(position: LspPosition, text: &str) -> Option<TextSize> {
        let mut line = 0u32;
        let mut col = 0u32;

        for (idx, ch) in text.char_indices() {
            if line == position.line && col == position.character {
                return TextSize::try_from(idx).ok();
            }

            if ch == '\n' {
                line += 1;
                col = 0;
                continue;
            }

            let next_col = col + ch.len_utf16() as u32;
            if line == position.line && position.character < next_col {
                return None;
            }
            col = next_col;
        }

        if line == position.line && col == position.character {
            TextSize::try_from(text.len()).ok()
        } else {
            None
        }
    }

    pub fn to_lsp_range(range: TextRange, text: &str) -> LspRange {
        let start = to_lsp_position(range.start(), text);
        let end = to_lsp_position(range.end(), text);
        LspRange::new(start, end)
    }

    pub fn from_lsp_range(range: LspRange, text: &str) -> Option<TextRange> {
        let start = from_lsp_position(range.start, text)?;
        let end = from_lsp_position(range.end, text)?;
        Some(TextRange::new(start, end))
    }
}

#[cfg(test)]
mod tests {
    use super::{TextRange, TextSize, lsp};

    #[test]
    fn text_range_invariants_and_empty_ranges() {
        let start = TextSize::from(3);
        let end = TextSize::from(1);
        assert!(std::panic::catch_unwind(|| TextRange::new(start, end)).is_err());

        let empty = TextRange::empty(TextSize::from(2));
        assert_eq!(empty.start(), empty.end());
        assert!(empty.is_empty());
    }

    #[test]
    fn utf16_position_conversion_ascii() {
        let text = "abc\ndef";
        let offset = TextSize::from(4);
        let pos = lsp::to_lsp_position(offset, text);
        assert_eq!(pos.line, 1);
        assert_eq!(pos.character, 0);
        assert_eq!(lsp::from_lsp_position(pos, text), Some(offset));
    }

    #[test]
    fn utf16_position_conversion_non_ascii() {
        let text = "a😀b";
        let offset = TextSize::from(5);
        let pos = lsp::to_lsp_position(offset, text);
        assert_eq!(pos.line, 0);
        assert_eq!(pos.character, 3);
        assert_eq!(lsp::from_lsp_position(pos, text), Some(offset));
    }

    #[test]
    fn lsp_range_round_trip() {
        let text = "contract Foo {}\nfunction bar() {}";
        let start = TextSize::from(17);
        let end = TextSize::from(33);
        let range = TextRange::new(start, end);
        let lsp_range = lsp::to_lsp_range(range, text);
        let round_trip = lsp::from_lsp_range(lsp_range, text).expect("range");
        assert_eq!(round_trip, range);
    }
}
