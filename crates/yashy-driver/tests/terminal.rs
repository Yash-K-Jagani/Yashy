//! The terminal surface: help, completions and plain piped output.
//!
//! Every verb is declared once in `cli_spec`; help and all four completion
//! scripts must render that same table. An unknown verb names the nearest one.

mod common;

use common::yashyc;
use std::process::Command;
use yashy_driver::cli_spec::COMMANDS;

fn run(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(yashyc())
        .args(args)
        .output()
        .expect("the CLI must run");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn help_lists_every_verb() {
    let (code, stdout, stderr) = run(&["help"]);
    assert_eq!(code, 0, "{stderr}");
    for command in COMMANDS {
        assert!(
            stdout.contains(command.name),
            "help must list `{}`",
            command.name
        );
    }
}

#[test]
fn help_one_verb_shows_its_flags() {
    let (code, stdout, stderr) = run(&["help", "build"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("yashyc build"), "{stdout}");
    assert!(stdout.contains("--timings"), "{stdout}");
    let (code, _, stderr) = run(&["help", "no-such-verb"]);
    assert_eq!(code, 2, "{stderr}");
}

#[test]
fn completions_cover_every_verb_for_every_shell() {
    for shell in ["bash", "zsh", "fish", "powershell"] {
        let (code, stdout, stderr) = run(&["completions", shell]);
        assert_eq!(code, 0, "{shell}: {stderr}");
        for command in COMMANDS {
            assert!(
                stdout.contains(command.name),
                "{shell} completions must name `{}`",
                command.name
            );
        }
        assert!(!stdout.is_empty(), "{shell} completions must not be empty");
    }
    let (code, _, stderr) = run(&["completions", "nonesuch"]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("unknown shell"), "{stderr}");
}

#[test]
fn an_unknown_verb_suggests_the_nearest_one() {
    let (code, _, stderr) = run(&["biuld"]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("did you mean `build`"), "{stderr}");
}

#[test]
fn piped_output_has_no_ansi_escapes() {
    let dir = common::work_dir("terminal-plain");
    common::write(&dir, "main.ya", "fn main() { missing(); }\n");
    let output = Command::new(yashyc())
        .args(["check", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        all.contains("E0001"),
        "the diagnostic must be present: {all}"
    );
    assert!(
        !all.contains('\u{1b}'),
        "piped output must have no ANSI: {all:?}"
    );
}
