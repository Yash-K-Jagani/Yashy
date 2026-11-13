//! The stdlib is a package, and its contribution to a build is visible.
//!
//! These tests answer three questions the plan for the precompiled stdlib
//! asks. Is the stdlib versioned like a package? Does a program that imports
//! nothing from it load no stdlib module? Does the emitted IR come from the
//! content-addressed cache on a second build, and is that build byte-for-byte
//! the one a from-source build produces?
//!
//! Every test owns a private `YASHY_CACHE_DIR`, so the warm and cold builds it
//! compares cannot see another test's artifacts.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::{exe, launch_output, work_dir, yashy};
use yashy_manifest::Manifest;

/// The repository root: `crates/yashy-driver/tests/stdlib_precompiled.rs`.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Run the CLI from the repository root so an entry path in the IR key is the
/// same string for every build in one test.
fn cli(cache: &Path, stdlib: Option<&Path>, args: &[&str]) -> Output {
    let mut command = Command::new(yashy());
    command
        .args(args)
        .current_dir(root())
        .env("YASHY_CACHE_DIR", cache);
    if let Some(stdlib) = stdlib {
        command.env("YASHY_STDLIB", stdlib);
    }
    command.output().expect("the yashy CLI must be runnable")
}

/// Build `entry` to `out` with `--timings`, optionally from source.
fn build(cache: &Path, stdlib: Option<&Path>, entry: &str, out: &Path, extra: &[&str]) -> Output {
    let out_text = out.to_string_lossy().into_owned();
    let mut args = vec!["build", entry, "-o", out_text.as_str(), "--timings"];
    args.extend_from_slice(extra);
    cli(cache, stdlib, &args)
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn the_stdlib_declares_itself_as_a_versioned_package() {
    let manifest = Manifest::load(&root().join("crates/yashy-stdlib/yashy.toml"))
        .expect("the stdlib must have a manifest");
    assert_eq!(manifest.package.name, "stdlib");
    assert_eq!(manifest.package.version, "1.0.0");
    // The compiler's fallback constant and the shipped manifest agree.
    assert_eq!(yashy_driver::modules::stdlib_version(), "1.0.0");
}

#[test]
fn hello_loads_no_stdlib_module() {
    let cache = work_dir("stdlib-hello-cache");
    let out = exe(&work_dir("stdlib-hello"), "hello");
    let output = build(&cache, None, "examples/hello/main.ya", &out, &[]);
    assert!(
        output.status.success(),
        "hello must build:\n{}",
        stderr(&output)
    );
    let log = stderr(&output);
    // The stdlib is not ambient: `hello` imports nothing, so it pays for no
    // stdlib module and never typechecks one.
    assert!(
        log.contains("stdlib_modules=0"),
        "hello must report no stdlib modules:\n{log}"
    );
}

#[test]
fn a_warm_ir_cache_skips_emit_and_reproduces_the_binary() {
    let cache = work_dir("stdlib-ir-cache");
    let dir = work_dir("stdlib-ir-out");
    let first = exe(&dir, "first");
    let second = exe(&dir, "second");
    let from_source = exe(&dir, "from_source");

    let cold = build(&cache, None, "examples/json-report/main.ya", &first, &[]);
    assert!(
        cold.status.success(),
        "the cold build must pass:\n{}",
        stderr(&cold)
    );
    let cold_log = stderr(&cold);
    assert!(
        cold_log.contains("ir_cache=miss"),
        "the first build must emit:\n{cold_log}"
    );
    // `json-report` imports `JSON`, so exactly one stdlib module is loaded.
    assert!(
        cold_log.contains("stdlib_modules=1"),
        "importing JSON must load one stdlib module:\n{cold_log}"
    );

    let warm = build(&cache, None, "examples/json-report/main.ya", &second, &[]);
    assert!(
        warm.status.success(),
        "the warm build must pass:\n{}",
        stderr(&warm)
    );
    let warm_log = stderr(&warm);
    assert!(
        warm_log.contains("ir_cache=hit"),
        "the second build must reuse the IR:\n{warm_log}"
    );
    assert!(
        warm_log.contains("timing: emit 0 ms"),
        "a cache hit must not emit:\n{warm_log}"
    );

    // The escape hatch never reads or writes the cache, so it reports no
    // decision at all and must still produce the same bytes.
    let source_build = build(
        &cache,
        None,
        "examples/json-report/main.ya",
        &from_source,
        &["--stdlib-source"],
    );
    assert!(
        source_build.status.success(),
        "a from-source build must pass:\n{}",
        stderr(&source_build)
    );
    let source_log = stderr(&source_build);
    assert!(
        !source_log.contains("ir_cache="),
        "`--stdlib-source` must bypass the cache:\n{source_log}"
    );

    // The IR each build handed to the linker, compared before the images are:
    // identical `.ll` files with differing images put the difference in the
    // link step, while a differing `.ll` puts it in the front end. The text is
    // written beside every output as a debugging aid, so it is already there.
    let cold_ir = fs::read(first.with_extension("ll")).expect("the cold build's IR must exist");
    let warm_ir = fs::read(second.with_extension("ll")).expect("the warm build's IR must exist");
    let source_ir =
        fs::read(from_source.with_extension("ll")).expect("the from-source build's IR must exist");
    assert_same_ir(&cold_ir, &warm_ir, "cold", "warm");
    assert_same_ir(&cold_ir, &source_ir, "cold", "from-source");

    let cold_bytes = fs::read(&first).expect("the first binary must exist");
    let warm_bytes = fs::read(&second).expect("the second binary must exist");
    let source_bytes = fs::read(&from_source).expect("the from-source binary must exist");
    assert_eq!(
        cold_bytes.len(),
        warm_bytes.len(),
        "a cache hit must produce the same image size"
    );
    assert_eq!(
        cold_bytes.len(),
        source_bytes.len(),
        "a from-source build must produce the same image size"
    );
    assert_same_binary(
        &without_link_identity(&cold_bytes),
        &without_link_identity(&warm_bytes),
        "a cache hit must reproduce the binary",
    );
    assert_same_binary(
        &without_link_identity(&cold_bytes),
        &without_link_identity(&source_bytes),
        "a from-source build must reproduce the cached binary",
    );

    // Both binaries agree about what the program prints, not just about the
    // bytes they were built from.
    let cached_run = launch_output(&first).expect("the cached binary must start");
    let source_run = launch_output(&from_source).expect("the from-source binary must start");
    assert!(cached_run.status.success(), "the cached binary must run");
    assert!(
        source_run.status.success(),
        "the from-source binary must run"
    );
    assert_eq!(cached_run.stdout, source_run.stdout);
}

#[test]
fn both_linker_clock_fields_are_cleared_from_a_pe_image() {
    // A synthetic PE32 image with one section whose raw data holds a
    // two-entry debug directory. The two clock readings are the COFF header's
    // and one per debug entry; every other byte must survive untouched, which
    // is what makes the comparison in the test above about the compiler rather
    // than about the linker's clock.
    let mut image = vec![0u8; 0x600];
    image[0..2].copy_from_slice(b"MZ");
    image[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    image[0x40..0x44].copy_from_slice(b"PE\0\0");
    image[0x46..0x48].copy_from_slice(&1u16.to_le_bytes()); // one section
    image[0x48..0x4c].copy_from_slice(&0x6af6_c98du32.to_le_bytes()); // header stamp
    image[0x54..0x56].copy_from_slice(&0xe0u16.to_le_bytes()); // PE32 optional header
    image[0x58..0x5a].copy_from_slice(&0x010bu16.to_le_bytes()); // PE32 magic
    image[0xe8..0xec].copy_from_slice(&0x1000u32.to_le_bytes()); // debug directory RVA
    image[0xec..0xf0].copy_from_slice(&56u32.to_le_bytes()); // two 28-byte entries
    image[0x144..0x148].copy_from_slice(&0x1000u32.to_le_bytes()); // section address
    image[0x148..0x14c].copy_from_slice(&0x400u32.to_le_bytes()); // section raw size
    image[0x14c..0x150].copy_from_slice(&0x200u32.to_le_bytes()); // section raw offset
    image[0x200..0x238].fill(0xaa);
    image[0x204..0x208].copy_from_slice(&0x6af6_c98du32.to_le_bytes());
    image[0x220..0x224].copy_from_slice(&0x6af6_ffffu32.to_le_bytes());

    let cleared = without_link_timestamp(&image);
    assert_eq!(&cleared[0x48..0x4c], &[0, 0, 0, 0], "the header stamp");
    assert_eq!(&cleared[0x204..0x208], &[0, 0, 0, 0], "the first entry");
    assert_eq!(&cleared[0x220..0x224], &[0, 0, 0, 0], "the second entry");
    for (offset, (&before, &after)) in image.iter().zip(&cleared).enumerate() {
        let clock = (0x48..0x4c).contains(&offset)
            || (0x204..0x208).contains(&offset)
            || (0x220..0x224).contains(&offset);
        assert!(
            clock || before == after,
            "byte {offset} must be untouched ({before} became {after})"
        );
    }
    assert_eq!(
        without_link_timestamp(b"not a PE image").as_slice(),
        b"not a PE image",
        "a non-PE image is compared as it is"
    );
}

#[test]
fn the_random_macho_uuid_is_cleared_from_a_mach_o_image() {
    // A 64-bit Mach-O header with two load commands: an `LC_UUID` whose
    // 16-byte value is the linker's own identity, and a command whose
    // payload is not an identity and must survive untouched.
    let mut image = vec![0u8; 0x80];
    image[0..4].copy_from_slice(&0xfeed_facfu32.to_le_bytes());
    image[16..20].copy_from_slice(&2u32.to_le_bytes()); // ncmds
    image[32..36].copy_from_slice(&0x1bu32.to_le_bytes()); // LC_UUID
    image[36..40].copy_from_slice(&24u32.to_le_bytes());
    image[40..56].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
    image[56..60].copy_from_slice(&0x0eu32.to_le_bytes()); // LC_LOAD_DYLINKER
    image[60..64].copy_from_slice(&32u32.to_le_bytes());
    image[64..80].fill(0x5a);

    let cleared = without_link_identity(&image);
    assert_eq!(&cleared[40..56], &[0u8; 16], "the UUID must be zeroed");
    assert_eq!(&cleared[64..80], &image[64..80], "other commands survive");
    assert_eq!(
        without_macho_uuid(b"not a Mach-O image").as_slice(),
        b"not a Mach-O image",
        "a non-Mach-O image is compared as it is"
    );
}

#[test]
fn a_stdlib_version_change_invalidates_the_ir_cache() {
    // A private copy of the stdlib, so the manifest can be edited without
    // touching the repository.
    let copy = work_dir("stdlib-copy");
    fs::create_dir_all(&copy).expect("the copy must be creatable");
    copy_tree(
        &root().join("crates/yashy-stdlib/yashy"),
        &copy.join("yashy"),
    );
    fs::copy(
        root().join("crates/yashy-stdlib/yashy.toml"),
        copy.join("yashy.toml"),
    )
    .expect("the manifest must be copied");
    let stdlib = copy.join("yashy");

    let cache = work_dir("stdlib-version-cache");
    let dir = work_dir("stdlib-version-out");
    let first = exe(&dir, "first");
    let second = exe(&dir, "second");
    let third = exe(&dir, "third");

    let cold = build(
        &cache,
        Some(&stdlib),
        "examples/json-report/main.ya",
        &first,
        &[],
    );
    assert!(
        cold.status.success(),
        "the cold build must pass:\n{}",
        stderr(&cold)
    );
    assert!(
        stderr(&cold).contains("ir_cache=miss"),
        "the first build must emit:\n{}",
        stderr(&cold)
    );

    let warm = build(
        &cache,
        Some(&stdlib),
        "examples/json-report/main.ya",
        &second,
        &[],
    );
    assert!(
        warm.status.success(),
        "the warm build must pass:\n{}",
        stderr(&warm)
    );
    assert!(
        stderr(&warm).contains("ir_cache=hit"),
        "an unchanged stdlib must hit the cache:\n{}",
        stderr(&warm)
    );

    // Only the manifest changes. The module bytes are identical, so a cache
    // key without the stdlib version would serve the old IR here.
    fs::write(
        copy.join("yashy.toml"),
        "[package]\nname = \"stdlib\"\nversion = \"1.0.1\"\n",
    )
    .expect("the copied manifest must be writable");
    let rebuilt = build(
        &cache,
        Some(&stdlib),
        "examples/json-report/main.ya",
        &third,
        &[],
    );
    assert!(
        rebuilt.status.success(),
        "the build after a stdlib bump must pass:\n{}",
        stderr(&rebuilt)
    );
    assert!(
        stderr(&rebuilt).contains("ir_cache=miss"),
        "a changed stdlib version must invalidate the entry:\n{}",
        stderr(&rebuilt)
    );
}

/// The file offset of the `PE\0\0` signature, or `None` for a non-PE image.
fn pe_offset(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 0x40 || &bytes[0..2] != b"MZ" {
        return None;
    }
    let offset = u32::from_le_bytes([bytes[0x3c], bytes[0x3d], bytes[0x3e], bytes[0x3f]]) as usize;
    if offset + 4 <= bytes.len() && &bytes[offset..offset + 4] == b"PE\0\0" {
        Some(offset)
    } else {
        None
    }
}

/// Normalise the link step's own identity so two images can be compared for
/// the bytes the compiler contributed.
///
/// A PE image carries a clock, and a Mach-O image carries an `LC_UUID` that
/// Apple's `ld` derives from its own inputs; neither is something the
/// compiler decided. `--reproducible` removes both; this helper ignores them
/// for the ordinary build the warm-cache test makes.
fn without_link_identity(bytes: &[u8]) -> Vec<u8> {
    let bytes = without_macho_uuid(bytes);
    without_link_timestamp(&bytes)
}

/// Zero every `LC_UUID` payload in a single-architecture Mach-O image.
///
/// The load commands are a chain: a 32-byte header (28 for a 32-bit image)
/// followed by `ncmds` records of `cmd` and `cmdsize`. `LC_UUID` is `0x1b` and
/// its 16-byte payload follows the 8-byte command header. A universal image
/// names its slices through a fat header instead; this leaves one alone, so
/// the comparison then fails on the UUID rather than quietly calling unequal
/// images equal.
fn without_macho_uuid(bytes: &[u8]) -> Vec<u8> {
    let mut bytes = bytes.to_vec();
    if bytes.len() < 28 {
        return bytes;
    }
    let magic = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let header = match magic {
        0xfeed_facf => 32,
        0xfeed_face => 28,
        _ => return bytes,
    };
    if bytes.len() < header {
        return bytes;
    }
    let commands = u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]) as usize;
    let mut at = header;
    for _ in 0..commands {
        if at + 8 > bytes.len() {
            return bytes;
        }
        let command = u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        let size = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]])
            as usize;
        if size < 8 || at + size > bytes.len() {
            return bytes;
        }
        if command == 0x1b && size >= 24 {
            bytes[at + 8..at + 24].fill(0);
        }
        at += size;
    }
    bytes
}

