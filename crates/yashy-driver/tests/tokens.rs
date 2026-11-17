//! `yashyc tokens --bpe`: the lexical single-token baseline.

mod common;

use std::process::Command;
#[test]
fn the_bpe_report_names_its_tokenizer_and_counts_tokens() {
    let dir = common::work_dir("tokens-bpe");
    common::write(&dir, "main.ya", "fn main() { print(1 + 2); }\n");
    let report =
        yashy_driver::tokens::bpe_json(&[dir.join("main.ya")]).expect("a valid file must measure");
    let text = report.render();
    assert!(text.contains("\"tokenizer\": \"yashy-lexer\""), "{text}");
    assert!(
        text.contains("not vendored"),
        "external vocabularies are not claimed: {text}"
    );
    assert!(
        text.contains("\"Fn\": 1"),
        "the `fn` keyword is one token: {text}"
    );
    assert!(text.contains("\"total_tokens\""), "{text}");
}

#[test]
fn the_verb_accepts_the_bpe_flag() {
    let output = Command::new(common::yashyc())
        .args(["help", "tokens"])
        .output()
        .expect("help must run");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("--bpe"));
}

#[test]
fn a_lex_error_is_a_message_not_a_panic() {
    let dir = common::work_dir("tokens-bpe-bad");
    common::write(&dir, "main.ya", "fn main() { \"unterminated }\n");
    let error = yashy_driver::tokens::bpe_json(&[dir.join("main.ya")])
        .expect_err("malformed source cannot be measured");
    assert!(error.contains("lex error"), "{error}");
}

#[test]
fn the_lexer_dumps_byte_spans() {
    let dir = common::work_dir("tokens-dump");
    common::write(&dir, "main.ya", "fn main() {}\n");
    let output = Command::new(common::yashyc())
        .args(["tokens", "--json", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"kind\": \"Fn\""), "{stdout}");
    assert!(stdout.contains("\"start\": 0"), "{stdout}");
    assert!(stdout.contains("\"end\": 2"), "{stdout}");
}
