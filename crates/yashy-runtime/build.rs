//! Compiles `runtime.c` on demand and links it into every dependent crate, so
//! generated IR can call `yashy_print_str` / `yashy_print_i64` /
//! `yashy_print_f64` without a separate library on the link line.

fn main() {
    println!("cargo:rerun-if-changed=runtime.c");
    println!("cargo:rerun-if-changed=hashmap.c");
    println!("cargo:rerun-if-changed=src/executor.c");
    println!("cargo:rerun-if-changed=src/net.c");
    println!("cargo:rerun-if-changed=src/platform.c");
    println!("cargo:rerun-if-changed=include/yashy_runtime.h");
    println!("cargo:rerun-if-changed=src/grapheme_data.h");

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

    // The Windows libraries the runtime's platform layer needs: `bcrypt` for
    // `BCryptGenRandom`, which is where the entropy comes from, and `ws2_32`
    // for the socket layer. The check is on the *target*, not on the host, so a
    // cross build links the libraries of the system it targets.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-lib=dylib=ws2_32");
        println!("cargo:rustc-link-lib=dylib=bcrypt");
    }

    cc::Build::new()
        .file("runtime.c")
        .file("hashmap.c")
        .file("src/executor.c")
        .file("src/net.c")
        .file("src/platform.c")
        .include("include")
        .warnings(true)
        .compile("yashy_runtime");
}
