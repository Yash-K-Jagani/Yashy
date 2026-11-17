//! `yashyc repl`: declarations persist, expressions report a type, and the
//! forms the REPL cannot evaluate are refused with the compiler alternative.

mod common;

use std::io::Write;
use std::process::{Command, Stdio};

fn repl(input: &str) -> std::process::Output {
    let mut child = Command::new(common::yashyc())
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the REPL must start");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(input.as_bytes())
        .expect("write the transcript");
    child.wait_with_output().expect("the REPL must finish")
}

#[test]
fn an_expression_is_compiled_and_its_type_is_printed() {
    let output = repl("1 + 2\n:quit\n");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains('3'), "the value must be printed: {stdout}");
    assert!(
        stdout.contains("type: int"),
        "the checker's type must be shown: {stdout}"
    );
}

#[test]
fn a_declaration_is_kept_for_later_lines() {
    let output = repl("fn double(int x) -> int { return x * 2; }\ndouble(21)\n:quit\n");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("ok"),
        "the declaration must be accepted: {stdout}"
    );
    assert!(
        stdout.contains("42"),
        "the retained function must be callable: {stdout}"
    );
    assert!(stdout.contains("type: int"), "{stdout}");
}

#[test]
fn unsupported_forms_name_the_compiler_alternative() {
    let output = repl("extern fn puts(string s) -> int;\n:quit\n");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("yashyc run"), "{stderr}");
    assert!(
        output.status.success(),
        "a refusal is not a crash: {stderr}"
    );
}

#[test]
fn a_bad_declaration_does_not_poison_the_session() {
    let output = repl("fn broken( {\n1 + 1\n:quit\n");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{all}");
    assert!(
        all.contains("error"),
        "the bad line must be diagnosed: {all}"
    );
    assert!(all.contains("type: int"), "later lines still work: {all}");
}
