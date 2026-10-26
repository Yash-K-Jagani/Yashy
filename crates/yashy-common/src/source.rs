//! The [`SourceMap`]: every file the compiler has read, and the byte-offset to
//! line/column mapping that lets a diagnostic point at something a human can
//! find.
//!
//! The invariant of this module is that no user input can make it panic. Every
//! offset is clamped into the file, every lookup goes through `get`/`Option`,
//! and every conversion to `u32` saturates rather than wrapping. A compiler
//! that panics on a bad byte offset is worse than useless, because the input
//! that triggered it is exactly the input the user needs diagnosed.

use crate::diagnostic::Diagnostic;
use crate::span::Span;

/// Which file a [`Span`] or a [`Diagnostic`] refers to.
///
/// This is an index into a [`SourceMap`], not a path: a source map is built up
/// as the driver opens files, and the index is all a span needs.
///
/// # A `FileId` is only meaningful with the map that issued it
///
/// Each map numbers its own files from zero, so an id taken from one map and
/// handed to another **resolves to whichever file that index happens to name
/// there -- silently, and almost certainly the wrong one.** That produces a
/// caret under an unrelated source line, which is worse than a clean "unknown
/// source": the reader is shown confident, wrong information. Only an index no
/// map has degrades to "no source available".
///
/// So a `FileId` must never outlive, or be mixed between, its [`SourceMap`]. If
/// that ever comes up -- ids cached across a reload, or a diagnostic deferred
/// past the map that produced it -- the fix belongs here, by adding a generation
/// counter to the id or a `debug_assert` that catches the mix-up, not at the
/// call sites. `tests/render.rs::a_file_can_be_read_back_by_id` pins the current
/// aliasing behaviour on purpose, so that such a change is a deliberate,
/// visible diff rather than a silent one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileId(u32);

impl FileId {
    /// Builds a `FileId` from a raw index.
    ///
    /// Only the indices handed out by [`SourceMap::add_file`] name a real file,
    /// and only in the map that handed them out. An index no map has is not an
    /// error either: every lookup on it degrades to "no source available"
    /// instead of panicking.
    pub fn from_index(index: u32) -> FileId {
        FileId(index)
    }

    /// The raw index this id was built from.
    pub fn index(self) -> u32 {
        self.0
    }
}

/// A one-based position in a source file.
///
/// Columns count characters, not bytes, so a multi-byte character advances the
/// column by one and a tab is one column as well (see [`SourceMap::render`] for
/// why the rendered caret keeps the tab).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Location {
    /// The one-based line number.
    pub line: u32,
    /// The one-based column number.
    pub col: u32,
}

/// The files the compiler has been given, with a newline index for each.
///
/// `add_file` records where every line begins, so turning a byte offset into a
/// [`Location`] is a binary search plus a count over one line rather than a walk
/// over the whole file.
#[derive(Debug, Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

/// One file, with the byte offset of the start of each of its lines.
#[derive(Debug)]
struct SourceFile {
    path: String,
    text: String,
    /// Byte offsets of the first byte of each line, ascending.
    ///
    /// Always starts with `0`. When the file ends with a newline it also ends
    /// with the offset just past that newline, which names the empty final
    /// line -- so the line count is one more than the newline count.
    line_starts: Vec<usize>,
}

impl SourceFile {
    fn new(path: String, text: String) -> SourceFile {
        let line_starts = collect_line_starts(&text);
        SourceFile { path, text, line_starts }
    }

    /// Turns a byte offset into a one-based line and column.
    ///
    /// An offset past the end of the file is clamped to its length, and an
    /// offset that lands inside a multi-byte character is rounded down to that
    /// character's start, so the result always names a position that exists.
    fn locate(&self, offset: usize) -> Location {
        let offset = floor_char_boundary(&self.text, offset);
        let line_index = match self.line_starts.binary_search(&offset) {
            Ok(index) => index,
            // `line_starts[0]` is 0 and `offset` is 0 or more, so `insert_at`
            // is at least 1 here; `saturating_sub` keeps that from being a
            // panic if the invariant ever changes.
            Err(insert_at) => insert_at.saturating_sub(1),
        };
        let line_start = self.line_starts.get(line_index).copied().unwrap_or(0);
        // A `\r` is part of the line ending, not of the line, so an offset
        // pointing at it -- or at the `\n` after it -- is reported as one
        // column past the last *visible* character. Counting it instead would
        // put the caret padding one column beyond the source line `render`
        // prints, which is exactly the line the reader is looking at.
        let visible_end = match self.line_bounds(line_index) {
            Some((_, end)) => offset.min(end),
            None => offset,
        };
        let preceding = self.text.get(line_start..visible_end).map(|s| s.chars().count()).unwrap_or(0);
        Location { line: saturating_u32(line_index.saturating_add(1)), col: saturating_u32(preceding.saturating_add(1)) }
    }

