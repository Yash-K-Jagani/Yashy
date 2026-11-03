//! Calls the runtime's print helpers for real and checks the exact bytes that
//! reached stdout.
//!
//! The C code is only reached through `yashy-runtime`'s `extern` declarations,
//! so this test also proves the static library built by `build.rs` is actually
//! linked into a Rust binary — a broken format string or a stale signature
//! fails here rather than in Task 8 when codegen first emits a call.
//!
//! `harness = false` (see `Cargo.toml`) because the harness owns `main`, and
//! stdout has to be captured by re-running this same binary as a child.

use std::ffi::{c_char, c_double, c_longlong, CStr};

use yashy_runtime::YashyStr;
use std::process::{Command, Stdio};

/// Marker that turns this binary into the child that does the printing.
const CHILD_VAR: &str = "YASHY_RUNTIME_PRINT_CHILD";

/// A static (`refcount -1`) string over a `'static` C-string literal, which is
/// the shape a literal has in a compiled program.
fn literal(text: &'static CStr) -> YashyStr {
    let bytes = text.to_bytes();
    let len = bytes.len() as i64;
    YashyStr {
        refcount: -1,
        len,
        cap: len,
        data: text.as_ptr() as *mut c_char,
    }
}

/// A byte string that contains a NUL, which a C string literal cannot express
/// but a Yashy `string` can. `print_str` must write the bytes the header says
/// are there, not stop at the first zero.
static EMBEDDED_NUL: &[u8] = b"a\0b";

/// The body of a C function in `runtime.c`, between its opening and closing
/// braces. Balanced-brace counting is enough for this file's style: it has no
/// braces inside comments or string literals in the region this is used on.
fn c_function_body<'a>(source: &'a str, signature: &str) -> Option<&'a str> {
    let start = source.find(signature)?;
    let open = start + source[start..].find('{')?;
    let mut depth = 0usize;
    for (offset, byte) in source[open..].bytes().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&source[open..=open + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

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
    // SAFETY: the string values are static literals, and the integers are plain
    // scalars.
    unsafe {
        match case {
            "str" => yashy_runtime::yashy_print_str(&literal(c"Hello, Yashy!")),
            "str_empty" => yashy_runtime::yashy_print_str(&literal(c"")),
            "str_embedded_nul" => {
                let value = YashyStr {
                    refcount: -1,
                    len: EMBEDDED_NUL.len() as i64,
                    cap: EMBEDDED_NUL.len() as i64,
                    data: EMBEDDED_NUL.as_ptr() as *mut c_char,
                };
                yashy_runtime::yashy_print_str(&value);
            }
            "i64_negative" => yashy_runtime::yashy_print_i64(-42 as c_longlong),
            "i64_zero" => yashy_runtime::yashy_print_i64(0 as c_longlong),
            "i64_min" => yashy_runtime::yashy_print_i64(c_longlong::MIN),
            "i64_million" => {
                let before = yashy_runtime::yashy_live_allocations_now();
                for index in 0..1_000_000i64 {
                    yashy_runtime::yashy_print_i64((index % 10) as c_longlong);
                }
                let after = yashy_runtime::yashy_live_allocations_now();
                if after != before {
                    eprintln!(
                        "print_i64 allocated while printing: {} live blocks before, {} after",
                        before, after
                    );
                    std::process::exit(2);
                }
            }
            "f64_exact" => yashy_runtime::yashy_print_f64(3.5 as c_double),
            "f64_round_trip" => yashy_runtime::yashy_print_f64(0.1 as c_double),
            other => eprintln!("unknown case {other}"),
        }
    }
}

fn check_all() -> Result<(), String> {
    let out = capture("str")?;
    expect(
        &out,
        b"Hello, Yashy!",
        "print_str writes the string verbatim",
    )?;
    expect_no_newline(&out, "print_str must not append a newline")?;

    let out = capture("str_empty")?;
    expect(&out, b"", "print_str of an empty string writes nothing")?;

    let out = capture("str_embedded_nul")?;
    expect(
        &out,
        b"a\0b",
        "print_str must use the length it carries, not stop at a NUL",
    )?;

    let out = capture("i64_negative")?;
    expect(&out, b"-42", "print(-42)")?;
    expect_no_newline(&out, "print_i64 must not append a newline")?;

    let out = capture("i64_zero")?;
    expect(&out, b"0", "print(0)")?;

    let out = capture("i64_min")?;
    expect(
        &out,
        b"-9223372036854775808",
        "print(i64::MIN) must not overflow the magnitude",
    )?;

    check_i64_million()?;

    let out = capture("f64_exact")?;
    expect(&out, b"3.5", "print(3.5) must round-trip as 3.5")?;

    // %.17g, not %g: a value that is not representable in fewer digits has to
    // come back with all of them, or it cannot round-trip.
    let out = capture("f64_round_trip")?;
    expect(
        &out,
        b"0.10000000000000001",
        "print(0.1) must carry enough digits to round-trip",
    )?;

    check_print_i64_does_not_format_or_allocate()?;

    println!("runtime print helpers: all cases produced the expected bytes");
    Ok(())
}

/// A million integers are a million digits, and not one heap block.
fn check_i64_million() -> Result<(), String> {
    let out = capture("i64_million")?;
    if out.len() != 1_000_000 {
        return Err(format!(
            "printing 1,000,000 digits produced {} bytes",
            out.len()
        ));
    }
    for (index, byte) in out.iter().enumerate() {
        let wanted = b'0' + (index % 10) as u8;
        if *byte != wanted {
            return Err(format!(
                "byte {index} was {:?}, wanted {:?}",
                *byte as char, wanted as char
            ));
        }
    }
    Ok(())
}

/// The integer path is formatting, not allocation. A source-level check is the
/// only way to say "no `printf`" portably: libc's own allocations are invisible
/// to the runtime's counter, so the million-digit case above cannot see them.
fn check_print_i64_does_not_format_or_allocate() -> Result<(), String> {
    let body = c_function_body(
        yashy_runtime::RUNTIME_C_SOURCE,
        "void yashy_print_i64(long long value)",
    )
    .ok_or_else(|| "runtime.c has no yashy_print_i64 body".to_string())?;
    for forbidden in [
        "printf",
        "sprintf",
        "snprintf",
        "malloc",
        "calloc",
        "yashy_alloc",
    ] {
        if body.contains(forbidden) {
            return Err(format!(
                "yashy_print_i64 must not use `{forbidden}`; its body is:\n{body}"
            ));
        }
    }
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
