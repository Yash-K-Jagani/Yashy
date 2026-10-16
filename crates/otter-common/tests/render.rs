//! Rendering tests for `SourceMap::render`.
//!
//! Every test here names the fixture bytes it depends on, because a caret that
//! is one column off is exactly the bug these tests exist to catch.

use otter_common::{
    Diagnostic, DiagnosticBag, FileId, Location, Severity, SourceMap, Span, EXIT_COMPILE_ERROR,
    EXIT_OK,
};

/// Byte offsets used by several tests below:
///
/// ```text
/// fn main() {\n      bytes  0..11, newline at 11
///     let x = 1;\n    bytes 12..26, newline at 26
/// }\n                bytes 27..28, newline at 28
/// ```
/// so byte 18 is the `t` of `let`: the fifth character of the line's content
/// after the four spaces of indent, which puts it at column 7 counting from 1.
/// (The brief writes `main.ot:2:5` for this offset; that count leaves out the
/// indent. The fixture is kept verbatim and the column is asserted from the
/// bytes, because a caret under the wrong character is the bug under test.)
const SAMPLE: &str = "fn main() {\n    let x = 1;\n}\n";

fn sample_map() -> (SourceMap, FileId) {
    let mut sm = SourceMap::new();
    let id = sm.add_file("main.ot", SAMPLE.to_string());
    (sm, id)
}

fn at(file: FileId, span: Span) -> Diagnostic {
    Diagnostic {
        severity: Severity::Error,
        message: "cannot find value `y` in this scope".to_string(),
        file,
        span,
        label: None,
    }
}

/// The line of rendered output at zero-based `index`, or a panic that prints
/// the whole rendering so a shape change is debuggable.
fn line(out: &str, index: usize) -> &str {
    match out.lines().nth(index) {
        Some(line) => line,
        None => panic!("rendered output has no line {index}: {out}"),
    }
}

#[test]
fn renders_caret_under_the_offending_column() {
    let (sm, f) = sample_map();
    let d = at(f, Span { start: 18, end: 19 });

    let out = sm.render(&d);

    assert!(out.contains("main.ot:2:7"), "got: {out}");
    assert!(
        out.contains("error: cannot find value `y` in this scope"),
        "got: {out}"
    );
    // Four lines, in order: location, message, source, caret.
    assert_eq!(
        out.lines().count(),
        4,
        "expected location, message, source and caret lines: {out}"
    );
    assert_eq!(line(&out, 0), "main.ot:2:7", "got: {out}");
    assert_eq!(
        line(&out, 1),
        "error: cannot find value `y` in this scope",
        "got: {out}"
    );
    assert_eq!(line(&out, 2), "    let x = 1;", "got: {out}");
    // The caret is six spaces in, so it sits under the `t` at column 7.
    assert_eq!(line(&out, 3), "      ^", "got: {out}");
    assert_eq!(sm.locate(f, d.span), Location { line: 2, col: 7 });
}

#[test]
fn tab_in_source_does_not_shift_the_caret() {
    // Same source, indented with one tab instead of four spaces. The tab is
    // one column, so the `x` is still at column 6 and the caret must land
    // there -- and the tab itself must survive into the printed source line.
    let mut sm = SourceMap::new();
    let f = sm.add_file("main.ot", "fn main() {\n\tlet x = 1;\n}\n".to_string());
    let d = at(f, Span { start: 17, end: 18 }); // the `x`

    let out = sm.render(&d);

    assert_eq!(line(&out, 0), "main.ot:2:6", "got: {out}");
    assert_eq!(
        line(&out, 2),
        "\tlet x = 1;",
        "the tab must be preserved in the source line: {out}"
    );
    // Counted in characters, so the caret is the sixth character of its line.
    assert_eq!(line(&out, 3), "\t    ^", "got: {out}");
    assert_eq!(sm.locate(f, d.span), Location { line: 2, col: 6 });

    // The control: the same source indented with a single space, which is also
    // one column, so the `x` lands on column 6 there too. If a tab were
    // counted as anything but one character these two would disagree -- and if
    // it were counted as zero, this test's own col assertion would be off.
    let mut spaced = SourceMap::new();
    let sf = spaced.add_file("main.ot", "fn main() {\n let x = 1;\n}\n".to_string());
    assert_eq!(
        spaced.locate(sf, Span { start: 17, end: 18 }),
        Location { line: 2, col: 6 },
        "a tab must count as exactly one column"
    );
}

