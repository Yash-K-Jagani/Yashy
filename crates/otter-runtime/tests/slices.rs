//! The slice half of the runtime: `otter_slice_push` and `otter_slice_free`.
//!
//! These call the same linked C the generated IR calls, through the
//! declarations in `otter-runtime`'s `extern` block. The header layout is the
//! contract, so the tests build an [`OtterSlice`] by hand and read the backing
//! array back as `i64`s.

use std::ffi::c_void;
use std::ptr::null_mut;

use otter_runtime::{
    otter_slice_free, otter_slice_free_strings, otter_slice_get, otter_slice_pop, otter_slice_push,
    otter_str_from_i64, otter_str_release, otter_str_retain, OtterStr, OtterSlice,
};

/// The `i64` elements currently in `slice`.
unsafe fn values(slice: &OtterSlice) -> Vec<i64> {
    if slice.data.is_null() || slice.len == 0 {
        return Vec::new();
    }
    std::slice::from_raw_parts(slice.data as *const i64, slice.len as usize).to_vec()
}

unsafe fn push(slice: &mut OtterSlice, value: i64) {
    otter_slice_push(slice, &value as *const i64 as *const c_void, 8);
}

#[test]
fn pushing_into_an_empty_slice_allocates_and_stores() {
    unsafe {
        let mut slice = OtterSlice::default();
        assert!(slice.data.is_null());

        push(&mut slice, 42);

        assert!(!slice.data.is_null(), "the first push must allocate");
        assert_eq!(slice.len, 1);
        assert_eq!(slice.cap, 4, "growth starts at four elements");
        assert_eq!(values(&slice), vec![42]);

        otter_slice_free(&mut slice);
    }
}

#[test]
fn pushing_past_capacity_doubles_and_preserves_the_elements() {
    unsafe {
        let mut slice = OtterSlice::default();
        for index in 0..100_i64 {
            push(&mut slice, index * 3);
            assert_eq!(slice.len, index + 1);
            assert!(slice.cap >= slice.len, "capacity must cover the length");
            assert_eq!(
                values(&slice),
                (0..=index).map(|value| value * 3).collect::<Vec<_>>(),
                "element {index} must not disturb earlier ones"
            );
        }
        // 100 elements fit in the fifth doubling (64 -> 128), so the witness
        // above is not the only thing proving growth: the capacity must have
        // doubled rather than grown one element at a time.
        assert_eq!(slice.cap, 128);
        otter_slice_free(&mut slice);
    }
}

#[test]
fn freeing_releases_and_zeroes_the_header_so_it_can_be_reused() {
    unsafe {
        let mut slice = OtterSlice::default();
        for value in [1_i64, 2, 3] {
            push(&mut slice, value);
        }

        otter_slice_free(&mut slice);

        assert!(slice.data.is_null(), "the data pointer is cleared");
        assert_eq!(slice.len, 0);
        assert_eq!(slice.cap, 0);

        // A freed header is an empty one: pushing again starts a new block.
        push(&mut slice, 7);
        assert_eq!(values(&slice), vec![7]);
        otter_slice_free(&mut slice);
    }
}

#[test]
fn freeing_a_null_slice_is_a_no_op() {
    unsafe {
        otter_slice_free(null_mut());
    }
}

#[test]
fn pushing_into_a_null_slice_is_ignored() {
    unsafe {
        let value = 9_i64;
        otter_slice_push(
            null_mut(),
            &value as *const i64 as *const c_void,
            8,
        );
    }
}

unsafe fn sliced(values: &[i64]) -> OtterSlice {
    let mut slice = OtterSlice::default();
    for value in values {
        otter_slice_push(&mut slice, value as *const i64 as *const c_void, 8);
    }
    slice
}

#[test]
fn get_copies_an_element_that_is_in_range() {
    unsafe {
        let mut slice = sliced(&[10, 20, 30]);
        let mut out = 0_i64;
        let found = otter_slice_get(
            &mut slice,
            1,
            &mut out as *mut i64 as *mut c_void,
            8,
        );
        assert_eq!(found, 1);
        assert_eq!(out, 20);
        otter_slice_free(&mut slice);
    }
}

#[test]
fn get_is_none_for_negative_out_of_range_and_empty() {
    unsafe {
        let mut slice = sliced(&[10]);
        for index in [-1_i64, 1, 100] {
            let mut out = 0_i64;
            let found = otter_slice_get(
                &mut slice,
                index,
                &mut out as *mut i64 as *mut c_void,
                8,
            );
            assert_eq!(found, 0, "index {index} is outside [0, len)");
            assert_eq!(out, 0, "a miss does not overwrite the out slot");
        }

        let mut empty = OtterSlice::default();
        let mut out = 0_i64;
        assert_eq!(
            otter_slice_get(&mut empty, 0, &mut out as *mut i64 as *mut c_void, 8),
            0
        );
        assert_eq!(
            otter_slice_get(null_mut(), 0, &mut out as *mut i64 as *mut c_void, 8),
            0
        );
        otter_slice_free(&mut slice);
    }
}

#[test]
fn pop_takes_the_last_element_and_shrinks_the_slice() {
    unsafe {
        let mut slice = sliced(&[10, 20, 30]);
        let mut out = 0_i64;
        assert_eq!(
            otter_slice_pop(&mut slice, &mut out as *mut i64 as *mut c_void, 8),
            1
        );
        assert_eq!(out, 30);
        assert_eq!(slice.len, 2);
        assert_eq!(
            otter_slice_pop(&mut slice, &mut out as *mut i64 as *mut c_void, 8),
            1
        );
        assert_eq!(out, 20);
        assert_eq!(
            otter_slice_pop(&mut slice, &mut out as *mut i64 as *mut c_void, 8),
            1
        );
        assert_eq!(out, 10);
        assert_eq!(slice.len, 0);
        assert_eq!(
            otter_slice_pop(&mut slice, &mut out as *mut i64 as *mut c_void, 8),
            0,
            "an empty slice has nothing to pop"
        );
        otter_slice_free(&mut slice);
    }
}

/// Push one `string` handle the way generated code does: the slice retains it.
unsafe fn push_string(slice: &mut OtterSlice, handle: *mut OtterStr) {
    let retained = otter_str_retain(handle);
    otter_slice_push(
        slice,
        &retained as *const *mut OtterStr as *const c_void,
        std::mem::size_of::<*mut OtterStr>() as i64,
    );
}

#[test]
fn freeing_a_string_slice_releases_every_element() {
    unsafe {
        let mut slice = OtterSlice::default();
        let first = otter_str_from_i64(1);
        let second = otter_str_from_i64(2);
        assert_eq!((*first).refcount, 1, "the caller owns one reference");
        push_string(&mut slice, first);
        push_string(&mut slice, second);
        assert_eq!((*first).refcount, 2, "the slice owns a second reference");

        otter_slice_free_strings(&mut slice);

        assert_eq!(
            (*first).refcount,
            1,
            "freeing the slice drops the slice's reference"
        );
        assert_eq!((*second).refcount, 1);
        assert!(slice.data.is_null(), "the header is zeroed");
        assert_eq!((slice.len, slice.cap), (0, 0));

        otter_str_release(first);
        otter_str_release(second);
    }
}

#[test]
fn freeing_a_string_slice_tolerates_null_and_empty() {
    unsafe {
        otter_slice_free_strings(null_mut());
        let mut slice = OtterSlice::default();
        otter_slice_free_strings(&mut slice);
        assert!(slice.data.is_null());
    }
}
