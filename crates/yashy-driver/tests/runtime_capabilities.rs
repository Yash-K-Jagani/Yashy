//! The runtime capability handle installed at entry.
//!
//! A manifest that declares `[capabilities]` is a promise the compiler enforces
//! twice: at the HIR call site and again in the runtime when the operation
//! executes. The second half starts with generated code installing the grant;
//! a program with no declaration leaves the handle permissive so old projects
//! keep building.

mod common;

use std::process::Command;

fn ir(dir: &std::path::Path) -> String {
    let output = Command::new(common::yashyc())
        .args(["build", "main.ya", "--emit-ir"])
        .current_dir(dir)
        .output()
        .expect("yashyc must run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn a_declaration_installs_the_grant_before_the_first_statement() {
    let dir = common::work_dir("runtime-cap-ir");
    common::write(
        &dir,
        "yashy.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n\
         [capabilities]\nfs.read = [\"data/*\"]\nnet = [\"*\"]\n",
    );
    common::write(&dir, "main.ya", "fn main() { print(1); }\n");
    let text = ir(&dir);
    assert!(
        text.contains("yashy_capability_init"),
        "the entry installs the grant:\n{text}"
    );
    assert!(
        text.contains("fs.read") && text.contains("net"),
        "the resolved grant names are the argument:\n{text}"
    );
    let init = text.find("call void @yashy_capability_init").unwrap();
    let first_statement = text.find("call void @yashy_print_line_str").unwrap();
    assert!(
        init < first_statement,
        "the grant is installed first:\n{text}"
    );
}

#[test]
fn no_declaration_leaves_the_runtime_handle_permissive() {
    let dir = common::work_dir("runtime-cap-none");
    common::write(&dir, "main.ya", "fn main() { print(1); }\n");
    let text = ir(&dir);
    assert!(
        !text.contains("call void @yashy_capability_init"),
        "no manifest means no init call:\n{text}"
    );
}

#[test]
fn the_call_site_check_still_refuses_an_undeclared_operation() {
    let dir = common::work_dir("runtime-cap-front");
    common::write(
        &dir,
        "yashy.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n\
         [capabilities]\nfs.read = [\"data/*\"]\n",
    );
    common::write(&dir, "main.ya", "fn main() { let v = getenv(\"HOME\"); }\n");
    let output = Command::new(common::yashyc())
        .args(["build", "main.ya", "--emit-ir"])
        .current_dir(&dir)
        .output()
        .expect("yashyc must run");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "the build is refused: {all}");
    assert!(
        all.contains("E1808"),
        "the missing capability is named: {all}"
    );
}