#[test]
fn span_past_end_of_file_clamps_instead_of_panicking() {
    let (sm, f) = sample_map();
    // SAMPLE is 29 bytes long and ends with a newline, so offset 29 is the
    // empty line 4. Both halves of the span are far out of bounds.
    let d = at(f, Span { start: 5_000, end: 9_999 });

    let out = sm.render(&d);

    assert_eq!(line(&out, 0), "main.ot:4:1", "got: {out}");
    assert_eq!(line(&out, 1), "error: cannot find value `y` in this scope");
    assert_eq!(sm.locate(f, d.span), Location { line: 4, col: 1 });

    // A file with no trailing newline is the stronger case. A span whose end is
    // past the end of the file must be measured to the end of that file --
    // five characters from byte 5 -- rather than to a swallowed out-of-range
    // range, which would leave a single caret at the first character.
    let mut unterminated = SourceMap::new();
    let uf = unterminated.add_file("unterminated.ot", "let x = 1;".to_string());
    let out = unterminated.render(&at(uf, Span { start: 5, end: 5_000 }));
    assert_eq!(line(&out, 0), "unterminated.ot:1:6", "got: {out}");
    assert_eq!(line(&out, 2), "let x = 1;", "got: {out}");
    assert_eq!(line(&out, 3), "     ^^^^^", "got: {out}");

    // A span entirely past the end lands one column past the last character,
    // which is the end-of-file position, and still renders.
    let out = unterminated.render(&at(uf, Span { start: 5_000, end: 9_999 }));
    assert_eq!(line(&out, 0), "unterminated.ot:1:11", "got: {out}");
    assert_eq!(line(&out, 3), "          ^", "got: {out}");
}

#[test]
fn a_span_on_an_empty_file_renders_without_panicking() {
    let mut sm = SourceMap::new();
    let f = sm.add_file("empty.ot", String::new());

    for span in [Span { start: 0, end: 0 }, Span { start: 7, end: 9 }] {
        let out = sm.render(&at(f, span));
        assert_eq!(line(&out, 0), "empty.ot:1:1", "got: {out}");
        assert_eq!(line(&out, 2), "", "the source line is empty: {out}");
        assert_eq!(line(&out, 3), "^", "got: {out}");
    }
}

#[test]
fn a_reversed_span_does_not_panic_and_still_draws_one_caret() {
    let (sm, f) = sample_map();
    // end < start is malformed input the compiler must survive.
    let d = at(f, Span { start: 18, end: 3 });

    let out = sm.render(&d);

    assert_eq!(line(&out, 0), "main.ot:2:7", "got: {out}");
    assert_eq!(line(&out, 3), "      ^", "got: {out}");
}

#[test]
fn a_multicharacter_span_draws_one_caret_per_character() {
    let (sm, f) = sample_map();
    // `x = 1` is bytes 20..25.
    let d = at(f, Span { start: 20, end: 25 });

    let out = sm.render(&d);

    assert_eq!(line(&out, 3), "        ^^^^^", "got: {out}");
}

#[test]
fn a_span_running_past_the_line_stops_at_the_end_of_the_line() {
    // Bytes 24..30 cover `1;` and then the line break into line 3. The caret
    // run stops at the end of line 2 instead of running off it.
    let (sm, f) = sample_map();
    let d = at(f, Span { start: 24, end: 30 });

    let out = sm.render(&d);

    assert_eq!(line(&out, 0), "main.ot:2:13", "got: {out}");
    assert_eq!(line(&out, 3), "            ^^", "got: {out}");
}

#[test]
fn multibyte_characters_count_as_one_column() {
    // `é` is two bytes, so byte 6 is column 6 rather than column 7.
    let mut sm = SourceMap::new();
    let f = sm.add_file("unicode.ot", "let \u{e9}x = 1;\n".to_string());
    let d = at(f, Span { start: 6, end: 7 }); // the `x`

    let out = sm.render(&d);

    assert_eq!(line(&out, 0), "unicode.ot:1:6", "got: {out}");
    assert_eq!(line(&out, 2), "let \u{e9}x = 1;", "got: {out}");
    assert_eq!(line(&out, 3), "     ^", "got: {out}");
}

#[test]
fn a_caret_inside_a_multibyte_character_falls_back_to_its_start() {
    // Offset 5 is the second byte of the `é`. There is no column there, so the
    // caret goes to the character that byte belongs to rather than slicing
    // through UTF-8 and panicking.
    let mut sm = SourceMap::new();
    let f = sm.add_file("unicode.ot", "let \u{e9}x = 1;\n".to_string());
    let d = at(f, Span { start: 5, end: 6 });

    let out = sm.render(&d);

    assert_eq!(line(&out, 0), "unicode.ot:1:5", "got: {out}");
    assert_eq!(line(&out, 3), "    ^", "got: {out}");
}

