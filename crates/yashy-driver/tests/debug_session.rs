//! A scripted LLDB session: stop on a source line and print a typed value.
//!
//! LLDB is not a required part of the toolchain, so the test is opt-in through
//! `YASHY_LLDB_TEST=1`. CI installs LLDB with the rest of the LLVM tools and
//! sets it; a developer without LLDB sees a skip note rather than a failure.
//! The same session also proves the summary providers are imported before the
//! first stop: the assertion is on the value (`ada`), not on a pointer.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

const PROGRAM: &str = concat!(
    "struct Point {\n",
    "    x: int,\n",
    "}\n",
    "\n",
    "fn total(int n) -> int {\n",
    "    let name = \"ada\";\n",
    "    let p = Point { x: n };\n",
    "    if n > 0 {\n",
    "        let inner = n + 1;\n",
    "        print(inner);\n",
    "    }\n",
    "    return p.x + n;\n",
    "}\n",
    "\n",
    "fn main() {\n",
    "    let answer = total(2);\n",
    "    print(answer);\n",
    "}\n",
);

/// The `print(inner)` line, where `name`, `p` and `inner` are all live.
const BREAK_LINE: usize = 10;

fn lldb_for_the_session() -> Option<PathBuf> {
    if std::env::var("YASHY_LLDB_TEST").ok().as_deref() != Some("1") {
        return None;
    }
    match yashy_driver::debug::detect_backend() {
        Ok(yashy_driver::debug::Backend::Lldb(path)) => Some(path),
        Ok(yashy_driver::debug::Backend::Cdb(_)) => {
            eprintln!("skipping the scripted debugger session: only LLDB speaks this script");
            None
        }
        Err(message) => {
            eprintln!("skipping the scripted debugger session: {message}");
            None
        }
    }
}

fn slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn a_scripted_session_stops_on_the_line_and_prints_the_string() {
    let Some(lldb) = lldb_for_the_session() else {
        return;
    };
    let dir = common::work_dir("debug-session");
    common::write(&dir, "main.ya", PROGRAM);
    let exe = common::exe(&dir, "debug_session");
    let build = Command::new(common::yashyc())
        .args(["build", "main.ya", "--debug-info", "-O0", "-o"])
        .arg(&exe)
        .current_dir(&dir)
        .output()
        .expect("the yashyc CLI must be runnable");
    assert!(
        build.status.success(),
        "the debug build must succeed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let printers =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../yashy-runtime/debug/pretty_printers.py");
    let script = dir.join("session.lldb");
    let body = format!(
        "settings set target.env-vars YASHY_DEBUG=1\n\
         target create \"{}\"\n\
         command script import \"{}\"\n\
         breakpoint set --file main.ya --line {BREAK_LINE}\n\
         run\n\
         frame variable\n\
         quit\n",
        slash(&exe),
        slash(&printers),
    );
    std::fs::write(&script, body).expect("the debugger script must be writable");

    let output = Command::new(lldb)
        .arg("--source")
        .arg(&script)
        .current_dir(&dir)
        .output()
        .expect("LLDB must be startable");
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "the scripted session must finish:\n{text}"
    );
    assert!(
        text.contains("stop reason = breakpoint"),
        "the session must stop on the breakpoint:\n{text}"
    );
    assert!(
        text.contains("main.ya") && text.contains(&format!(":{BREAK_LINE}")),
        "the stop names the source line:\n{text}"
    );
    // The provider returns `repr("ada")`, i.e. `'ada'`; the exact quote style
    // is LLDB's. What matters is that the value is the string, not its address.
    assert!(
        text.contains("ada"),
        "the string local prints its value, not a pointer:\n{text}"
    );
}
