//! The map half of the runtime: insertion, lookup, removal, growth, and the
//! reference counting that makes a `string` key safe.
//!
//! These are the same linked C the generated IR calls, through `otter-runtime`'s
//! `extern` block. The table is opaque, so every assertion goes through the API:
//! there is no layout to pin here, only behaviour.

use std::ffi::{c_void, CString};
use std::ptr::null_mut;

use otter_runtime::{
    otter_map_contains_key, otter_map_free, otter_map_get, otter_map_insert, otter_map_keys,
    otter_map_len, otter_map_new, otter_map_remove, otter_map_values, otter_slice_free,
    otter_slice_free_strings, otter_slice_get, otter_str_from_i64, otter_str_len, otter_str_release,
    OtterMap, OtterSlice, OtterStr,
};

/// A `CString` and the static `OtterStr` that points at its bytes.
///
/// `refcount -1` is what a literal is, so a static key never has to be released.
struct Static {
    value: OtterStr,
    _bytes: CString,
}

impl Static {
    fn new(text: &str) -> Self {
        let bytes = CString::new(text).expect("test strings must not contain a NUL byte");
        let len = bytes.as_bytes().len() as i64;
        let value = OtterStr {
            refcount: -1,
            len,
            cap: len,
            data: bytes.as_ptr() as *mut i8,
        };
        Self {
            value,
            _bytes: bytes,
        }
    }

    fn handle(&self) -> *const OtterStr {
        &self.value
    }
}

/// An empty table with the given key kind (0 = i64, 1 = bool, 2 = string) and
/// strides. The tables below all store `i64` values, so `value_size` is 8.
unsafe fn table(kind: i64, key_size: i64) -> *mut OtterMap {
    let map = otter_map_new(kind, key_size, 8);
    assert!(!map.is_null(), "the runtime must create the table");
    map
}

unsafe fn put(map: *mut OtterMap, key: i64, value: i64) {
    otter_map_insert(
        map,
        &key as *const i64 as *const c_void,
        &value as *const i64 as *const c_void,
    );
}

unsafe fn get(map: *const OtterMap, key: i64) -> Option<i64> {
    let mut out: i64 = i64::MIN;
    let found = otter_map_get(
        map,
        &key as *const i64 as *const c_void,
        &mut out as *mut i64 as *mut c_void,
    );
    (found != 0).then_some(out)
}

unsafe fn remove(map: *mut OtterMap, key: i64) -> Option<i64> {
    let mut out: i64 = i64::MIN;
    let found = otter_map_remove(
        map,
        &key as *const i64 as *const c_void,
        &mut out as *mut i64 as *mut c_void,
    );
    (found != 0).then_some(out)
}

#[test]
fn a_new_table_is_empty() {
    unsafe {
        let map = table(0, 8);
        assert_eq!(otter_map_len(map), 0);
        assert_eq!(get(map, 1), None);
        otter_map_free(map);
    }
}

#[test]
fn an_inserted_pair_comes_back() {
    unsafe {
        let map = table(0, 8);
        put(map, 7, 70);
        assert_eq!(otter_map_len(map), 1);
        assert_eq!(get(map, 7), Some(70));
        otter_map_free(map);
    }
}

#[test]
fn a_missing_key_is_a_zero_flag_and_leaves_the_out_slot_alone() {
    // The generated code zeroes the slot before every lookup, so a 0 answer is
    // never read. The runtime still must not write to it: this is what lets the
    // caller's zeroed payload stay defined.
    unsafe {
        let map = table(0, 8);
        put(map, 1, 10);
        let mut out: i64 = 99;
        let found = otter_map_get(
            map,
            &2_i64 as *const i64 as *const c_void,
            &mut out as *mut i64 as *mut c_void,
        );
        assert_eq!(found, 0);
        assert_eq!(out, 99);
        otter_map_free(map);
    }
}

#[test]
fn inserting_the_same_key_replaces_the_value_and_keeps_one_entry() {
    unsafe {
        let map = table(0, 8);
        put(map, 3, 30);
        put(map, 3, 33);
        assert_eq!(otter_map_len(map), 1, "an overwrite is not a second entry");
        assert_eq!(get(map, 3), Some(33));
        otter_map_free(map);
    }
}

#[test]
fn remove_returns_the_value_and_decrements_the_length() {
    unsafe {
        let map = table(0, 8);
        put(map, 4, 40);
        assert_eq!(remove(map, 4), Some(40));
        assert_eq!(otter_map_len(map), 0);
        assert_eq!(get(map, 4), None);
        assert_eq!(remove(map, 4), None, "a second removal finds nothing");
        otter_map_free(map);
    }
}