/// A binary with the fields the linker fills in from the clock cleared.
///
/// A PE image carries two readings of the link step's clock: `TimeDateStamp`
/// in the COFF header, and one in every debug directory entry. Some links
/// also stamp a POGO record, whose entry lives in `.rdata`, so clearing only
/// the header leaves two identical builds made a second apart differing in
/// the middle of the image. Everything the compiler controls is still
/// compared byte-for-byte. A non-PE image is returned unchanged: ELF embeds no
/// timestamp, and a GNU build-id is a hash of the image contents, so identical
/// builds are identical files there.
fn without_link_timestamp(bytes: &[u8]) -> Vec<u8> {
    let mut bytes = bytes.to_vec();
    let Some(pe) = pe_offset(&bytes) else {
        return bytes;
    };
    // The COFF header starts just past the four-byte signature, and its
    // `TimeDateStamp` is the third field.
    let stamp = pe + 8;
    if stamp + 4 <= bytes.len() {
        bytes[stamp..stamp + 4].fill(0);
    }
    // The debug directory is data directory 6, which starts after 96 bytes of
    // PE32 optional header or 112 bytes of PE32+; each 28-byte entry carries
    // the same clock reading at offset 4.
    let optional = pe + 24;
    if optional + 2 > bytes.len() {
        return bytes;
    }
    let directories = match u16::from_le_bytes([bytes[optional], bytes[optional + 1]]) {
        0x010b => optional + 96,
        0x020b => optional + 112,
        _ => return bytes,
    };
    let directory = directories + 6 * 8;
    if directory + 8 > bytes.len() {
        return bytes;
    }
    let start = u32::from_le_bytes([
        bytes[directory],
        bytes[directory + 1],
        bytes[directory + 2],
        bytes[directory + 3],
    ]) as usize;
    let size = u32::from_le_bytes([
        bytes[directory + 4],
        bytes[directory + 5],
        bytes[directory + 6],
        bytes[directory + 7],
    ]) as usize;
    let Some(start) = rva_to_file_offset(&bytes, pe, start) else {
        return bytes;
    };
    for entry in (start..start + size).step_by(28) {
        if entry + 28 > bytes.len() {
            break;
        }
        bytes[entry + 4..entry + 8].fill(0);
    }
    bytes
}

