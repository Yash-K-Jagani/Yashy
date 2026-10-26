mod common;

use common::{exe, work_dir, write, yashy};
use std::process::Command;

#[test]
fn run_compiles_and_prints_program_output() {
    let dir = work_dir("run-output");
    write(
        &dir,
        "main.ya",
        "fn main() { println(\"hello from run\"); }\n",
    );

    let output = Command::new(yashy())
        .arg("run")
        .arg(dir.join("main.ya"))
        .current_dir(&dir)
        .output()
        .expect("the yashy CLI must be runnable");

    assert!(
        output.status.success(),
        "run must succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "hello from run\n");
}

#[test]
fn run_forwards_the_program_exit_code() {
    let dir = work_dir("run-exit");
    write(&dir, "main.ya", "fn main() -> int { return 7; }\n");

    let output = Command::new(yashy())
        .arg("run")
        .arg(dir.join("main.ya"))
        .current_dir(&dir)
        .output()
        .expect("the yashy CLI must be runnable");

    assert_eq!(output.status.code(), Some(7));
}

#[test]
fn run_forwards_user_arguments_after_the_separator() {
    let dir = work_dir("run-args");
    write(
        &dir,
        "main.ya",
        "fn main() {\n\
         \x20   let argv = args();\n\
         \x20   print(argv.len());\n\
         \x20   println(\"\");\n\
         \x20   for value in argv { println(value); }\n\
         \x20   argv.free();\n\
         }\n",
    );

    let output = Command::new(yashy())
        .arg("run")
        .arg(dir.join("main.ya"))
        .arg("--")
        .arg("one")
        .arg("two")
        .current_dir(&dir)
        .env("YASHY_LEAK_REPORT", "1")
        .output()
        .expect("the yashy CLI must be runnable");

    assert!(
        output.status.success(),
        "run must succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "2\none\ntwo\n");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("yashy: leak report: 0 live allocations"),
        "the arguments must be released:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn run_reports_the_directory_it_started_in() {
    let dir = work_dir("run-cwd");
    write(&dir, "main.ya", "fn main() { println(getcwd()); }\n");

    let output = Command::new(yashy())
        .arg("run")
        .arg(dir.join("main.ya"))
        .current_dir(&dir)
        .env("YASHY_LEAK_REPORT", "1")
        .output()
        .expect("the yashy CLI must be runnable");

    assert!(
        output.status.success(),
        "run must succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reported = std::path::PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    assert_eq!(
        reported.canonicalize().expect("the reported path must exist"),
        dir.canonicalize().expect("the work directory must exist")
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("yashy: leak report: 0 live allocations"),
        "the directory string must be released:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn run_sleeps_for_at_least_the_requested_time() {
    let dir = work_dir("run-time");
    write(
        &dir,
        "main.ya",
        "fn main() {\n\
         \x20   let start = now();\n\
         \x20   sleep(30);\n\
         \x20   let end = now();\n\
         \x20   let stamp = str_from_i64(end);\n\
         \x20   if end - start >= 20 { println(\"slept\"); } else { println(\"too fast\"); }\n\
         \x20   if stamp.len() > 0 { println(\"stamp\"); }\n\
         }\n",
    );

    let started = std::time::Instant::now();
    let output = Command::new(yashy())
        .arg("run")
        .arg(dir.join("main.ya"))
        .current_dir(&dir)
        .env("YASHY_LEAK_REPORT", "1")
        .output()
        .expect("the yashy CLI must be runnable");
    let elapsed = started.elapsed();

    assert!(
        output.status.success(),
        "run must succeed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "slept\nstamp\n");
    assert!(
        elapsed >= std::time::Duration::from_millis(20),
        "run returned after only {elapsed:?}"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("yashy: leak report: 0 live allocations"),
        "the clock builtins allocate nothing:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(windows)]
#[test]
fn run_forwards_a_32_bit_exit_code() {
    let dir = work_dir("run-exit-large");
    write(&dir, "main.ya", "fn main() -> int { return 256; }\n");

    let output = Command::new(yashy())
        .arg("run")
        .arg(dir.join("main.ya"))
        .current_dir(&dir)
        .output()
        .expect("the yashy CLI must be runnable");

    assert_eq!(output.status.code(), Some(256));
}

#[test]
fn run_rejects_output_paths() {
    let dir = work_dir("run-output-flag");
    write(&dir, "main.ya", "fn main() {}\n");

    let output = Command::new(yashy())
        .arg("run")
        .arg(dir.join("main.ya"))
        .arg("-o")
        .arg(exe(&dir, "kept"))
        .output()
        .expect("the yashy CLI must be runnable");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("run"),
        "the error should mention run:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
