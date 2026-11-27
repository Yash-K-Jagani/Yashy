//! The capability handle: a grant installed at entry is what the runtime uses
//! when an operation executes, not just what the front end checked.
//!
//! The handle is process-global, so all states are exercised in one test in a
//! fixed order rather than racing independent tests against the same table.

use std::ffi::CString;

use yashy_runtime::{yashy_capability_has, yashy_capability_init};

fn has(name: &str) -> bool {
    let c_name = CString::new(name).unwrap();
    unsafe { yashy_capability_has(c_name.as_ptr()) != 0 }
}

#[test]
fn grants_are_opt_in_and_exact() {
    // Before init the handle is permissive: a program built without a manifest
    // keeps the behaviour it always had.
    assert!(has("fs.read"));
    assert!(has("net"));

    let grants = CString::new("fs.read,net").unwrap();
    unsafe { yashy_capability_init(grants.as_ptr()) };

    assert!(has("fs.read"), "the first grant is parsed");
    assert!(has("net"), "the second grant is parsed");
    assert!(!has("fs.write"), "an ungranted capability is refused");
    assert!(!has("env"), "an ungranted capability is refused");
    assert!(
        !has("fs.rea"),
        "a prefix of a grant is not the grant itself"
    );
    assert!(
        !has("fs.read,net"),
        "the whole csv is not a single capability name"
    );

    // Re-initialising replaces the set rather than unioning it.
    let grants = CString::new("env").unwrap();
    unsafe { yashy_capability_init(grants.as_ptr()) };
    assert!(has("env"));
    assert!(!has("fs.read"));
}