/// Map a virtual address to a file offset through the section table.
///
/// The debug directory is named by an address in the optional header, not by a
/// file offset, so the section that backs it has to be found first.
fn rva_to_file_offset(bytes: &[u8], pe: usize, rva: usize) -> Option<usize> {
    let sections = usize::from(u16::from_le_bytes([bytes[pe + 6], bytes[pe + 7]]));
    let optional = usize::from(u16::from_le_bytes([bytes[pe + 20], bytes[pe + 21]]));
    for index in 0..sections {
        let entry = pe + 24 + optional + 40 * index;
        if entry + 40 > bytes.len() {
            return None;
        }
        let raw = u32::from_le_bytes([
            bytes[entry + 20],
            bytes[entry + 21],
            bytes[entry + 22],
            bytes[entry + 23],
        ]) as usize;
        let size = u32::from_le_bytes([
            bytes[entry + 16],
            bytes[entry + 17],
            bytes[entry + 18],
            bytes[entry + 19],
        ]) as usize;
        let address = u32::from_le_bytes([
            bytes[entry + 12],
            bytes[entry + 13],
            bytes[entry + 14],
            bytes[entry + 15],
        ]) as usize;
        if rva >= address && rva < address + size {
            return Some(raw + (rva - address));
        }
    }
    None
}

