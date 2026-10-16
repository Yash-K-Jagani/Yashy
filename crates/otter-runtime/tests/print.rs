//! Calls the runtime's print helpers for real and checks the exact bytes that
//! reached stdout.
//!
//! The C code is only reached through `otter-runtime`'s `extern` declarations,
//! so this test also proves the static library built by `build.rs` is actually
//! linked into a Rust binary — a broken format string or a stale signature
//! fails here rather than in Task 8 when codegen first emits a call.
//!
//! `harness = false` (see `Cargo.toml`) because the harness owns `main`, and
//! stdout has to be captured by re-running this same binary as a child.

use std::ffi::{c_double, c_longlong};
use std::process::{Command, Stdio};

/// Marker that turns this binary into the child that does the printing.
const CHILD_VAR: &str = "OTTER_RUNTIME_PRINT_CHILD";

fn main() {
    if std::env::var_os(CHILD_VAR).is_some() {
        emit(std::env::args().nth(1).as_deref().unwrap_or(""));
        return;
    }
    if let Err(message) = check_all() {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

/// The child's whole job: call one runtime helper and let the C stdio buffer
/// flush at exit.
fn emit(case: &str) {
    // SAFETY: every pointer handed over here is a NUL-terminated `'static`
    // literal, and the integers are plain scalars.
    unsafe {
        match case {
            "str" => otter_runtime::otter_print_str(c"Hello, Otter!".as_ptr()),
            "str_empty" => otter_runtime::otter_print_str(c"".as_ptr()),
            "i64_negative" => otter_runtime::otter_print_i64(-42 as c_longlong),
            "i64_zero" => otter_runtime::otter_print_i64(0 as c_longlong),
            "f64_exact" => otter_runtime::otter_print_f64(3.5 as c_double),
            "f64_round_trip" => otter_runtime::otter_print_f64(0.1 as c_double),
            other => eprintln!("unknown case {other}"),
        }
    }
}

fn check_all() -> Result<(), String> {
    let out = capture("str")?;
    expect(&out, b"Hello, Otter!", "print_str writes the string verbatim")?;
    expect_no_newline(&out, "print_str must not append a newline")?;

    let out = capture("str_empty")?;
    expect(&out, b"", "print_str of an empty string writes nothing")?;

    let out = capture("i64_negative")?;
    expect(&out, b"-42", "print_i64(-42)")?;
    expect_no_newline(&out, "print_i64 must not append a newline")?;

    let out = capture("i64_zero")?;
    expect(&out, b"0", "print_i64(0)")?;

    let out = capture("f64_exact")?;
    expect(&out, b"3.5", "print_f64(3.5) must round-trip as 3.5")?;

    // %.17g, not %g: a value that is not representable in fewer digits has to
    // come back with all of them, or it cannot round-trip.
    let out = capture("f64_round_trip")?;
    expect(
        &out,
        b"0.10000000000000001",
        "print_f64(0.1) must carry enough digits to round-trip",
    )?;

    println!("runtime print helpers: all cases produced the expected bytes");
    Ok(())
}

/// Runs this binary again as the printing child and returns what it wrote.
fn capture(case: &str) -> Result<Vec<u8>, String> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("cannot locate this test binary to re-run it: {error}"))?;

    let output = Command::new(&exe)
        .arg(case)
        .env(CHILD_VAR, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("cannot run {} {case}: {error}", exe.display()))?;

    if !output.status.success() {
        return Err(format!(
            "child for {case} exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    if !output.stderr.is_empty() {
        return Err(format!(
            "child for {case} wrote to stderr: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

fn expect(actual: &[u8], wanted: &[u8], what: &str) -> Result<(), String> {
    if actual == wanted {
        return Ok(());
    }
    Err(format!(
        "{what}\n  expected: {:?}\n  actual:   {:?}",
        String::from_utf8_lossy(wanted),
        String::from_utf8_lossy(actual)
    ))
}

fn expect_no_newline(actual: &[u8], what: &str) -> Result<(), String> {
    if actual.contains(&b'\n') {
        return Err(format!(
            "{what}, but output was {:?}",
            String::from_utf8_lossy(actual)
        ));
    }
    Ok(())
}
