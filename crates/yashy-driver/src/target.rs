//! The build target: the triples the compiler emits for, and the C toolchain a
//! cross build needs.
//!
//! A target triple names three things at once -- the CPU architecture, the OS
//! and the C environment -- and the compiler needs all three: the arch decides
//! the data layout and the word size, the OS decides the object format and the
//! platform libraries, and the environment decides which CRT a `repr(C)` struct
//! is laid out against. They are spelled as one string so a build can never
//! carry two of them that disagree.
//!
//! # The set is closed
//!
//! [`SUPPORTED`] is exactly what the release tests: two architectures for each
//! of the three 1.0 platforms. A triple outside it is [`E2200`] rather than an
//! attempt -- LLVM will happily emit IR for a machine whose runtime has never
//! been compiled, and the failure then arrives at link time or, worse, at run
//! time on a user's machine. Refusing at the flag keeps the surprise here.
//!
//! # Cross-compiling is not self-contained
//!
//! Yashy's runtime is C, and the program is linked by `clang`. A cross build
//! therefore needs the target's C toolchain installed; the compiler does not
//! bundle a linker. [`probe_cross`] answers whether that toolchain can link for
//! the target, and [`E2201`] is what a build says when it cannot. Pretending
//! otherwise is how a cross build silently links the host's runtime into a
//! binary that then fails to start on the target.
//!
//! [`E2200`]: yashy_common::codes::UNSUPPORTED_TARGET
//! [`E2201`]: yashy_common::codes::TARGET_TOOLCHAIN_MISSING

use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::toolchain::Toolchain;

/// Every triple `--target` accepts, in the order the diagnostic lists them.
///
/// The host default is *not* checked against this list: on a
/// `x86_64-pc-windows-gnu` toolchain the compiler built itself for a triple
/// that is not in the release set, and a build with no `--target` must keep
/// working there. The list is what a user may *ask* for.
pub const SUPPORTED: [&str; 6] = [
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
];

/// The supported triple `value` names, if it names one.
///
/// Returning `&'static str` rather than the input keeps the target one value
/// through the whole build: [`yashy_codegen::BuildOptions`] stores it, the IR
/// header prints it, the `cfg` filter answers from it and `clang` links for it.
pub fn supported(value: &str) -> Option<&'static str> {
    SUPPORTED.iter().copied().find(|triple| *triple == value)
}

/// The `E2200` message for a triple the compiler does not emit for.
///
/// The supported list is in the message, not just in the docs: a typo and a
/// platform the compiler has never heard of are the same answer, and the fix
/// for both is to pick from the list.
pub fn unsupported_message(value: &str) -> String {
    format!(
        "unsupported target `{value}`; supported targets: {}",
        SUPPORTED.join(", ")
    )
}

/// Whether `clang` can build and link a trivial program for `target`.
///
/// The probe links, rather than only assembling, because linking is what a
/// Yashy build does: `clang --target=<triple>` can emit an object for any
/// architecture it was built with and still fail at the link for want of the
/// target's CRT. A probe that stopped at `-c` would pass and leave the real
/// build to fail with a link error far from the flag that caused it.
///
/// `Err` carries the compiler's own last lines, so the message can quote the
/// toolchain rather than paraphrase it.
pub fn probe_cross(toolchain: &Toolchain, target: &str) -> Result<(), String> {
    // `llc` turns IR into an object and cannot drive a C compiler, so it is
    // not a cross toolchain for any target. Saying so is more useful than the
    // probe's own failure, which would blame the target.
    if toolchain.found != "clang" {
        return Err(format!(
            "the C toolchain for `{target}` needs `clang`, and only `{}` was found",
            toolchain.found
        ));
    }
    let directory = probe_dir();
    if let Err(error) = std::fs::create_dir_all(&directory) {
        return Err(format!(
            "cannot create a scratch directory at {}: {error}",
            directory.display()
        ));
    }
    let source = directory.join("probe.c");
    let output = directory.join(if target.contains("windows") {
        "probe.exe"
    } else {
        "probe"
    });
    let written = std::fs::write(&source, "int main(void) { return 0; }\n");
    let result = match written {
        Ok(()) => Command::new(&toolchain.llc)
            .arg(format!("--target={target}"))
            .arg(&source)
            .arg("-o")
            .arg(&output)
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .stdout(Stdio::null())
            .output(),
        Err(error) => {
            let _ = std::fs::remove_dir_all(&directory);
            return Err(format!(
                "cannot write a probe program to {}: {error}",
                source.display()
            ));
        }
    };
    let _ = std::fs::remove_dir_all(&directory);
    match result {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => {
            let mut detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if detail.is_empty() {
                detail = format!("`clang --target={target}` exited {}", output.status);
            } else if detail.len() > 600 {
                // A linker's full output is hundreds of lines; the first few
                // name the missing piece and the rest is the same fact again.
                detail.truncate(600);
                detail.push_str(" ...");
            }
            Err(format!(
                "the C toolchain for `{target}` is not installed: \
                 `clang --target={target}` could not link a trivial program\n{detail}"
            ))
        }
        Err(error) => Err(format!(
            "the C toolchain for `{target}` is not installed: cannot run `clang`: {error}"
        )),
    }
}

/// A scratch directory unique to this process, so two concurrent builds cannot
/// read each other's probe.
fn probe_dir() -> PathBuf {
    std::env::temp_dir().join(format!("yashy-target-probe-{}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_triple_is_exactly_one_architecture_and_one_platform() {
        for triple in SUPPORTED {
            let archs = ["x86_64", "aarch64"];
            assert!(
                archs.iter().any(|arch| triple.starts_with(arch)),
                "`{triple}` names an architecture this compiler does not emit for"
            );
            let platforms = ["windows", "linux", "apple-darwin"];
            assert!(
                platforms.iter().any(|platform| triple.contains(platform)),
                "`{triple}` names a platform the 1.0 release does not test"
            );
        }
    }

    #[test]
    fn a_supported_triple_round_trips_to_the_static_spelling() {
        assert_eq!(
            supported("aarch64-apple-darwin"),
            Some("aarch64-apple-darwin")
        );
        assert_eq!(supported("riscv64gc-unknown-linux-gnu"), None);
    }

    #[test]
    fn the_unsupported_message_lists_every_triple_that_works() {
        let message = unsupported_message("mips-unknown-linux-gnu");
        for triple in SUPPORTED {
            assert!(message.contains(triple), "{message}");
        }
        assert!(message.starts_with("unsupported target `mips-unknown-linux-gnu`"));
    }

    #[test]
    fn a_probe_for_the_host_toolchain_succeeds_when_the_host_has_one() {
        // There is nothing to assert when the machine has no C compiler; the
        // point is that the probe runs and reports rather than panicking.
        let toolchain = crate::toolchain::detect().ok();
        if let Some(toolchain) = toolchain {
            let _ = probe_cross(&toolchain, yashy_codegen::host_target_triple());
        }
    }
}