    /// The text of a one-based line, without its line terminator.
    ///
    /// Every `\r` at the end is a terminator, not content -- on every platform
    /// that uses CRLF, and also in the `\r\r\n` a file picks up from a tool that
    /// has already doubled them -- so all of them are dropped rather than
    /// printed.
    fn line_text(&self, line: u32) -> &str {
        // `line_starts` is indexed from zero while locations are one-based, so
        // the line number is shifted down here. A location of zero -- which no
        // `locate` produces, but which a caller could pass -- has no line, and
        // renders as an empty line.
        let bounds = usize::try_from(line)
            .ok()
            .and_then(|line| line.checked_sub(1))
            .and_then(|index| self.line_bounds(index));
        match bounds {
            Some((start, end)) => self.text.get(start..end).unwrap_or(""),
            None => "",
        }
    }

    /// The byte range of a zero-based line, excluding its terminator.
    ///
    /// Returns `None` for a line index the file does not have, which is how an
    /// out-of-range location renders as an empty line instead of panicking.
    fn line_bounds(&self, line_index: usize) -> Option<(usize, usize)> {
        let start = *self.line_starts.get(line_index)?;
        let next = self.line_starts.get(line_index + 1).copied();
        // For every line but the last, the stored offset points just past this
        // line's `\n`, so step back over it.
        let mut end = match next {
            Some(next) => next.checked_sub(1)?,
            None => self.text.len(),
        };
        // Then step back over every `\r` before it. One is the ordinary CRLF;
        // more than one happens when a tool has already doubled them, and they
        // are just as much terminator as the first. `end > start` keeps the
        // subtraction in bounds on an empty line.
        while end > start && self.text.as_bytes().get(end - 1) == Some(&b'\r') {
            end -= 1;
        }
        Some((start, end))
    }
}

impl SourceMap {
    /// An empty source map.
    pub fn new() -> SourceMap {
        SourceMap { files: Vec::new() }
    }

    /// Stores a file and returns the id that names it.
    pub fn add_file(&mut self, path: impl Into<String>, text: String) -> FileId {
        let index = match u32::try_from(self.files.len()) {
            Ok(index) => index,
            // More than four billion files cannot fit in memory, so this is
            // unreachable in practice. Returning an id beats panicking: it
            // resolves to a file that does not exist, which renders as
            // "no source available".
            Err(_) => return FileId(u32::MAX),
        };
        self.files.push(SourceFile::new(path.into(), text));
        FileId(index)
    }

    /// The text of a file, or `""` if the id names no file in this map.
    pub fn file(&self, id: FileId) -> &str {
        self.get(id).map(|file| file.text.as_str()).unwrap_or("")
    }

    /// The path a file was added under, or `"<unknown>"`.
    pub fn path(&self, id: FileId) -> &str {
        self.get(id).map(|file| file.path.as_str()).unwrap_or("<unknown>")
    }

    /// The one-based line and column of a span's start.
    ///
    /// A span pointing at or past the end of the file resolves to the end of
    /// that file rather than failing.
    pub fn locate(&self, id: FileId, span: Span) -> Location {
        match self.get(id) {
            Some(file) => file.locate(offset_of(span.start)),
            None => Location { line: 1, col: 1 },
        }
    }

    /// The source text a span covers, with both ends clamped into the file.
    ///
    /// This is how the compiler quotes a program back to itself: a failed
    /// `assert` reports the expression that was written, which means slicing the
    /// text the span names. The ends are clamped to character boundaries and to
    /// the file's length, and a span that names nothing -- a synthetic one, an
    /// id no map has, a reversed range -- yields `""` rather than a panic, which
    /// is the same rule the rest of this module follows.
    pub fn snippet(&self, id: FileId, span: Span) -> &str {
        let file = match self.get(id) {
            Some(file) => file,
            None => return "",
        };
        let start = floor_char_boundary(&file.text, offset_of(span.start));
        let end = floor_char_boundary(&file.text, offset_of(span.end).min(file.text.len()));
        if end < start {
            return "";
        }
        file.text.get(start..end).unwrap_or("")
    }

