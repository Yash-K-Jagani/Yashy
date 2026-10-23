//! `Span`: a half-open byte range into a source file.

/// A half-open byte range `[start, end)` into a source file.
///
/// The `Default` value (`start == end == 0`) is the span of a synthetic node:
/// one the compiler invented rather than read from source, so it has no caret to
/// draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    /// The span used for nodes that did not come from source text.
    pub const SYNTHETIC: Span = Span { start: 0, end: 0 };

    /// A span covering `[start, end)`.
    pub fn new(start: u32, end: u32) -> Span {
        Span { start, end }
    }

    /// True for the synthetic span, which renders no caret.
    pub fn is_synthetic(&self) -> bool {
        *self == Span::SYNTHETIC
    }
}

#[cfg(test)]
mod tests {
    use super::Span;

    #[test]
    fn default_is_the_synthetic_span() {
        let span = Span::default();
        assert_eq!(span.start, 0);
        assert_eq!(span.end, 0);
        assert!(span.is_synthetic());
    }

    #[test]
    fn new_keeps_the_byte_range() {
        let span = Span::new(3, 7);
        assert_eq!(span, Span { start: 3, end: 7 });
        assert!(!span.is_synthetic());
    }
}
