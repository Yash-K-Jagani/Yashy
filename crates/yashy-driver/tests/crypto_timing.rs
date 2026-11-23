//! A smoke test, not a proof: the library's tag comparison must not have a
//! gross timing leak. The shim times equal and unequal 16-byte tags over the
//! same number of calls and reports the ratio in hundredths.

mod common;

use std::process::Command;

#[test]
fn constant_time_tag_compare_has_no_gross_leak() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = root.join("packages/crypto/tests/timing.ya");
    let dir = common::work_dir("crypto-timing");
    let binary = dir.join(format!("timing{}", std::env::consts::EXE_SUFFIX));
    let build = Command::new(common::yashyc())
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .current_dir(&root)
        .output()
        .expect("the compiler must run");
    assert!(
        build.status.success(),
        "building the timing probe failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&binary)
        .output()
        .expect("the timing probe must run");
    assert!(
        run.status.success(),
        "the timing probe exited with {:?}",
        run.status.code()
    );
    let text = String::from_utf8_lossy(&run.stdout);
    let ratio: i64 = text
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("the probe must print a ratio: {text}"));
    assert!(
        (5..=2000).contains(&ratio),
        "mismatching/matching tag timing ratio {ratio} hundredths is outside 0.05x..20x"
    );
}
