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

fn pipe_plain(cmd: &mut Command) -> (i32, String, String) {
    let output = cmd.output().expect("the CLI must run");
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
fn the_man_page_renders_every_verb() {
    let (code, stdout, stderr) = run(&["man"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.starts_with(".TH YASHC 1"), "{stdout}");
    for command in COMMANDS {
        assert!(stdout.contains(&format!(".B {}", command.name)), "{stdout}");
    }
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

#[test]
fn always_colour_emits_sgr_and_never_does_not() {
    let dir = common::work_dir("terminal-colour");
    common::write(&dir, "main.ya", "fn main() { missing(); }\n");
    let (code, _, always) = pipe_plain(
        Command::new(yashyc())
            .args(["check", "--color=always", "main.ya"])
            .current_dir(&dir),
    );
    assert_eq!(code, 1, "{always}");
    assert!(
        always.contains('\u{1b}'),
        "--color=always must emit SGR: {always:?}"
    );

    let (_, _, never) = pipe_plain(
        Command::new(yashyc())
            .args(["check", "--color=never", "main.ya"])
            .current_dir(&dir),
    );
    assert!(
        !never.contains('\u{1b}'),
        "--color=never must be plain: {never:?}"
    );
}

#[test]
fn columns_wraps_a_long_diagnostic() {
    let dir = common::work_dir("terminal-columns");
    common::write(&dir, "main.ya", "fn main() { let x: int = 1; }\n");
    let (_, _, stderr) = pipe_plain(
        Command::new(yashyc())
            .args(["check", "--color=never", "main.ya"])
            .env("COLUMNS", "40")
            .current_dir(&dir),
    );
    assert!(stderr.contains("E0109"), "{stderr}");
    for line in stderr.lines() {
        assert!(
            line.chars().count() <= 40,
            "COLUMNS=40 must wrap `{line}` ({} chars)",
            line.chars().count()
        );
    }
}

#[test]
fn an_unknown_colour_policy_is_a_usage_error() {
    let (code, _, stderr) = run(&["--color=sometimes", "check", "main.ya"]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("unknown colour policy"), "{stderr}");
}

#[test]
fn an_unknown_hyperlink_policy_is_a_usage_error() {
    let (code, _, stderr) = run(&["--hyperlink=sometimes", "check", "main.ya"]);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("unknown hyperlink policy"), "{stderr}");
}

#[test]
fn a_pipe_is_never_paged() {
    // A pager that would leave a marker if it ran. The output is a pipe, so
    // the page must reach stdout and the marker must never appear.
    let dir = common::work_dir("terminal-pager");
    common::write(&dir, "main.ya", "fn main() {}\n");
    let marker = dir.join("paged.txt");
    let script = dir.join("pager.cmd");
    common::write(
        &dir,
        "pager.cmd",
        &format!("@echo paged> \"{}\"\r\n", marker.display()),
    );
    let output = Command::new(yashyc())
        .args(["explain", "E0100"])
        .env("PAGER", script.display().to_string())
        .current_dir(&dir)
        .output()
        .expect("the CLI must run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("## Cause"),
        "the page must reach a pipe: {stdout}"
    );
    assert!(
        !marker.exists(),
        "a pipe must not start a pager: {} exists",
        marker.display()
    );
}

#[test]
fn the_pager_flags_are_accepted() {
    for flag in ["--no-pager", "--pager"] {
        let (code, stdout, stderr) = run(&[flag, "explain", "E0100"]);
        assert_eq!(code, 0, "{flag}: {stderr}");
        assert!(stdout.contains("## Cause"), "{flag}: {stdout}");
    }
}

#[test]
fn hyperlink_always_wraps_the_source_position() {
    let dir = common::work_dir("terminal-hyperlink");
    common::write(&dir, "main.ya", "fn main() { missing(); }\n");
    let (code, _, always) = pipe_plain(
        Command::new(yashyc())
            .args(["check", "--hyperlink=always", "--color=never", "main.ya"])
            .current_dir(&dir),
    );
    assert_eq!(code, 1, "{always}");
    assert!(
        always.contains("\u{1b}]8;;file:///"),
        "--hyperlink=always must emit an OSC 8 link: {always:?}"
    );
    assert!(
        always.contains("\u{1b}]8;;\u{1b}\\"),
        "the link must be closed: {always:?}"
    );

    let (_, _, never) = pipe_plain(
        Command::new(yashyc())
            .args(["check", "--hyperlink=never", "main.ya"])
            .current_dir(&dir),
    );
    assert!(
        !never.contains('\u{1b}'),
        "--hyperlink=never must be plain: {never:?}"
    );

    // A pipe defaults to plain text even when the terminal would allow a link.
    let (_, _, default) = pipe_plain(
        Command::new(yashyc())
            .args(["check", "main.ya"])
            .current_dir(&dir),
    );
    assert!(
        !default.contains('\u{1b}'),
        "a pipe must be plain: {default:?}"
    );
}

#[test]
fn json_never_carries_a_link_or_a_colour() {
    let dir = common::work_dir("terminal-hyperlink-json");
    common::write(&dir, "main.ya", "fn main() { missing(); }\n");
    let (code, stdout, stderr) = pipe_plain(
        Command::new(yashyc())
            .args([
                "check",
                "--json",
                "--color=always",
                "--hyperlink=always",
                "main.ya",
            ])
            .current_dir(&dir),
    );
    assert_eq!(code, 1, "{stderr}");
    assert!(
        !stdout.contains('\u{1b}'),
        "the JSON envelope must carry no escape: {stdout:?}"
    );
}

/// The four shells the compiler promises to complete.
const SHELLS: &[(&str, &str)] = &[
    ("bash", "yashyc.bash"),
    ("zsh", "yashyc.zsh"),
    ("fish", "yashyc.fish"),
    ("powershell", "yashyc.ps1"),
];

fn scripts_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/completions")
}

#[test]
fn the_committed_completion_scripts_are_the_rendered_ones() {
    for (shell, file) in SHELLS {
        let rendered = yashy_driver::cli_spec::render_completions(shell)
            .unwrap_or_else(|error| panic!("`{shell}` must render: {error}"));
        let committed = std::fs::read_to_string(scripts_dir().join(file)).unwrap_or_else(|error| {
            panic!("scripts/completions/{file} must exist: {error}");
        });
        assert_eq!(
            committed, rendered,
            "scripts/completions/{file} is stale; run `yashyc completions {shell} > scripts/completions/{file}`"
        );
        for command in COMMANDS {
            assert!(
                committed.contains(command.name),
                "scripts/completions/{file} must complete `{}`",
                command.name
            );
        }
    }
}

#[test]
fn the_committed_completion_scripts_parse() {
    // `bash -n` parses without executing, which is the check CI needs. The
    // zsh script deliberately uses no bash-only syntax, so one parser covers
    // both; a host without bash skips the check rather than failing.
    let bash = Command::new("bash").arg("--version").output();
    let Ok(version) = bash else {
        return;
    };
    assert!(version.status.success(), "`bash --version` must succeed");
    for (shell, file) in SHELLS.iter().filter(|(shell, _)| *shell != "powershell") {
        let path = format!("scripts/completions/{file}");
        let output = Command::new("bash")
            .args(["-n", path.as_str()])
            .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
            .output()
            .expect("bash must run");
        assert!(
            output.status.success(),
            "scripts/completions/{file} ({shell}) must parse:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
