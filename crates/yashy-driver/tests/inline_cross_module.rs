//! The inliner's cross-module half: a small function in an imported module is
//! still one function in the emitted module, so the same policy marks it for
//! inlining and the report still names the caller.

mod common;

use common::{work_dir, write, yashyc};
use std::path::Path;
use std::process::Command;

fn run(dir: &Path, args: &[&str]) -> String {
    let output = Command::new(yashyc())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the yashyc CLI must be runnable");
    assert!(
        output.status.success(),
        "{} failed:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn a_small_function_in_an_imported_module_earns_alwaysinline() {
    let dir = work_dir("inline-cross-module");
    write(
        &dir,
        "main.ya",
        "import lib.helper;\nfn main() {\n    assert(value() == 7);\n    assert(value() == 7);\n}\n",
    );
    write(
        &dir,
        "lib/helper.ya",
        "pub fn value() -> int { return 7; }\n",
    );
    let ir = run(&dir, &["build", "main.ya", "--emit-ir"]);
    let report = run(&dir, &["build", "main.ya", "--opt-report=json"]);

    assert!(
        ir.lines()
            .any(|line| line.starts_with("define ") && line.contains("alwaysinline")),
        "the imported callee must carry the attribute in the emitted module: {ir}"
    );
    assert!(
        report.contains("2 call sites from main"),
        "the report must name the caller in the imported module's decision: {report}"
    );
}