    /// Renders a diagnostic the way a compiler should print it.
    ///
    /// The output starts with four lines: `path:line:col`, the severity and
    /// message, the source line with its whitespace untouched, and a caret run
    /// under the offending span with an optional label after it. Any notes
    /// follow, one `note: ...` line each, and a coded diagnostic ends with a
    /// `docs:` line naming its catalog page.
    ///
    /// Columns count characters, so the caret line reproduces the source
    /// prefix character for character -- with tabs kept as tabs, so the caret
    /// also lines up in a terminal that expands them.
    pub fn render(&self, d: &Diagnostic) -> String {
        let file = match self.get(d.file) {
            Some(file) => file,
            // An id no file in this map has -- see the warning on [`FileId`]
            // about ids from another map. Report the message anyway: losing the
            // diagnostic would hide the very problem that produced the bad id.
            None => {
                let mut out = format!(
                    "{}:1:1\n{}: {}\n<source line unavailable>\n",
                    self.path(d.file),
                    severity_label(d),
                    d.message
                );
                push_docs_line(&mut out, d);
                return out;
            }
        };

        let located = file.locate(offset_of(d.span.start));
        let line_text = file.line_text(located.line);
        let prefix = line_text.chars().take(offset_of(located.col.saturating_sub(1)));
        // A tab stays a tab so the caret stays aligned when it is displayed; any
        // other character becomes a single space, which keeps one column per
        // character for wide and multi-byte characters alike.
        let padding: String = prefix.map(|c| if c == '\t' { '\t' } else { ' ' }).collect();
        let carets = "^".repeat(caret_count(file, line_text, located, d.span));

        let mut out = String::new();
        out.push_str(&format!(
            "{}:{}:{}\n",
            file.path,
            located.line,
            located.col
        ));
        out.push_str(&format!("{}: {}\n", severity_label(d), d.message));
        out.push_str(line_text);
        out.push('\n');
        out.push_str(&padding);
        out.push_str(&carets);
        if let Some(label) = d.label.as_deref() {
            out.push(' ');
            out.push_str(label);
        }
        out.push('\n');
        for note in &d.notes {
            out.push_str("note: ");
            out.push_str(note);
            out.push('\n');
        }
        push_docs_line(&mut out, d);
        out
    }

    fn get(&self, id: FileId) -> Option<&SourceFile> {
        usize::try_from(id.index()).ok().and_then(|index| self.files.get(index))
    }
}

/// Appends the catalog link for a coded diagnostic, if it has one.
///
/// The link is the last line so a reader always finds it in the same place,
/// after the notes that explain the mistake. An uncoded diagnostic prints
/// nothing, because there is no page to point at yet.
fn push_docs_line(out: &mut String, d: &Diagnostic) {
    if let Some(url) = d.docs_url() {
        out.push_str("  docs: ");
        out.push_str(&url);
        out.push('\n');
    }
}

/// The word before the message: `error`, or `error[E0100]` when the
/// diagnostic carries a catalog code.
fn severity_label(d: &Diagnostic) -> String {
    match d.code {
        Some(code) => format!("{}[{code}]", d.severity.as_str()),
        None => d.severity.as_str().to_string(),
    }
}

/// How many carets to draw for a span, at least one and never past the line.
///
/// The span is measured in characters so the run matches what is printed, and
/// is clipped to the end of the line so a span that runs onto the next line
/// still renders sensibly here. A reversed span (`end < start`) measures as an
/// empty range, which still leaves the one caret that marks the position.
fn caret_count(file: &SourceFile, line_text: &str, at: Location, span: Span) -> usize {
    let start = floor_char_boundary(&file.text, offset_of(span.start));
    let end = floor_char_boundary(
        &file.text,
        offset_of(span.end).min(file.text.len()),
    );
    let spanned = file.text.get(start..end).map(|s| s.chars().count()).unwrap_or(0);
    let already_past = usize::try_from(at.col.saturating_sub(1)).unwrap_or(usize::MAX);
    // The `.max(1)` is load-bearing: a span pointing at the end of the file
    // leaves `at.col` one past the last character, so the subtraction yields 0,
    // and `clamp(1, 0)` below *panics* when min exceeds max. Do not drop the
    // `.max(1)` or move it below the `clamp` without a proof that `available`
    // can never be zero.
    let available = line_text.chars().count().saturating_sub(already_past).max(1);
    spanned.clamp(1, available)
}

/// The byte offset of every line start in `text`.
fn collect_line_starts(text: &str) -> Vec<usize> {
    let mut starts = Vec::with_capacity(text.len() / 24 + 1);
    starts.push(0);
    for (offset, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(offset.saturating_add(1));
        }
    }
    starts
}

/// Clamps an offset to a character boundary inside `text`.
///
/// A span can name any byte, including one in the middle of a multi-byte
/// character. There is no column there, so the position rounds down to the
/// start of the character that byte belongs to, which keeps later slicing on a
/// boundary.
fn floor_char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn offset_of(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// Narrows a `usize` to the `u32` a [`Location`] holds, saturating.
fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