#[test]
fn bool_keys_are_distinct() {
    unsafe {
        let map = table(1, 1);
        let (yes, no) = (1_i8, 0_i8);
        otter_map_insert(
            map,
            &yes as *const i8 as *const c_void,
            &10_i64 as *const i64 as *const c_void,
        );
        otter_map_insert(
            map,
            &no as *const i8 as *const c_void,
            &20_i64 as *const i64 as *const c_void,
        );
        let mut out: i64 = 0;
        assert_eq!(
            otter_map_get(
                map,
                &yes as *const i8 as *const c_void,
                &mut out as *mut i64 as *mut c_void
            ),
            1
        );
        assert_eq!(out, 10);
        assert_eq!(
            otter_map_get(
                map,
                &no as *const i8 as *const c_void,
                &mut out as *mut i64 as *mut c_void
            ),
            1
        );
        assert_eq!(out, 20);
        assert_eq!(otter_map_len(map), 2);
        otter_map_free(map);
    }
}

#[test]
fn string_keys_compare_by_content_and_the_table_retains_them() {
    unsafe {
        let map = table(2, 8);
        // The caller owns one reference; the table takes a second.
        let key = otter_str_from_i64(7);
        assert_eq!((*key).refcount, 1);
        let handle = key as *const OtterStr;
        otter_map_insert(
            map,
            &handle as *const *const OtterStr as *const c_void,
            &70_i64 as *const i64 as *const c_void,
        );
        assert_eq!((*key).refcount, 2, "the table keeps its own reference");

        // The caller lets go. The bytes stay alive because the table holds them.
        otter_str_release(key);

        // A different string with the same bytes is the same key.
        let probe = Static::new("7");
        let probe_handle = probe.handle();
        let mut out: i64 = 0;
        let found = otter_map_get(
            map,
            &probe_handle as *const *const OtterStr as *const c_void,
            &mut out as *mut i64 as *mut c_void,
        );
        assert_eq!(found, 1);
        assert_eq!(out, 70);

        assert_eq!(
            otter_map_remove(
                map,
                &probe_handle as *const *const OtterStr as *const c_void,
                &mut out as *mut i64 as *mut c_void
            ),
            1
        );
        otter_map_free(map);
    }
}

#[test]
fn overwriting_a_string_key_keeps_exactly_one_reference() {
    unsafe {
        let map = table(2, 8);
        let key = otter_str_from_i64(1);
        let handle = key as *const OtterStr;
        let key_ptr = &handle as *const *const OtterStr as *const c_void;
        otter_map_insert(map, key_ptr, &1_i64 as *const i64 as *const c_void);
        assert_eq!((*key).refcount, 2);

        // An equal string overwrites: no second retain, the stored key stays.
        let equal = Static::new("1");
        let equal_handle = equal.handle();
        let equal_ptr = &equal_handle as *const *const OtterStr as *const c_void;
        otter_map_insert(map, equal_ptr, &2_i64 as *const i64 as *const c_void);
        assert_eq!((*key).refcount, 2, "an overwrite must not retain twice");

        let mut out: i64 = 0;
        assert_eq!(
            otter_map_get(map, equal_ptr, &mut out as *mut i64 as *mut c_void),
            1
        );
        assert_eq!(out, 2);

        otter_map_free(map);
        assert_eq!((*key).refcount, 1, "free drops only the table's reference");
        otter_str_release(key);
    }
}

#[test]
fn growth_keeps_every_entry_across_rehashes() {
    unsafe {
        let map = table(0, 8);
        // Well past the initial capacity, so the table rehashes several times.
        for key in 0..1000 {
            put(map, key, key * 3);
        }
        assert_eq!(otter_map_len(map), 1000);
        for key in [0, 1, 7, 255, 512, 999] {
            assert_eq!(get(map, key), Some(key * 3), "key {key} survived growth");
        }

        // Removing half leaves the other half reachable: tombstones must not
        // break the probe chains that cross them.
        for key in (0..1000).step_by(2) {
            assert_eq!(remove(map, key), Some(key * 3));
        }
        assert_eq!(otter_map_len(map), 500);
        for key in (0..1000).step_by(2) {
            assert_eq!(get(map, key), None);
        }
        for key in (1..1000).step_by(2) {
            assert_eq!(get(map, key), Some(key * 3));
        }
        otter_map_free(map);
    }
}

#[test]
fn null_and_invalid_tables_are_rejected_without_crashing() {
    unsafe {
        assert_eq!(otter_map_len(std::ptr::null()), 0);
        otter_map_free(null_mut());

        let mut out: i64 = 5;
        assert_eq!(
            otter_map_get(
                std::ptr::null(),
                &1_i64 as *const i64 as *const c_void,
                &mut out as *mut i64 as *mut c_void
            ),
            0
        );
        assert_eq!(out, 5);
        assert_eq!(
            otter_map_remove(
                null_mut(),
                &1_i64 as *const i64 as *const c_void,
                &mut out as *mut i64 as *mut c_void
            ),
            0
        );

        // The compiler only ever passes 0/1/2 and positive strides; a bad call
        // is a null table rather than a table that faults on insert.
        assert!(otter_map_new(3, 8, 8).is_null());
        assert!(otter_map_new(0, 0, 8).is_null());
        assert!(otter_map_new(0, 8, 0).is_null());
    }
}

