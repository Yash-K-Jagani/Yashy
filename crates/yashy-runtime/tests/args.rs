//! The runtime's program-argument and working-directory entry points.
//!
//! `yashy_args_init` is the one piece of state the generated entry point hands
//! the runtime. The tests build a synthetic `argv` the same way the C runtime
//! receives one, fill a slice, and read the owned strings back through the
//! header layout the IR also uses. The leak assertion for the same path lives
//! in the driver's `leaks` suite, which runs a real program with
//! `YASHY_LEAK_REPORT=1`.

use std::ffi::{c_char, CStr, CString};

use yashy_runtime::{
    yashy_args_init, yashy_args_slice, yashy_getcwd, yashy_slice_free_strings, yashy_str_release,
    YashySlice, YashyStr,
};

/// The text of an owned string the runtime built.
unsafe fn text(string: *mut YashyStr) -> String {
    CStr::from_ptr((*string).data).to_str().unwrap().to_string()
}

#[test]
fn args_excludes_the_program_name_and_owns_every_element() {
    let program = CString::new("prog").unwrap();
    let one = CString::new("one").unwrap();
    let two = CString::new("two").unwrap();
    let mut argv: Vec<*mut c_char> = vec![
        program.as_ptr() as *mut c_char,
        one.as_ptr() as *mut c_char,
        two.as_ptr() as *mut c_char,
    ];

    unsafe {
        yashy_args_init(3, argv.as_mut_ptr());

        let mut slice = YashySlice::default();
        yashy_args_slice(&mut slice);

        assert_eq!(slice.len, 2, "the executable path is not a user argument");
        let elements = std::slice::from_raw_parts(slice.data as *const *mut YashyStr, 2);
        assert_eq!(text(elements[0]), "one");
        assert_eq!(text(elements[1]), "two");
        assert_eq!((*elements[0]).refcount, 1, "the slice owns its strings");

        yashy_slice_free_strings(&mut slice);
        assert!(slice.data.is_null(), "freeing zeroes the header");
        assert_eq!(slice.len, 0);
    }
}

#[test]
fn getcwd_returns_a_non_empty_owned_path() {
    unsafe {
        let cwd = yashy_getcwd();
        assert!(!cwd.is_null());
        assert!(!text(cwd).is_empty());
        assert!((*cwd).refcount > 0, "the caller owns the result");
        yashy_str_release(cwd);
    }
}
