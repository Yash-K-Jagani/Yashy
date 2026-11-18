//! Source-based coverage: the report names Yashy source lines, not LLVM
//! instructions. The test skips on a machine without the LLVM coverage tools,
//! which is the same answer the CLI gives there rather than a fake success.

mod common;

use std::process::Command;

fn coverage_tools_available() -> bool {
    let Ok(toolchain) = yashy_driver::toolchain::detect() else {
        return false;
    };
    yashy_driver::coverage::find_tool("llvm-profdata", &toolchain.llc, "YASHY_LLVM_PROFDATA")
        .is_some()
        && yashy_driver::coverage::find_tool("llvm-cov", &toolchain.llc, "YASHY_LLVM_COV").is_some()
}

#[test]
fn coverage_reports_lines_from_the_yashy_source() {
    if !coverage_tools_available() {
        return;
    }
    let dir = common::work_dir("coverage-cli");
    common::write(
        &dir,
        "main.ya",
        "fn hot() -> int {\n    return 1;\n}\n\nfn cold() -> int {\n    return 2;\n}\n\n\
         fn main() {\n    print(hot());\n}\n",
    );
    let output = Command::new(common::yashyc())
        .args(["coverage", "--json", "main.ya"])
        .current_dir(&dir)
        .output()
        .expect("yashyc must run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "coverage must succeed:\n{}\n{stdout}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("\"files\""), "{stdout}");
    assert!(
        stdout.contains("main.ya"),
        "lines must name the source: {stdout}"
    );
    assert!(stdout.contains("\"total_lines\""), "{stdout}");
}

#[test]
fn a_test_suite_is_merged_into_one_report() {
    if !coverage_tools_available() {
        return;
    }
    let dir = common::work_dir("coverage-suite");
    // One test reaches `hot`, the other reaches `cold`; the merged report must
    // count both, because the suite as a whole executed both lines.
    common::write(
        &dir,
        "tests/hot.ya",
        "fn hot() -> int {\n    return 1;\n}\n\nfn main() {\n    assert(hot() == 1);\n}\n",
    );
    common::write(
        &dir,
        "tests/cold.ya",
        "fn cold() -> int {\n    return 2;\n}\n\nfn main() {\n    assert(cold() == 2);\n}\n",
    );
    let output = Command::new(common::yashyc())
        .args(["coverage", "--tests", "--json"])
        .current_dir(&dir)
        .output()
        .expect("yashyc must run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "suite coverage must succeed:\n{}\n{stdout}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("\"files\""), "{stdout}");
    assert!(stdout.contains("hot.ya"), "{stdout}");
    assert!(stdout.contains("cold.ya"), "{stdout}");
    let percent = stdout
        .split("\"percent\":")
        .nth(1)
        .and_then(|rest| rest.split([',', '}']).next())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0);
    assert!(
        percent > 0,
        "the merged suite must cover something: {stdout}"
    );
}
