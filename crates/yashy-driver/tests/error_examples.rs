//! The examples on an error page are the page's evidence.
//!
//! A transcript repeats the source line the diagnostic is about, under a
//! `path:line:column` header. If the page's snippet and its transcript disagree
//! about that line, the reader is told to look somewhere the compiler would not
//! name. These tests read every page and check the contract without running the
//! compiler, so they hold on a host that refuses to execute a built binary.
//!
//! Three facts are checked:
//!
//! 1. Every transcript that follows a `yashy` block echoes the snippet's line.
//! 2. `docs/errors.md`'s table is exactly the registry, in registry order.
//! 3. Every `docs/errors/<code>.md` belongs to a code, and every code has one.

use std::collections::BTreeSet;
use std::path::Path;

use yashy_common::codes;

/// One fenced block: its info string and its body.
struct Block<'a> {
    kind: &'a str,
    body: Vec<&'a str>,
}

/// The fenced blocks of a page, in order.
///
/// A fence opens with ```` ``` ```` plus an info string and closes with a line
/// that starts with ```` ``` ````. An opening fence with no info string is kept
/// too, so the block indices stay stable; only `yashy` and `text` are read.
fn blocks(page: &str) -> Vec<Block<'_>> {
    let mut blocks = Vec::new();
    let mut lines = page.lines();
    while let Some(line) = lines.next() {
        let opening = line.trim_start();
        let Some(kind) = opening.strip_prefix("```") else {
            continue;
        };
        let mut body = Vec::new();
        for inner in lines.by_ref() {
            if inner.trim_start().starts_with("```") {
                break;
            }
            body.push(inner);
        }
        blocks.push(Block {
            kind: kind.trim(),
            body,
        });
    }
    blocks
}

/// `path:line:column`, the first line of a diagnostic transcript.
fn location(line: &str) -> Option<(&str, usize)> {
    let mut parts = line.rsplitn(3, ':');
    let column = parts.next()?.trim().parse::<usize>().ok()?;
    let row = parts.next()?.trim().parse::<usize>().ok()?;
    let file = parts.next()?.trim();
    if file.is_empty() || file.contains(' ') || row == 0 || column == 0 {
        return None;
    }
    Some((file, row))
}

/// A page's multi-file example labels each block with `// name.ya`.
///
/// The label is not source: a reader who copies one block into `name.ya` starts
/// at the line after it, which is the numbering the transcript uses too.
fn is_file_label(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("//") && trimmed.ends_with(".ya") && !trimmed[2..].trim().contains(' ')
}

/// The line a transcript points at and the source it echoes for it.
///
/// The transcript shape is a location, a `error[CODE]:` (or `warning`) line,
/// then the echoed source, then a caret line. A transcript with no echoed source
/// (a package-level diagnostic with only a location and a message) contributes
/// nothing to check.
fn evidence(transcript: &[&str]) -> Option<(usize, Vec<String>)> {
    let at = transcript
        .iter()
        .position(|line| location(line).is_some())?;
    let row = location(transcript[at])?.1;
    let diagnostic = transcript[at..].iter().position(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("error") || trimmed.starts_with("warning")
    })? + at;
    let mut source = Vec::new();
    for line in &transcript[diagnostic + 1..] {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('^') {
            break;
        }
        if trimmed.starts_with("docs:")
            || trimmed.starts_with('=')
            || trimmed.starts_with('|')
            || trimmed.starts_with("note:")
            || trimmed.starts_with("label:")
            || trimmed.starts_with("did you mean")
            || trimmed.starts_with("the first instantiation")
        {
            break;
        }
        source.push(trimmed.to_string());
    }
    if source.is_empty() {
        return None;
    }
    Some((row, source))
}

#[test]
fn every_transcript_points_at_the_line_it_echoes() {
    let mut checked = 0;
    let mut failures = Vec::new();
    for entry in codes::CODES {
        let blocks = blocks(entry.page);
        let mut last_example = None;
        for (index, block) in blocks.iter().enumerate() {
            match block.kind {
                "yashy" => last_example = Some(index),
                "text" => {
                    let Some((row, source)) = evidence(&block.body) else {
                        continue;
                    };
                    let Some(example) = last_example else {
                        continue;
                    };
                    let mut snippet = blocks[example].body.clone();
                    if snippet.first().is_some_and(|line| is_file_label(line)) {
                        snippet.remove(0);
                    }
                    let echoed = source.last().expect("source is non-empty");
                    checked += 1;
                    match snippet.get(row - 1).map(|line| line.trim()) {
                        Some(text) if text == echoed => {}
                        Some(text) => failures.push(format!(
                            "{}: `{}:{row}` holds `{text}`, but the transcript echoes `{echoed}`",
                            entry.id, entry.id
                        )),
                        None => failures.push(format!(
                            "{}: the transcript names line {row}, but the example has {} line(s)",
                            entry.id,
                            snippet.len()
                        )),
                    }
                }
                _ => {}
            }
        }
    }
    assert!(checked > 0, "no page had a transcript to check");
    assert!(
        failures.is_empty(),
        "error-page transcripts disagree with their snippets:\n{}",
        failures.join("\n")
    );
}

#[test]
fn the_catalog_table_is_generated_from_the_registry() {
    let document = include_str!("../../../docs/errors.md");
    let expected: Vec<String> = codes::CODES
        .iter()
        .map(|entry| {
            format!(
                "| [{}](errors/{}.md) | {} |",
                entry.id, entry.id, entry.title
            )
        })
        .collect();
    let mut lines = document.lines();
    lines
        .find(|line| line.trim() == "| Code | Meaning |")
        .expect("docs/errors.md has a `| Code | Meaning |` table");
    assert!(
        lines.next().is_some_and(|line| line.trim() == "|---|---|"),
        "the catalog table has its separator"
    );
    let mut actual = Vec::new();
    for line in lines {
        if !line.starts_with('|') {
            break;
        }
        actual.push(line.to_string());
    }
    assert_eq!(
        expected, actual,
        "docs/errors.md's table is stale: render it from the registry (id, link, title)"
    );
}

#[test]
fn every_page_belongs_to_a_code() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/errors");
    let known: BTreeSet<&str> = codes::CODES.iter().map(|entry| entry.id).collect();
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&directory).expect("docs/errors is readable") {
        let path = entry.expect("an entry is readable").path();
        if path
            .extension()
            .is_some_and(|extension| extension == std::ffi::OsStr::new("md"))
        {
            names.push(
                path.file_stem()
                    .expect("a markdown file has a stem")
                    .to_string_lossy()
                    .to_string(),
            );
        }
    }
    names.sort();
    let orphans: Vec<&String> = names
        .iter()
        .filter(|name| !known.contains(name.as_str()))
        .collect();
    assert!(
        orphans.is_empty(),
        "these pages name no code, so `explain` cannot reach them: {orphans:?}"
    );
    for entry in codes::CODES {
        assert!(
            names.iter().any(|name| name == entry.id),
            "{} has a catalog entry but no page",
            entry.id
        );
    }
}