/// Assert two IR texts are identical, naming the first line that parted.
///
/// The `.ll` beside each output is the exact text that build handed to the
/// linker, which makes it the discriminator when two images differ: a
/// mismatch here blames the front end, a match blames the link step. Printing
/// the line from each is usually enough to see which.
fn assert_same_ir(left: &[u8], right: &[u8], left_name: &str, right_name: &str) {
    if left == right {
        return;
    }
    let Some(offset) = left.iter().zip(right).position(|(a, b)| a != b) else {
        panic!(
            "the {left_name} and {right_name} IR differ in length only ({} and {} bytes)",
            left.len(),
            right.len()
        );
    };
    let (line, column) = line_and_column(left, offset);
    panic!(
        "the {left_name} and {right_name} IR differ at byte {offset} of {} \
         (line {line}, column {column})\n  {left_name}: {}\n  {right_name}: {}",
        left.len(),
        line_at(left, line),
        line_at(right, line),
    );
}

/// The one-based line and column a byte offset falls on.
fn line_and_column(bytes: &[u8], offset: usize) -> (usize, usize) {
    let before = &bytes[..offset];
    let line = before.iter().filter(|&&byte| byte == b'\n').count() + 1;
    let line_start = before
        .iter()
        .rposition(|&byte| byte == b'\n')
        .map_or(0, |at| at + 1);
    (line, offset - line_start + 1)
}

