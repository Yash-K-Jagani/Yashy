//! Compiles `runtime.c` on demand and links it into every dependent crate, so
//! generated IR can call `otter_print_str` / `otter_print_i64` /
//! `otter_print_f64` without a separate library on the link line.

fn main() {
    println!("cargo:rerun-if-changed=runtime.c");
    println!("cargo:rerun-if-changed=hashmap.c");
    println!("cargo:rerun-if-changed=include/otter_runtime.h");

    cc::Build::new()
        .file("runtime.c")
        .file("hashmap.c")
        .include("include")
        .warnings(true)
        .compile("otter_runtime");
}
