//! `Encode`/`Decode` derives at run time: a struct round-trips through a
//! deterministic wire form, and a malformed wire is an error value rather
//! than a fault.

mod common;

use std::process::Command;

fn run(name: &str, body: &str) -> (String, String, bool) {
    let dir = common::work_dir(name);
    let source = format!(
        "struct Pair derive(Encode, Decode) {{ a: int, b: int }}\nfn main() {{\n{body}\n}}\n"
    );
    common::write(&dir, "main.ya", &source);
    let exe = dir.join(format!("main{}", std::env::consts::EXE_SUFFIX));
    let build = Command::new(common::yashyc())
        .arg("build")
        .arg(dir.join("main.ya"))
        .arg("-o")
        .arg(&exe)
        .current_dir(&dir)
        .output()
        .expect("the compiler must run");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let output = Command::new(&exe).output().expect("the program must run");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

#[test]
fn a_derived_codec_round_trips() {
    let body = "let p = Pair { a: 42, b: -7 }; \
                let wire = p.encode(); \
                print(wire); \
                switch Pair::decode(wire) { \
                    case Ok(q) { print(str_from_i64(q.a) + \":\" + str_from_i64(q.b)); } \
                    case Err(_) { print(\"bad\"); } \
                };";
    let (stdout, stderr, ok) = run("codec-derive-roundtrip", body);
    assert!(ok, "{stderr}");
    assert_eq!(stdout, "42:-7\n42:-7\n");
}

#[test]
fn a_malformed_wire_is_an_error_value() {
    let body = "switch Pair::decode(\"not:an:int\") { \
                    case Ok(_) { print(\"unexpected\"); } \
                    case Err(_) { print(\"rejected\"); } \
                };";
    let (stdout, stderr, ok) = run("codec-derive-malformed", body);
    assert!(ok, "{stderr}");
    assert_eq!(stdout, "rejected\n");
}

#[test]
fn a_malformed_wire_names_the_typed_codec_error() {
    let body = "switch Pair::decode(\"1\") { \
                    case Ok(_) { print(\"unexpected\"); } \
                    case Err(error) { \
                        switch error { \
                            case codec.CodecError.BadFieldCount { print(\"count\"); } \
                            case codec.CodecError.BadField { print(\"field\"); } \
                        } \
                    } \
                };";
    let (stdout, stderr, ok) = run("codec-derive-typed-error", body);
    assert!(ok, "{stderr}");
    assert_eq!(stdout, "count\n");
}

#[test]
fn a_wrong_field_count_is_rejected() {
    let body = "switch Pair::decode(\"1\") { \
                    case Ok(_) { print(\"unexpected\"); } \
                    case Err(_) { print(\"rejected\"); } \
                };";
    let (stdout, stderr, ok) = run("codec-derive-count", body);
    assert!(ok, "{stderr}");
    assert_eq!(stdout, "rejected\n");
}
