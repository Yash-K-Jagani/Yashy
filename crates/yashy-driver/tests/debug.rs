//! `yashyc debug`: backend selection and script generation.

mod common;

use std::path::Path;
use std::process::Command;

use yashy_driver::debug::{script, Backend};

#[test]
fn the_debug_verb_is_in_the_command_table() {
    let output = Command::new(common::yashyc())
        .args(["help", "debug"])
        .output()
        .expect("help must run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--break"), "{stdout}");
    assert!(stdout.contains("--binary"), "{stdout}");
}

#[test]
fn both_backends_get_a_runnable_script() {
    let lldb = Backend::Lldb(Path::new("lldb").to_path_buf());
    let cdb = Backend::Cdb(Path::new("cdb.exe").to_path_buf());
    for backend in [&lldb, &cdb] {
        let text = script(backend, Path::new("app"), &["main".to_string()], &[], None);
        assert!(
            text.ends_with('\n'),
            "a script ends with a newline: {text:?}"
        );
        assert!(text.contains("main"), "{text}");
        assert_eq!(
            backend.name(),
            if backend == &lldb { "lldb" } else { "cdb" }
        );
    }
}

#[test]
fn the_debug_docs_name_the_limits() {
    let docs = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/debugging.md"),
    )
    .expect("docs/debugging.md exists");
    assert!(docs.contains("no line table"), "{docs}");
    assert!(docs.contains("symbol breakpoints"), "{docs}");
}
