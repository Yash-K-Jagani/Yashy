//! Module resolution: imports, search roots, cycles, and one definition per
//! module.
//!
//! These tests drive the CLI so the environment variable, the diagnostic
//! rendering and the exit code are all covered the way a user sees them. Each
//! one builds into its own temporary directory; nothing is shared between
//! tests except the compiler binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use yashy_driver::modules::resolve_import;

/// The `yashy` binary cargo built for this test run.
fn yashy() -> &'static str {
    env!("CARGO_BIN_EXE_yashy")
}

/// A fresh, empty directory for one test, unique per test name and process.
fn work_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yashy-modules-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("the test work directory must be creatable");
    dir
}

/// Write `relative` under `dir`, creating parent directories.
fn write(dir: &Path, relative: &str, text: &str) {
    let path = dir.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("module directories must be creatable");
    }
    fs::write(&path, text).expect("module sources must be writable");
}

fn exe(dir: &Path) -> PathBuf {
    dir.join(format!("program{}", std::env::consts::EXE_SUFFIX))
}

/// Build `entry` with an explicit `YASHY_PATH` (`None` removes the variable, so
/// the default root is what is under test).
fn build(dir: &Path, entry: &Path, search: Option<&Path>) -> Output {
    let mut command = Command::new(yashy());
    command.arg("build").arg(entry).arg("-o").arg(exe(dir));
    match search {
        Some(root) => command.env("YASHY_PATH", root),
        None => command.env_remove("YASHY_PATH"),
    };
    command
        .output()
        .expect("the yashy CLI must be runnable")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Run a built executable, with the Device Guard hint the other suites use.
fn run(path: &Path) -> Output {
    Command::new(path).output().unwrap_or_else(|error| {
        let policy = error.to_string().contains("blocked")
            || error.to_string().contains("Application Control")
            || error.raw_os_error() == Some(4551);
        let hint = if policy {
            "\n\nthe operating system refused to start the binary: an Application \
             Control / Device Guard policy blocked it. Rebuilding produces a \
             different binary (the linker stamps a timestamp), which usually has a \
             different verdict."
        } else {
            ""
        };
        panic!(
            "the compiled executable at {} must be runnable: {error}{hint}",
            path.display()
        )
    })
}

/// The `YASHY_PATH` separator on this platform.
fn path_separator() -> &'static str {
    if cfg!(windows) { ";" } else { ":" }
}

#[test]
fn a_dotted_path_resolves_to_a_nested_file() {
    let dir = work_dir("dotted");
    write(
        &dir,
        "main.ya",
        "import lib.util;\nfn main() { print_i64(seven()); print_str(\"\\n\"); }\n",
    );
    write(&dir, "lib/util.ya", "pub fn seven() -> i64 { return 7; }\n");

    let output = build(&dir, &dir.join("main.ya"), None);
    assert!(
        output.status.success(),
        "the build must succeed:\n{}",
        stderr_of(&output)
    );
    assert_eq!(stdout_of(&run(&exe(&dir))), "7\n");
}

#[test]
fn a_module_directory_containing_mod_ot_is_found() {
    let dir = work_dir("mod-ot");
    write(
        &dir,
        "main.ya",
        "import pkg.nested;\nfn main() { print_i64(three()); print_str(\"\\n\"); }\n",
    );
    write(&dir, "pkg/nested/mod.ya", "pub fn three() -> i64 { return 3; }\n");

    let output = build(&dir, &dir.join("main.ya"), None);
    assert!(
        output.status.success(),
        "a directory module must be found:\n{}",
        stderr_of(&output)
    );
    assert_eq!(stdout_of(&run(&exe(&dir))), "3\n");
}

#[test]
fn a_module_search_prefers_the_flat_file_then_the_directory() {
    let dir = work_dir("resolve");
    write(&dir, "root/a/b.ya", "fn flat() -> i64 { return 1; }\n");
    write(&dir, "root/a/c/mod.ya", "fn nested() -> i64 { return 2; }\n");
    let roots = vec![dir.join("root")];

    assert_eq!(
        resolve_import("a.b", None, &roots),
        Ok(dir.join("root").join("a").join("b.ya"))
    );
    assert_eq!(
        resolve_import("a.c", None, &roots),
        Ok(dir.join("root").join("a").join("c").join("mod.ya"))
    );
}

#[test]
fn a_missing_module_is_a_diagnostic_naming_every_root_tried() {
    let dir = work_dir("missing");
    write(&dir, "main.ya", "import lib.nope;\nfn main() { }\n");
    let first = dir.join("root_a");
    let second = dir.join("root_b");
    fs::create_dir_all(&first).expect("first root must be creatable");
    fs::create_dir_all(&second).expect("second root must be creatable");

    let search = format!(
        "{}{}{}",
        first.display(),
        path_separator(),
        second.display()
    );
    let mut command = Command::new(yashy());
    command
        .arg("build")
        .arg(dir.join("main.ya"))
        .arg("-o")
        .arg(exe(&dir))
        .env("YASHY_PATH", &search);
    let output = command
        .output()
        .expect("the yashy CLI must be runnable");

    assert!(!output.status.success(), "a missing module must fail the build");
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("cannot find module `lib.nope`"),
        "the message must name the module:\n{stderr}"
    );
    for candidate in [
        first.join("lib").join("nope.ya"),
        second.join("lib").join("nope.ya"),
        first.join("lib").join("nope").join("mod.ya"),
        second.join("lib").join("nope").join("mod.ya"),
    ] {
        assert!(
            stderr.contains(&candidate.display().to_string()),
            "every root and both shapes must be named; missing {}:\n{stderr}",
            candidate.display()
        );
    }
}

