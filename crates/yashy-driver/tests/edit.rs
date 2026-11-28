//! Structured edits: hash-checked, all-or-nothing, and re-verified.

mod common;

use std::path::Path;

use yashy_common::Span;
use yashy_driver::edit::{
    apply_patch, file_hash, rename_tokens, tree_hash, Edit, EditError, Patch,
};

fn ok(_path: &Path, _text: &str) -> Result<(), Vec<String>> {
    Ok(())
}

fn rejects(_path: &Path, _text: &str) -> Result<(), Vec<String>> {
    Err(vec!["does not check".to_string()])
}

fn patch(dir: &Path, text: &str, from: usize, to: usize, replacement: &str) -> Patch {
    let file = dir.join("main.ya");
    let source = std::fs::read_to_string(&file).unwrap_or_default();
    Patch {
        edits: vec![Edit {
            file: "main.ya".to_string(),
            span: Span::new(from as u32, to as u32),
            expected_hash: file_hash(&file).unwrap(),
            replacement: replacement.to_string(),
            rationale: "test edit".to_string(),
        }],
        expected_tree_hash: tree_hash(dir).unwrap(),
    }
}

#[test]
fn an_edit_is_applied_and_the_file_hash_changes() {
    let dir = common::work_dir("edit-apply");
    common::write(&dir, "main.ya", "fn main() { print(1); }\n");
    let source = std::fs::read_to_string(dir.join("main.ya")).unwrap();
    let at = source.find('1').unwrap();
    let before = file_hash(&dir.join("main.ya")).unwrap();
    let applied = apply_patch(&dir, &patch(&dir, &source, at, at + 1, "2"), ok).unwrap();
    assert_eq!(applied, vec!["main.ya".to_string()]);
    let after = std::fs::read_to_string(dir.join("main.ya")).unwrap();
    assert!(after.contains("print(2)"), "{after}");
    assert_ne!(before, file_hash(&dir.join("main.ya")).unwrap());
}

#[test]
fn every_applied_edit_is_recorded_and_a_rolled_back_one_is_not() {
    let dir = common::work_dir("edit-provenance");
    common::write(&dir, "main.ya", "fn main() { print(1); }\n");
    let source = std::fs::read_to_string(dir.join("main.ya")).unwrap();
    let at = source.find('1').unwrap();
    apply_patch(&dir, &patch(&dir, &source, at, at + 1, "2"), ok).unwrap();

    let log = yashy_driver::edit::read_provenance(&dir).unwrap();
    assert!(log.contains("main.ya"), "{log}");
    assert!(log.contains("test edit"), "{log}");
    assert!(log.contains("\"tree_hash\""), "{log}");
    assert!(log.contains("\"hash\""), "{log}");

    // A patch that fails its re-check is rolled back and leaves no trace.
    let source = std::fs::read_to_string(dir.join("main.ya")).unwrap();
    let at = source.find('2').unwrap();
    let result = apply_patch(&dir, &patch(&dir, &source, at, at + 1, "3"), rejects);
    assert!(matches!(result, Err(EditError::CheckFailed(_))));
    assert_eq!(yashy_driver::edit::read_provenance(&dir).unwrap(), log);
}

#[test]
fn a_stale_hash_leaves_the_file_byte_identical() {
    let dir = common::work_dir("edit-stale");
    common::write(&dir, "main.ya", "fn main() { print(1); }\n");
    let mut edit = patch(&dir, "", 0, 0, "x").edits.remove(0);
    edit.expected_hash = "stale".to_string();
    let before = std::fs::read(&dir.join("main.ya")).unwrap();
    let result = apply_patch(
        &dir,
        &Patch {
            edits: vec![edit],
            expected_tree_hash: tree_hash(&dir).unwrap(),
        },
        ok,
    );
    assert_eq!(result, Err(EditError::StaleHash("main.ya".to_string())));
    assert_eq!(std::fs::read(&dir.join("main.ya")).unwrap(), before);
}

#[test]
fn overlapping_edits_are_refused_before_any_write() {
    let dir = common::work_dir("edit-overlap");
    common::write(&dir, "main.ya", "fn main() { print(1); }\n");
    let source = std::fs::read_to_string(dir.join("main.ya")).unwrap();
    let at = source.find('1').unwrap();
    let one = patch(&dir, &source, at, at + 1, "2").edits.remove(0);
    let mut two = one.clone();
    two.replacement = "3".to_string();
    let before = std::fs::read(&dir.join("main.ya")).unwrap();
    let result = apply_patch(
        &dir,
        &Patch {
            expected_tree_hash: tree_hash(&dir).unwrap(),
            edits: vec![one, two],
        },
        ok,
    );
    assert!(matches!(result, Err(EditError::Overlapping(_))));
    assert_eq!(std::fs::read(&dir.join("main.ya")).unwrap(), before);
}

#[test]
fn an_outside_path_is_refused() {
    let dir = common::work_dir("edit-outside");
    common::write(&dir, "main.ya", "fn main() {}\n");
    let source = std::fs::read_to_string(dir.join("main.ya")).unwrap();
    let mut one = patch(&dir, &source, 0, 0, "x").edits.remove(0);
    one.file = "../secret.ya".to_string();
    let result = apply_patch(
        &dir,
        &Patch {
            expected_tree_hash: tree_hash(&dir).unwrap(),
            edits: vec![one],
        },
        ok,
    );
    assert!(matches!(result, Err(EditError::OutsideWorkspace(_))));
}

#[test]
fn a_patch_that_does_not_check_is_rolled_back() {
    let dir = common::work_dir("edit-rollback");
    common::write(&dir, "main.ya", "fn main() { print(1); }\n");
    let source = std::fs::read_to_string(dir.join("main.ya")).unwrap();
    let at = source.find('1').unwrap();
    let before = std::fs::read(&dir.join("main.ya")).unwrap();
    let result = apply_patch(&dir, &patch(&dir, &source, at, at + 1, "2"), rejects);
    assert!(matches!(result, Err(EditError::CheckFailed(_))));
    assert_eq!(std::fs::read(&dir.join("main.ya")).unwrap(), before);
}

#[test]
fn rename_uses_tokens_not_text() {
    let text = "fn old(int old) {\n    let copy = \"old\";\n    print(old);\n}\n";
    let edits = rename_tokens(text, "old", "new", "rename").unwrap();
    assert_eq!(edits.len(), 3, "the string literal must not be renamed");
    let mut patched = text.to_string();
    for edit in edits.iter().rev() {
        let start = edit.span.start as usize;
        let end = edit.span.end as usize;
        patched.replace_range(start..end, &edit.replacement);
    }
    assert!(patched.contains("fn new(int new)"), "{patched}");
    assert!(patched.contains("\"old\""), "{patched}");
}