/// One line of IR text, by one-based line number.
fn line_at(bytes: &[u8], line: usize) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .nth(line - 1)
        .unwrap_or("<past the end>")
        .to_string()
}

/// Assert two images are identical, naming where they parted.
///
/// A plain `assert_eq!` on two quarter-megabyte images dumps thousands of
/// numbers into the failure message and hides the one fact worth knowing: the
/// offset where they parted. The offset alone is still thin evidence, so the
/// message carries the section it falls in and the bytes around it -- a
/// timestamp, a count and a length are told apart by their neighbours.
fn assert_same_binary(left: &[u8], right: &[u8], what: &str) {
    assert_eq!(
        left.len(),
        right.len(),
        "{what}: image lengths differ ({} and {} bytes)",
        left.len(),
        right.len()
    );
    if let Some(offset) = left.iter().zip(right).position(|(a, b)| a != b) {
        panic!(
            "{what}: images differ at byte {offset} of {} ({} and {}){}\n  left:  {}\n  right: {}",
            left.len(),
            left[offset],
            right[offset],
            image_site(left, offset),
            byte_window(left, offset),
            byte_window(right, offset),
        );
    }
}

/// The PE section an offset falls in, as a line to append to a failure.
///
/// "byte 235332" alone does not say whether the compiler's code or data
/// parted; ".rdata +50500" does. A non-PE image gets nothing: this is
/// diagnosis for the platform whose linker stamps the image.
fn image_site(bytes: &[u8], offset: usize) -> String {
    let Some(pe) = pe_offset(bytes) else {
        return String::new();
    };
    if pe + 24 > bytes.len() {
        return String::new();
    }
    let sections = usize::from(u16::from_le_bytes([bytes[pe + 6], bytes[pe + 7]]));
    let optional = usize::from(u16::from_le_bytes([bytes[pe + 20], bytes[pe + 21]]));
    for index in 0..sections {
        let entry = pe + 24 + optional + 40 * index;
        if entry + 40 > bytes.len() {
            break;
        }
        let size = u32::from_le_bytes([
            bytes[entry + 16],
            bytes[entry + 17],
            bytes[entry + 18],
            bytes[entry + 19],
        ]) as usize;
        let raw = u32::from_le_bytes([
            bytes[entry + 20],
            bytes[entry + 21],
            bytes[entry + 22],
            bytes[entry + 23],
        ]) as usize;
        if offset >= raw && offset < raw + size {
            let name_len = bytes[entry..entry + 8]
                .iter()
                .position(|&byte| byte == 0)
                .unwrap_or(8);
            let name = String::from_utf8_lossy(&bytes[entry..entry + name_len]);
            return format!(
                "\n  site: {name} +{} (section raw {raw}..{})",
                offset - raw,
                raw + size
            );
        }
    }
    String::new()
}

