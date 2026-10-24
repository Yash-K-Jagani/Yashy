//! Link directives: `extern "C" link "m"` must reach the linker, and a
//! program that needs a library the platform hides (libm on Linux) must link
//! without the user writing anything.
//!
//! The execution test at the bottom runs only on the host; the CI matrix runs
//! the same test on Windows and Linux and pins the same bytes, which is what
//! makes "identical output on both platforms" a checked claim rather than an
//! intention.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use yashy_common::SourceMap;
use yashy_lexer::Lexer;
use yashy_parser::Parser;
use yashy_sema::{check_program, HirProgram};

use yashy_driver::pipeline::link_directives;

/// Parse and check `src`, treating every failure as a broken test fixture.
fn hir_of(src: &str) -> HirProgram {
    let mut sources = SourceMap::new();
    let file = sources.add_file("link.ya", src.to_string());
    let tokens = Lexer::new(src)
        .tokenize()
        .expect("the fixture's lexer must succeed");
    let mut parser = Parser::new(tokens, &sources, file);
    let program = parser
        .parse_program()
        .expect("the fixture must parse");
    check_program(&program, &sources, file).expect("the fixture must check")
}

fn directives(src: &str) -> Vec<String> {
    link_directives(&hir_of(src))
}

#[test]
fn a_link_directive_adds_the_library_flag() {
    let flags = directives(
        r#"extern "C" link "m" {
               unsafe fn sqrt(x: float) -> float;
           }

           fn main() {}
           "#,
    );
    assert_eq!(flags, vec!["-lm".to_string()]);
}

#[test]
fn a_declaration_without_a_link_contributes_nothing() {
    let flags = directives(
        r#"extern "C" {
               unsafe fn abs(x: i32) -> i32;
           }

           fn main() {}
           "#,
    );
    assert!(flags.is_empty(), "no `link` directive, no flag: {flags:?}");
}

#[test]
fn one_library_named_by_two_blocks_is_linked_once() {
    // Two headers can both say they need libm; passing `-lm` twice is legal but
    // wrong in a build command a user reads, and the plan promises one flag per
    // library.
    let flags = directives(
        r#"extern "C" link "m" {
               unsafe fn sqrt(x: float) -> float;
           }

           extern "C" link "m" {
               unsafe fn cos(x: float) -> float;
           }

           fn main() {}
           "#,
    );
    assert_eq!(flags, vec!["-lm".to_string()]);
}

#[test]
fn two_different_libraries_keep_source_order() {
    let flags = directives(
        r#"extern "C" link "m" {
               unsafe fn sqrt(x: float) -> float;
           }

           extern "C" link "pthread" {
               unsafe fn pthread_self() -> usize;
           }

           fn main() {}
           "#,
    );
    assert_eq!(flags, vec!["-lm".to_string(), "-lpthread".to_string()]);
}

fn work_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yashy-link-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("the test work directory must be creatable");
    dir
}

/// Run the executable, or `None` when the operating system refused to start it.
///
/// An Application Control / Device Guard policy blocks a content-defined subset
/// of newly linked executables (`os error 4551`). That verdict is about the
/// machine, not the compiler, and it is retried deterministically for the same
/// bytes, so the execution assertion is skipped rather than reported as a code
/// failure; CI, which has no such policy, runs it for real. The build step
/// above is never skipped, so a link failure still fails this test on every
/// host.
fn run_or_skip(exe: &Path) -> Option<std::process::Output> {
    match common::launch_output(exe) {
        Ok(output) => Some(output),
        Err(error) if is_policy_block(&error) => {
            eprintln!(
                "skipping the execution assertion: an Application Control policy \
                 blocked {} ({error}); the build itself succeeded",
                exe.display()
            );
            None
        }
        Err(error) => panic!(
            "the compiled executable at {} must be runnable: {error}",
            exe.display()
        ),
    }
}

fn is_policy_block(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(4551)
        || error.to_string().contains("blocked")
        || error.to_string().contains("Application Control")
}

#[test]
fn the_same_program_produces_identical_output_on_both_platforms() {
    // `sqrt` is in libm on Linux and in the CRT on Windows. The program names
    // no library, so this passes only if the driver supplies the platform
    // default. `%.17g` is exact for a double on both, so the pinned bytes are
    // meaningful in either CI job.
    let dir = work_dir();
    let source = dir.join("math.ya");
    fs::write(
        &source,
        r#"extern "C" {
    unsafe fn sqrt(x: float) -> float;
}

fn main() {
    print(sqrt(2.0));
}
"#,
    )
    .expect("the fixture must be writable");

    let exe = dir.join(format!("math{}", std::env::consts::EXE_SUFFIX));
    let built = Command::new(common::yashy())
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("the yashy CLI must be runnable");
    assert!(
        built.status.success(),
        "the math program must build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let output = match run_or_skip(&exe) {
        Some(output) => output,
        None => return,
    };
    assert!(output.status.success(), "the math program must exit 0");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "1.4142135623730951",
        "the same program must print the same bytes on every platform"
    );
}
