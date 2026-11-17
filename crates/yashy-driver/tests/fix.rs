//! One edit model: a diagnostic carries a byte span and a replacement, and
//! applying it changes exactly that span.

mod common;

use common::{work_dir, write, yashyc};
use std::fs;
use std::process::Command;
use yashy_common::{Applicability, Span, Suggestion};
use yashy_driver::fix::{apply_suggestions, apply_suggestions_checked, source_hash};

#[test]
fn check_json_carries_suggestions_for_a_removed_keyword() {
    let dir = work_dir("fix-json");
    write(&dir, "main.ya", "fn main() { var int x = 1; }\n");
    let output = Command::new(yashyc())
        .args(["check", "--json", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"suggestions\""), "{stdout}");
    assert!(stdout.contains("\"replacement\": \"let\""), "{stdout}");
    assert!(
        stdout.contains("\"applicability\": \"machine-applicable\""),
        "{stdout}"
    );
}

#[test]
fn check_json_offers_a_missing_move_and_fix_applies_it() {
    let dir = work_dir("fix-move");
    write(
        &dir,
        "main.ya",
        "fn main() {\n    let buf = alloc(16);\n    let other = buf;\n    other.free();\n}\n",
    );
    let output = Command::new(yashyc())
        .args(["check", "--json", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"replacement\": \"move buf\""), "{stdout}");
    assert!(
        stdout.contains("\"applicability\": \"machine-applicable\""),
        "{stdout}"
    );

    let fix = Command::new(yashyc())
        .args(["fix", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    assert!(
        fix.status.success(),
        "{}",
        String::from_utf8_lossy(&fix.stderr)
    );
    let fixed = fs::read_to_string(dir.join("main.ya")).expect("the fixed file");
    assert!(fixed.contains("let other = move buf;"), "{fixed}");

    let check = Command::new(yashyc())
        .args(["check", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    assert!(
        check.status.success(),
        "a second check must be clean:\n{}",
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn check_json_offers_the_argument_swap_as_one_fix() {
    let dir = work_dir("fix-order");
    write(
        &dir,
        "main.ya",
        "fn pair(int a, string b) -> int {\n    return a;\n}\n\nfn main() {\n    \
         const v = pair(\"x\", 1);\n    print(v);\n}\n",
    );
    let output = Command::new(yashyc())
        .args(["check", "--json", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("are out of order"), "{stdout}");
    // Both positions are rewritten; neither edit alone fixes the call.
    assert_eq!(stdout.matches("\"suggestions\"").count(), 1, "{stdout}");
    assert!(stdout.contains("\"replacement\": \"1\""), "{stdout}");
    assert!(
        stdout.contains("\"replacement\": \"\\\"x\\\"\""),
        "{stdout}"
    );

    let fix = Command::new(yashyc())
        .args(["fix", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    assert!(
        fix.status.success(),
        "{}",
        String::from_utf8_lossy(&fix.stderr)
    );
    let fixed = fs::read_to_string(dir.join("main.ya")).expect("the fixed file");
    assert!(fixed.contains("pair(1, \"x\")"), "{fixed}");

    let check = Command::new(yashyc())
        .args(["check", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    assert!(
        check.status.success(),
        "a second check must be clean:\n{}",
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn fix_rewrites_the_file_and_a_second_check_is_clean() {
    let dir = work_dir("fix-apply");
    write(&dir, "main.ya", "fn main() { var int x = 1; }\n");
    let output = Command::new(yashyc())
        .args(["fix", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let fixed = fs::read_to_string(dir.join("main.ya")).expect("the fixed file");
    assert!(
        fixed.contains("let int x"),
        "the edit must replace `var`: {fixed}"
    );
    assert!(!fixed.contains("var int"), "{fixed}");

    let check = Command::new(yashyc())
        .args(["check", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    assert!(
        check.status.success(),
        "a second check must be clean:\n{}",
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn dry_run_does_not_write() {
    let dir = work_dir("fix-dry");
    let original = "fn main() { var int x = 1; }\n";
    write(&dir, "main.ya", original);
    let output = Command::new(yashyc())
        .args(["fix", "--dry-run", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("let int x"));
    assert_eq!(fs::read_to_string(dir.join("main.ya")).unwrap(), original);
}

#[test]
fn applying_edits_back_to_front_is_safe() {
    let source = "let a = 1; let b = 2;";
    let edits = vec![
        Suggestion {
            span: Span::new(0, 3),
            replacement: "const".to_string(),
            applicability: Applicability::MachineApplicable,
            message: "const".to_string(),
        },
        Suggestion {
            span: Span::new(11, 14),
            replacement: "const".to_string(),
            applicability: Applicability::MachineApplicable,
            message: "const".to_string(),
        },
    ];
    assert_eq!(
        apply_suggestions(source, &edits).unwrap(),
        "const a = 1; const b = 2;"
    );
}

#[test]
fn a_stale_hash_is_refused() {
    let source = "fn main() {}";
    let edits: Vec<Suggestion> = Vec::new();
    assert_eq!(
        apply_suggestions_checked(source, &edits, Some(source_hash(source))).unwrap(),
        source
    );
    assert!(apply_suggestions_checked(source, &edits, Some(123)).is_err());
}