#[test]
fn a_carriage_return_before_a_newline_is_not_rendered() {
    // Windows line endings: the `\r` belongs to the terminator, not the line.
    let mut sm = SourceMap::new();
    let f = sm.add_file("crlf.ot", "fn main() {\r\n\tlet x = 1;\r\n}\r\n".to_string());
    let d = at(f, Span { start: 18, end: 19 }); // the `x`, line 2 starts at 13

    let out = sm.render(&d);

    assert_eq!(line(&out, 0), "crlf.ot:2:6", "got: {out}");
    assert_eq!(line(&out, 2), "\tlet x = 1;", "got: {out}");
    assert!(!out.contains('\r'), "no carriage return may be rendered: {out}");
}

#[test]
fn locate_reports_one_based_positions_across_lines() {
    let (sm, f) = sample_map();
    let at_ = |offset| sm.locate(f, Span { start: offset, end: offset });

    assert_eq!(at_(0), Location { line: 1, col: 1 });
    assert_eq!(at_(10), Location { line: 1, col: 11 });
    assert_eq!(at_(11), Location { line: 1, col: 12 }, "the newline is the last column of line 1");
    assert_eq!(at_(12), Location { line: 2, col: 1 });
    assert_eq!(at_(29), Location { line: 4, col: 1 }, "past the final newline");
}

#[test]
fn locate_reports_line_one_for_a_file_it_does_not_have() {
    let (sm, _) = sample_map();
    let bogus = FileId::from_index(9_999);

    assert_eq!(sm.locate(bogus, Span { start: 0, end: 1 }), Location { line: 1, col: 1 });
    assert_eq!(sm.file(bogus), "");
    assert_eq!(sm.path(bogus), "<unknown>");

    let out = sm.render(&at(bogus, Span { start: 0, end: 1 }));
    assert_eq!(line(&out, 0), "<unknown>:1:1", "got: {out}");
}

#[test]
fn a_warning_renders_as_a_warning() {
    let (sm, f) = sample_map();
    let mut d = at(f, Span { start: 18, end: 19 });
    d.severity = Severity::Warning;

    let out = sm.render(&d);

    assert_eq!(
        line(&out, 1),
        "warning: cannot find value `y` in this scope",
        "got: {out}"
    );
}

#[test]
fn a_label_is_rendered_after_the_caret() {
    let (sm, f) = sample_map();
    let mut d = at(f, Span { start: 18, end: 19 });
    d.label = Some("declared here".to_string());

    let out = sm.render(&d);

    assert_eq!(line(&out, 3), "      ^ declared here", "got: {out}");
}

#[test]
fn a_file_can_be_read_back_by_id() {
    let (sm, f) = sample_map();

    assert_eq!(sm.file(f), SAMPLE, "the text must come back unchanged");
    assert_eq!(sm.path(f), "main.ot");
    assert_eq!(sm.len(), 1);
    assert!(!sm.is_empty());

    let mut empty = SourceMap::new();
    assert!(empty.is_empty());
    let second = empty.add_file("second.ot", "fn main() {}\n".to_string());
    let third = empty.add_file("third.ot", "let y = 2;\n".to_string());
    assert_ne!(second, third, "ids within one map must be distinct");
    assert_eq!(empty.len(), 2);
    assert_eq!(empty.file(second), "fn main() {}\n");
    assert_eq!(empty.file(third), "let y = 2;\n");

    // A `FileId` only means something next to the map that issued it: `f` is
    // index 0 here too, so it names this map's first file rather than nothing.
    assert_eq!(empty.file(f), "fn main() {}\n", "ids are per-map indices");
    assert_eq!(f.index(), second.index());
}

#[test]
fn a_bag_keeps_diagnostics_in_push_order() {
    let (_, f) = sample_map();
    let mut bag = DiagnosticBag::new();
    assert!(bag.is_empty());

    let first = at(f, Span { start: 18, end: 19 });
    let second = at(f, Span { start: 0, end: 1 });
    bag.push(first.clone());
    bag.push(second.clone());

    assert!(!bag.is_empty());
    assert_eq!(bag.len(), 2);
    assert_eq!(bag.iter().count(), 2);
    assert_eq!(bag.error_count(), 2);

    let collected = bag.into_vec();
    assert_eq!(collected, vec![first, second]);
}

#[test]
fn a_bag_counts_only_errors_towards_the_exit_code() {
    let (_, f) = sample_map();
    let mut warning = at(f, Span { start: 0, end: 1 });
    warning.severity = Severity::Warning;
    let mut bag = DiagnosticBag::new();
    bag.push(warning);

    assert_eq!(bag.len(), 1);
    assert_eq!(bag.error_count(), 0);

    bag.push(at(f, Span { start: 18, end: 19 }));
    assert_eq!(bag.error_count(), 1);
}

#[test]
fn exit_codes_distinguish_success_from_a_compile_error() {
    assert_eq!(EXIT_OK, 0);
    assert_ne!(EXIT_OK, EXIT_COMPILE_ERROR);
}