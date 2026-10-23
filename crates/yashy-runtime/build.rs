//! Compiles `runtime.c` on demand and links it into every dependent crate, so
//! generated IR can call `yashy_print_str` / `yashy_print_i64` /
//! `yashy_print_f64` without a separate library on the link line.

fn main() {
    println!("cargo:rerun-if-changed=runtime.c");
    println!("cargo:rerun-if-changed=hashmap.c");
    println!("cargo:rerun-if-changed=include/yashy_runtime.h");

    // `tests/threads.rs` compiles `tests/threads.c` at test time with `cc`,
    // which expects the environment a build script gets. Cargo sets `TARGET`
    // and `HOST` for this script but not for the test binary, and the triple is
    // the one piece a `cfg!` cannot reconstruct, so pass it through.
    println!(
        "cargo:rustc-env=YASHY_TARGET={}",
        std::env::var("TARGET").expect("Cargo sets TARGET for build scripts")
    );
    println!(
        "cargo:rustc-env=YASHY_HOST={}",
        std::env::var("HOST").expect("Cargo sets HOST for build scripts")
    );

    cc::Build::new()
        .file("runtime.c")
        .file("hashmap.c")
        .include("include")
        .warnings(true)
        .compile("yashy_runtime");
}