#[test]
fn an_import_cycle_is_reported_not_looped_on() {
    let dir = work_dir("cycle");
    write(&dir, "main.ya", "import a;\nfn main() { }\n");
    write(&dir, "a.ya", "import b;\npub fn a() -> i64 { return b(); }\n");
    write(&dir, "b.ya", "import a;\npub fn b() -> i64 { return 1; }\n");

    let output = build(&dir, &dir.join("main.ya"), None);
    assert!(!output.status.success(), "a cycle must fail the build");
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("import cycle"),
        "the message must say it is a cycle:\n{stderr}"
    );
    assert!(
        stderr.contains("a.ya") && stderr.contains("b.ya"),
        "the cycle must name both modules:\n{stderr}"
    );
}

#[test]
fn importing_the_same_module_from_two_files_defines_it_once() {
    let dir = work_dir("diamond");
    write(
        &dir,
        "main.ya",
        "import a;\nimport b;\n\
         fn main() { print_i64(from_a() + from_b()); print_str(\"\\n\"); }\n",
    );
    write(
        &dir,
        "a.ya",
        "import shared;\npub fn from_a() -> i64 { return answer(); }\n",
    );
    write(
        &dir,
        "b.ya",
        "import shared;\npub fn from_b() -> i64 { return answer(); }\n",
    );
    write(
        &dir,
        "shared.ya",
        "pub fn answer() -> i64 { return 21; }\n",
    );

    let output = build(&dir, &dir.join("main.ya"), None);
    assert!(
        output.status.success(),
        "a diamond must load the shared module once:\n{}",
        stderr_of(&output)
    );
    assert_eq!(stdout_of(&run(&exe(&dir))), "42\n");
}

#[test]
fn a_name_defined_in_two_modules_is_an_ambiguity_error() {
    let dir = work_dir("ambiguous");
    write(
        &dir,
        "main.ya",
        "import a;\nimport b;\nfn main() { print_i64(thing()); }\n",
    );
    write(&dir, "a.ya", "pub fn thing() -> i64 { return 1; }\n");
    write(&dir, "b.ya", "pub fn thing() -> i64 { return 2; }\n");

    let output = build(&dir, &dir.join("main.ya"), None);
    assert!(
        !output.status.success(),
        "the same name in two modules must fail the build"
    );
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("duplicate definition of `thing`"),
        "the message must name the duplicated item:\n{stderr}"
    );
    assert!(
        stderr.contains("b.ya"),
        "the caret must be in the module that redefined it:\n{stderr}"
    );
}

#[test]
fn yashy_path_overrides_the_default_root() {
    let dir = work_dir("yashy-path");
    write(
        &dir,
        "main.ya",
        "import vendor.helper;\nfn main() { print_i64(helper()); print_str(\"\\n\"); }\n",
    );
    write(&dir, "external/vendor/helper.ya", "pub fn helper() -> i64 { return 9; }\n");
    let search = dir.join("external");

    let without = build(&dir, &dir.join("main.ya"), None);
    assert!(
        !without.status.success(),
        "without YASHY_PATH the module is not under the entry's directory"
    );

    let with = build(&dir, &dir.join("main.ya"), Some(&search));
    assert!(
        with.status.success(),
        "YASHY_PATH must be searched:\n{}",
        stderr_of(&with)
    );
    assert_eq!(stdout_of(&run(&exe(&dir))), "9\n");
}

#[test]
fn check_accepts_a_multi_file_program_without_building_it() {
    let dir = work_dir("check");
    write(
        &dir,
        "main.ya",
        "import lib.util;\nfn main() { print_i64(seven()); }\n",
    );
    write(&dir, "lib/util.ya", "pub fn seven() -> i64 { return 7; }\n");

    let mut command = Command::new(yashy());
    command
        .arg("check")
        .arg(dir.join("main.ya"))
        .env_remove("YASHY_PATH");
    let output = command
        .output()
        .expect("the yashy CLI must be runnable");

    assert!(
        output.status.success(),
        "`check` must resolve imports without a toolchain:\n{}",
        stderr_of(&output)
    );
    assert!(
        !exe(&dir).exists(),
        "`check` must not produce an executable"
    );
}

#[test]
fn an_error_in_a_module_names_the_module_not_the_entry_point() {
    let dir = work_dir("module-error");
    write(
        &dir,
        "main.ya",
        "import lib.broken;\nfn main() { print_i64(value()); }\n",
    );
    write(
        &dir,
        "lib/broken.ya",
        "pub fn value() -> i64 { return \"not a number\"; }\n",
    );

    let output = build(&dir, &dir.join("main.ya"), None);
    assert!(!output.status.success(), "the module must fail to check");
    let stderr = stderr_of(&output);
    let module = dir.join("lib").join("broken.ya").display().to_string();
    assert!(
        stderr.contains(&module),
        "the diagnostic must name {module}:\n{stderr}"
    );
    assert!(
        !stderr.contains("main.ya:2"),
        "the caret must not point into the entry point:\n{stderr}"
    );
}
