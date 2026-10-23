//! Exercises the runtime's heap and arena primitives through the same
//! declarations the generated IR uses, so a signature that drifts from the
//! header fails here rather than in a linked Yashy program.
//!
//! The plan called for a compiled C `main` shelled out to from a test. These
//! are ordinary Rust tests instead: they reach the same C symbols through
//! `yashy-runtime`'s `extern` block (which the static library from `build.rs`
//! backs, exactly as `print.rs` does), so they prove the library is linked
//! while needing no extra compiler invocation and no child process.

use std::ffi::c_void;

/// A write-read round trip over `len` bytes starting at `ptr`, using distinct
/// byte patterns so a stale value cannot pass by accident.
unsafe fn touch_pattern(ptr: *mut c_void, len: usize, seed: u8) -> bool {
    let bytes = std::slice::from_raw_parts_mut(ptr as *mut u8, len);
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = seed.wrapping_add(index as u8);
    }
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != seed.wrapping_add(index as u8) {
            return false;
        }
    }
    true
}

fn is_aligned(ptr: *mut c_void, align: usize) -> bool {
    (ptr as usize) % align == 0
}

#[test]
fn a_megabyte_allocation_lets_both_ends_be_written() {
    unsafe {
        let pointer = yashy_runtime::yashy_alloc(1024 * 1024, 8);
        assert!(!pointer.is_null(), "a 1 MiB allocation must succeed");

        let bytes = pointer as *mut u8;
        *bytes = 0xAB;
        *bytes.add(1024 * 1024 - 1) = 0xCD;
        assert_eq!(*bytes, 0xAB, "the first byte must survive");
        assert_eq!(*bytes.add(1024 * 1024 - 1), 0xCD, "the last byte must survive");

        yashy_runtime::yashy_free(pointer);
    }
}

#[test]
fn every_allocation_respects_its_alignment() {
    unsafe {
        for &align in &[1, 2, 4, 8, 16, 32, 64, 256] {
            let pointer = yashy_runtime::yashy_alloc(24, align);
            assert!(!pointer.is_null(), "an allocation for align {align} must succeed");
            assert!(
                is_aligned(pointer, align as usize),
                "yashy_alloc(24, {align}) returned {:#x}, which is not {align}-aligned",
                pointer as usize
            );
            yashy_runtime::yashy_free(pointer);
        }
    }
}

#[test]
fn a_non_power_of_two_alignment_is_not_trusted_into_a_misaligned_block() {
    unsafe {
        // 24 is not a power of two, so the allocator must fall back rather than
        // honour it literally and hand back something no load can use.
        let pointer = yashy_runtime::yashy_alloc(64, 24);
        assert!(!pointer.is_null());
        assert!(
            is_aligned(pointer, 8),
            "a fallback allocation must still be pointer-aligned, got {:#x}",
            pointer as usize
        );
        yashy_runtime::yashy_free(pointer);
    }
}

#[test]
fn a_zero_sized_allocation_is_still_a_unique_usable_block() {
    unsafe {
        let first = yashy_runtime::yashy_alloc(0, 8);
        let second = yashy_runtime::yashy_alloc(0, 8);
        assert!(!first.is_null() && !second.is_null());
        assert_ne!(first, second, "two live allocations must not share an address");
        yashy_runtime::yashy_free(first);
        yashy_runtime::yashy_free(second);
    }
}

#[test]
fn freeing_a_null_pointer_is_a_no_op() {
    unsafe {
        yashy_runtime::yashy_free(std::ptr::null_mut());
    }
}

#[test]
fn many_allocate_free_cycles_do_not_exhaust_the_heap() {
    unsafe {
        // Not a leak proof -- Task 8 runs a real checker -- but a broken free
        // path would run the process out of memory long before the loop ends.
        for _ in 0..20_000 {
            let pointer = yashy_runtime::yashy_alloc(4096, 16);
            assert!(!pointer.is_null());
            *(pointer as *mut u8) = 1;
            yashy_runtime::yashy_free(pointer);
        }
    }
}

#[test]
fn an_arena_hands_out_writable_independent_blocks() {
    unsafe {
        let arena = yashy_runtime::yashy_arena_new();
        assert!(!arena.is_null(), "creating an arena must succeed");

        const COUNT: usize = 1000;
        let mut pointers: Vec<(*mut c_void, usize)> = Vec::with_capacity(COUNT);
        for index in 0..COUNT {
            // A mix of sizes, so a bump pointer that forgets to advance is
            // caught by two allocations landing on the same address.
            let size = 1 + (index % 97);
            let pointer = yashy_runtime::yashy_arena_alloc(arena, size as i64, 8);
            assert!(!pointer.is_null(), "arena allocation {index} must succeed");
            assert!(
                touch_pattern(pointer, size, index as u8),
                "arena allocation {index} must be writable and independent"
            );
            pointers.push((pointer, size));
        }

        // Nothing else was written, so every block must still hold its pattern.
        for (index, &(pointer, size)) in pointers.iter().enumerate() {
            let bytes = std::slice::from_raw_parts(pointer as *const u8, size);
            for (offset, byte) in bytes.iter().enumerate() {
                assert_eq!(
                    *byte,
                    (index as u8).wrapping_add(offset as u8),
                    "block {index} was clobbered by a later allocation"
                );
            }
        }

        yashy_runtime::yashy_arena_free_all(arena);
    }
}

#[test]
fn an_arena_allocates_more_than_a_default_block() {
    unsafe {
        let arena = yashy_runtime::yashy_arena_new();
        assert!(!arena.is_null());
        // Larger than YASHY_ARENA_BLOCK_SIZE (64 KiB): a fresh block has to be
        // sized to the request rather than to the default.
        let huge: usize = 512 * 1024;
        let pointer = yashy_runtime::yashy_arena_alloc(arena, huge as i64, 16);
        assert!(!pointer.is_null(), "an over-sized arena allocation must succeed");
        assert!(is_aligned(pointer, 16));
        assert!(touch_pattern(pointer, huge, 0x5A));
        yashy_runtime::yashy_arena_free_all(arena);
    }
}

#[test]
fn freeing_an_arena_tolerates_null_and_leaves_it_reusable() {
    unsafe {
        yashy_runtime::yashy_arena_free_all(std::ptr::null_mut());

        let arena = yashy_runtime::yashy_arena_new();
        let _ = yashy_runtime::yashy_arena_alloc(arena, 128, 8);
        yashy_runtime::yashy_arena_free_all(arena);

        // After free_all the header is empty again, so the arena can be reused
        // without a fresh yashy_arena_new.
        let pointer = yashy_runtime::yashy_arena_alloc(arena, 128, 8);
        assert!(!pointer.is_null(), "a freed arena must still allocate");
        yashy_runtime::yashy_arena_free_all(arena);
    }
}