#[test]
fn contains_key_answers_without_touching_the_value() {
    unsafe {
        let map = table(0, 8);
        put(map, 5, 50);
        assert_eq!(
            otter_map_contains_key(map, &5_i64 as *const i64 as *const c_void),
            1
        );
        assert_eq!(
            otter_map_contains_key(map, &6_i64 as *const i64 as *const c_void),
            0,
            "a key the table never held is not in it"
        );

        // A removed key is gone, even though its slot is a tombstone.
        remove(map, 5);
        assert_eq!(
            otter_map_contains_key(map, &5_i64 as *const i64 as *const c_void),
            0
        );
        otter_map_free(map);
    }
}

#[test]
fn contains_key_answers_for_a_string_key_by_content() {
    unsafe {
        let map = table(2, 8);
        let key = otter_str_from_i64(7);
        let handle = key as *const OtterStr;
        otter_map_insert(
            map,
            &handle as *const *const OtterStr as *const c_void,
            &70_i64 as *const i64 as *const c_void,
        );

        let equal = Static::new("7");
        let equal_handle = equal.handle();
        assert_eq!(
            otter_map_contains_key(
                map,
                &equal_handle as *const *const OtterStr as *const c_void
            ),
            1,
            "a string key is found by its content"
        );

        let other = Static::new("8");
        let other_handle = other.handle();
        assert_eq!(
            otter_map_contains_key(
                map,
                &other_handle as *const *const OtterStr as *const c_void
            ),
            0
        );
        otter_map_free(map);
        otter_str_release(key);
    }
}

/// The elements of a fresh slice, read back as `i64`.
unsafe fn i64_elements(slice: &mut OtterSlice) -> Vec<i64> {
    let mut values = Vec::with_capacity(slice.len as usize);
    for index in 0..slice.len {
        let mut value: i64 = 0;
        let found = otter_slice_get(
            slice,
            index,
            &mut value as *mut i64 as *mut c_void,
            8,
        );
        assert_eq!(found, 1, "element {index} must be readable");
        values.push(value);
    }
    values
}

#[test]
fn keys_and_values_come_back_as_arrays() {
    unsafe {
        let map = table(0, 8);
        put(map, 1, 10);
        put(map, 2, 20);
        put(map, 3, 30);

        let mut keys = OtterSlice {
            data: null_mut(),
            len: 0,
            cap: 0,
        };
        otter_map_keys(&mut keys, map);
        assert_eq!(keys.len, 3);
        let mut sorted = i64_elements(&mut keys);
        sorted.sort_unstable();
        assert_eq!(sorted, vec![1, 2, 3], "every key is in the array");
        otter_slice_free(&mut keys);

        let mut values = OtterSlice {
            data: null_mut(),
            len: 0,
            cap: 0,
        };
        otter_map_values(&mut values, map);
        assert_eq!(values.len, 3);
        let mut sorted = i64_elements(&mut values);
        sorted.sort_unstable();
        assert_eq!(sorted, vec![10, 20, 30], "every value is in the array");
        otter_slice_free(&mut values);

        otter_map_free(map);
    }
}

#[test]
fn the_key_and_value_arrays_follow_the_table() {
    unsafe {
        let map = table(0, 8);
        put(map, 1, 10);

        let mut keys = OtterSlice {
            data: null_mut(),
            len: 0,
            cap: 0,
        };
        otter_map_keys(&mut keys, map);
        assert_eq!(keys.len, 1, "a fresh table's array is empty");
        otter_slice_free(&mut keys);

        put(map, 2, 20);
        let mut keys = OtterSlice {
            data: null_mut(),
            len: 0,
            cap: 0,
        };
        otter_map_keys(&mut keys, map);
        assert_eq!(keys.len, 2, "the array is a copy of the table as it is now");
        otter_slice_free(&mut keys);

        remove(map, 1);
        let mut values = OtterSlice {
            data: null_mut(),
            len: 0,
            cap: 0,
        };
        otter_map_values(&mut values, map);
        assert_eq!(values.len, 1, "a removed entry is not in the array");
        otter_slice_free(&mut values);

        otter_map_free(map);
    }
}

#[test]
fn the_key_array_owns_its_string_keys() {
    unsafe {
        let map = table(2, 8);
        let key = otter_str_from_i64(7);
        let handle = key as *const OtterStr;
        otter_map_insert(
            map,
            &handle as *const *const OtterStr as *const c_void,
            &70_i64 as *const i64 as *const c_void,
        );
        assert_eq!((*key).refcount, 2, "the table keeps a reference");

        let mut keys = OtterSlice {
            data: null_mut(),
            len: 0,
            cap: 0,
        };
        otter_map_keys(&mut keys, map);
        assert_eq!((*key).refcount, 3, "the array keeps a reference of its own");
        assert_eq!(keys.len, 1);

        // The table goes away. The array's own reference keeps the bytes alive.
        otter_map_free(map);
        assert_eq!((*key).refcount, 2, "the table's reference is gone");
        let element = *(keys.data as *const *const OtterStr);
        assert_eq!(otter_str_len(element), 1, "the key is still readable");

        // The array goes away too: exactly one reference is left, the caller's.
        otter_slice_free_strings(&mut keys);
        assert_eq!((*key).refcount, 1, "the array's reference is gone");
        otter_str_release(key);
    }
}