/// The differing byte in context: 32 bytes either side, hex with the byte
/// marked, and the same window as text. Enough to recognise a small field --
/// a timestamp, a count, a length -- without dumping the image.
fn byte_window(bytes: &[u8], offset: usize) -> String {
    let first = offset.saturating_sub(32);
    let last = (offset + 33).min(bytes.len());
    let mut hex = String::new();
    let mut text = String::new();
    for (index, &byte) in bytes[first..last].iter().enumerate() {
        if first + index == offset {
            hex.push_str(&format!("[{byte:02x}]"));
        } else {
            hex.push_str(&format!("{byte:02x} "));
        }
        text.push(if (0x20..0x7f).contains(&byte) {
            char::from(byte)
        } else {
            '.'
        });
    }
    format!("bytes {first}..{last}: {hex}\n  ascii: {text}")
}

/// Copy a directory tree recursively, without following into a symlink.
fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("the destination must be creatable");
    for entry in fs::read_dir(from).expect("the stdlib source must be readable") {
        let entry = entry.expect("a directory entry must be readable");
        let source = entry.path();
        let destination = to.join(entry.file_name());
        if source.is_dir() {
            copy_tree(&source, &destination);
        } else {
            fs::copy(&source, &destination).expect("a stdlib file must be copyable");
        }
    }
}
