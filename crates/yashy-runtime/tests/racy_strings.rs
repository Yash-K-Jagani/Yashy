//! Builds and runs `tests/racy_strings.c` against the real runtime.
//!
//! The test has to call `yashy_str_retain_atomic`/`yashy_str_release_atomic`
//! directly and run real OS threads, so a C program built with the same
//! compiler `build.rs` found is the only faithful harness. A codegen test sees
//! the IR that calls these symbols but never their concurrency.

use std::path::PathBuf;
use std::process::Command;

fn runtime_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A fresh directory per run: `cl` writes `.obj` files beside its working
/// directory, and two concurrent runs must not share them.
fn output_dir() -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("yashy-racy-{}-{stamp}", std::process::id()))
}

fn build_racy_program() -> PathBuf {
    // `cc` normally runs inside a build script, where Cargo sets `OPT_LEVEL`,
    // `TARGET` and `HOST`. This test runs outside one, so supply them; the
    // build script passed the two triples through as compile-time constants.
    if std::env::var_os("OPT_LEVEL").is_none() {
        std::env::set_var("OPT_LEVEL", "0");
    }
    if std::env::var_os("TARGET").is_none() {
        std::env::set_var("TARGET", env!("YASHY_TARGET"));
    }
    if std::env::var_os("HOST").is_none() {
        std::env::set_var("HOST", env!("YASHY_HOST"));
    }

    let root = runtime_dir();
    let out_dir = output_dir();
    std::fs::create_dir_all(&out_dir).expect("create the racy strings test directory");

    let executable = out_dir.join(if cfg!(windows) { "racy.exe" } else { "racy" });
    let include = root.join("include");
    let sources = [
        root.join("tests").join("racy_strings.c"),
        root.join("runtime.c"),
        root.join("hashmap.c"),
    ];

    let compiler = cc::Build::new().cargo_metadata(false).get_compiler();
    let mut command = compiler.to_command();
    command.current_dir(&out_dir);

    if compiler.is_like_msvc() {
        command.arg("/nologo");
        command.arg(format!("/I{}", include.display()));
        command.args(&sources);
        command.arg(format!("/Fe:{}", executable.display()));
    } else {
        command.args(["-std=c11", "-O1", "-g", "-Wall", "-Wextra"]);
        if !cfg!(windows) {
            command.arg("-pthread");
        }
        command.arg(format!("-I{}", include.display()));
        command.args(&sources);
        command.arg("-o").arg(&executable);
        if !cfg!(windows) {
            command.arg("-lm");
        }
    }

    let built = command
        .output()
        .expect("run the C compiler on tests/racy_strings.c");
    assert!(
        built.status.success(),
        "building tests/racy_strings.c failed:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    executable
}

#[test]
fn concurrent_retain_and_release_do_not_double_free() {
    let executable = build_racy_program();
    let run = Command::new(&executable)
        .output()
        .expect("run the compiled racy strings test");
    assert!(
        run.status.success(),
        "tests/racy_strings.c failed with {:?}:\nstdout:\n{}\nstderr:\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );

    let directory = executable.parent().expect("the executable has a parent");
    let _ = std::fs::remove_dir_all(directory);
}
