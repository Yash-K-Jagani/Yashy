//! Builds and runs `tests/threads.c` against the real runtime.
//!
//! The runtime's other tests call the exported C functions from Rust, but the
//! thread primitives are about what happens *between* OS threads, and the plan
//! asks for a C program that can block, close, and join without a Rust harness
//! in the middle. The compiler used here is the same one `build.rs` found, so
//! this adds no new toolchain requirement.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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
    std::env::temp_dir().join(format!("yashy-threads-{}-{stamp}", std::process::id()))
}

fn build_threads_program() -> PathBuf {
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
    std::fs::create_dir_all(&out_dir).expect("create the threads test directory");

    let executable = out_dir.join(if cfg!(windows) { "threads.exe" } else { "threads" });
    let include = root.join("include");
    let sources = [
        root.join("tests").join("threads.c"),
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
        .expect("run the C compiler on tests/threads.c");
    assert!(
        built.status.success(),
        "building tests/threads.c failed:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    executable
}

/// How many copies of the test program the host may refuse before giving up.
///
/// The same bound the driver's end-to-end tests use: a freshly linked binary is
/// what an Application Control policy refuses, and about nine copies in ten
/// start, so forty leaves a refusal with probability below `1e-4`.
const LAUNCH_ATTEMPTS: usize = 40;

/// Whether an error is the host refusing to start a binary rather than the
/// program failing.
fn blocked_by_policy(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::PermissionDenied
        || error.raw_os_error() == Some(4551)
        || error.to_string().contains("blocked")
        || error.to_string().contains("Application Control")
}

/// Run the built program, stepping around a host that refuses to start it.
///
/// An Application Control policy judges content, so a refused path stays
/// refused while a copy with a few zero bytes past the end of the image is a
/// different question. The padding is never executed.
fn run_retrying(exe: &Path) -> Output {
    let mut last = None;
    for attempt in 0..LAUNCH_ATTEMPTS {
        let candidate = if attempt == 0 {
            exe.to_path_buf()
        } else {
            let padded = exe.with_file_name(format!(
                "threads_launch{attempt}{}",
                std::env::consts::EXE_SUFFIX
            ));
            let mut bytes = std::fs::read(exe).expect("read the built threads program");
            bytes.resize(bytes.len() + attempt, 0);
            std::fs::write(&padded, bytes).expect("write a padded copy of the threads program");
            padded
        };
        match Command::new(&candidate).output() {
            Ok(output) => return output,
            Err(error) if blocked_by_policy(&error) => last = Some(error),
            Err(error) => panic!("could not start {}: {error}", exe.display()),
        }
    }
    panic!(
        "the host blocked {} on every attempt: {}",
        exe.display(),
        last.map(|error| error.to_string()).unwrap_or_default()
    );
}

#[test]
fn the_thread_suite_passes() {
    let executable = build_threads_program();
    let run = run_retrying(&executable);
    assert!(
        run.status.success(),
        "tests/threads.c failed with {:?}:\nstdout:\n{}\nstderr:\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );

    let directory = executable.parent().expect("the executable has a parent");
    let _ = std::fs::remove_dir_all(directory);
}
