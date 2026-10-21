//! The runtime's closure environment: the block, its reference count, and what
//! it releases when the last owner lets go.
//!
//! The environment is the one piece of the runtime that the *generated code*
//! builds: the runtime owns the count and the release of what the block
//! captured, and the generated code writes the table that says which captured
//! strings the environment owns. These tests drive the same entry points the
//! IR calls, so a layout that drifts from the backend's fails here.

use std::ffi::{c_longlong, c_void};

use otter_runtime::{
    otter_env_new, otter_env_release, otter_env_retain, otter_str_from_i64, OTTER_ENV_HEADER_SIZE,
    OTTER_ENV_SLOT_SIZE,
};

/// The environment's header, as the runtime lays it out.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct EnvHeader {
    refcount: c_longlong,
    slot_count: c_longlong,
}

/// The kind of a table entry that releases a captured `string`.
const SLOT_STRING: i64 = 1;

#[test]
fn a_new_environment_has_one_reference_and_an_empty_table() {
    unsafe {
        let env = otter_env_new(0, 8, 16);
        assert!(!env.is_null(), "the environment must be allocated");
        let header = &*(env as *const EnvHeader);
        assert_eq!(header.refcount, 1, "a new environment has one reference");
        assert_eq!(header.slot_count, 0, "and no captured values to release");
        otter_env_release(env);
    }
}

#[test]
fn the_captured_values_start_after_the_header_and_the_table() {
    unsafe {
        let env = otter_env_new(2, 16, 16);
        assert!(!env.is_null());
        let header = &*(env as *const EnvHeader);
        assert_eq!(header.slot_count, 2);
        // The table is two entries of sixteen bytes after the sixteen-byte
        // header, so the payload starts at forty-eight.
        assert_eq!(OTTER_ENV_HEADER_SIZE, 16);
        assert_eq!(OTTER_ENV_SLOT_SIZE, 16);
        assert_eq!(
            OTTER_ENV_HEADER_SIZE + 2 * OTTER_ENV_SLOT_SIZE,
            48,
            "the captured values start after the header and the table"
        );
        otter_env_release(env);
    }
}

#[test]
fn the_count_goes_up_and_down_and_the_block_lives_until_the_last_one() {
    unsafe {
        let env = otter_env_new(0, 8, 16);
        let kept = otter_env_retain(env);
        assert_eq!(kept, env, "retain returns the environment");
        let header = &*(env as *const EnvHeader);
        assert_eq!(header.refcount, 2, "the second owner counts");
        otter_env_release(env);
        let header = &*(env as *const EnvHeader);
        assert_eq!(header.refcount, 1, "one owner is left");
        otter_env_release(env);
    }
}

#[test]
fn a_null_environment_is_left_alone() {
    unsafe {
        assert!(otter_env_retain(std::ptr::null_mut()).is_null());
        otter_env_release(std::ptr::null_mut());
    }
}

#[test]
fn the_last_release_frees_the_string_the_environment_owned() {
    unsafe {
        // A string the runtime built: refcount 1, on the heap, so the
        // environment's release is what frees it.
        let string = otter_str_from_i64(7);
        assert!(!string.is_null());
        let env = otter_env_new(1, 8, 16);
        // The table entry says the environment owns this reference, and the
        // pointer is the string's own.
        let table = (env as *mut u8).add(OTTER_ENV_HEADER_SIZE) as *mut i64;
        table.write(SLOT_STRING);
        let pointer = (env as *mut u8)
            .add(OTTER_ENV_HEADER_SIZE + 8) as *mut *mut c_void;
        pointer.write(string as *mut c_void);
        // The runtime's release reads the string through the table.
        otter_env_release(env);
    }
}
