//! The runtime primitives behind the format engine: fixed-precision float
//! text and sign-aware, display-width padding.

use std::ffi::{c_char, CStr, CString};

use yashy_runtime::YashyStr;

struct Static {
    _bytes: CString,
    value: YashyStr,
}

impl Static {
    fn new(text: &str) -> Self {
        let bytes = CString::new(text).expect("no NUL in a test string");
        let len = bytes.as_bytes().len() as i64;
        let value = YashyStr {
            refcount: -1,
            len,
            cap: len,
            data: bytes.as_ptr() as *mut c_char,
        };
        Static {
            _bytes: bytes,
            value,
        }
    }

    fn ptr(&self) -> *const YashyStr {
        &self.value
    }
}

unsafe fn read(string: *const YashyStr) -> String {
    assert!(!string.is_null(), "the runtime returned a null string");
    CStr::from_ptr((*string).data)
        .to_str()
        .expect("the runtime returns UTF-8 for these inputs")
        .to_string()
}

unsafe fn release(string: *mut YashyStr) {
    yashy_runtime::yashy_str_release(string);
}

/// The default spelling of a float is the shortest decimal that reads back as
/// the same double: `print(0.1)` says `0.1`, not `0.10000000000000001`.
#[test]
fn default_float_text_is_the_shortest_round_trip() {
    unsafe {
        let cases: [(f64, &str); 7] = [
            (0.1, "0.1"),
            (0.1 + 0.2, "0.30000000000000004"),
            (1.0 / 3.0, "0.3333333333333333"),
            (2.5, "2.5"),
            (1200.0, "1200"),
            (1e20, "1e+20"),
            (1e-5, "0.00001"),
        ];
        for (value, expected) in cases {
            let text = yashy_runtime::yashy_str_from_f64(value);
            assert_eq!(read(text), expected, "{value:?}");
            release(text);
        }
        // Every candidate really does read back to the same double.
        for value in [0.1, 0.1 + 0.2, 1.0 / 3.0, 1e20, 1e-5, f64::MAX] {
            let text = yashy_runtime::yashy_str_from_f64(value);
            let printed = read(text);
            release(text);
            assert_eq!(
                printed.parse::<f64>().expect("prints a float"),
                value,
                "{printed} does not round-trip"
            );
        }
    }
}

#[test]
fn fixed_precision_is_locale_independent() {
    unsafe {
        let text = yashy_runtime::yashy_str_from_f64_prec(3.14159, 2);
        assert_eq!(read(text), "3.14");
        release(text);

        let rounded = yashy_runtime::yashy_str_from_f64_prec(3.145, 2);
        assert_eq!(read(rounded), "3.15");
        release(rounded);

        let whole = yashy_runtime::yashy_str_from_f64_prec(3.0, 0);
        assert_eq!(read(whole), "3");
        release(whole);
    }
}

#[test]
fn padding_counts_display_cells_and_keeps_the_sign_first() {
    unsafe {
        let text = Static::new("3.14");
        let spaces = Static::new(" ");
        let right = yashy_runtime::yashy_str_pad(text.ptr(), 8, 1, spaces.ptr());
        assert_eq!(read(right), "    3.14");
        release(right);

        let left = yashy_runtime::yashy_str_pad(text.ptr(), 8, 0, spaces.ptr());
        assert_eq!(read(left), "3.14    ");
        release(left);

        let center = yashy_runtime::yashy_str_pad(text.ptr(), 9, 2, spaces.ptr());
        assert_eq!(read(center), "  3.14   ");
        release(center);

        let negative = Static::new("-3.14");
        let zeros = Static::new("0");
        let numeric = yashy_runtime::yashy_str_pad(negative.ptr(), 8, 1, zeros.ptr());
        assert_eq!(read(numeric), "-0003.14");
        release(numeric);

        let cjk = Static::new("漢");
        let wide = yashy_runtime::yashy_str_pad(cjk.ptr(), 4, 0, spaces.ptr());
        assert_eq!(read(wide), "漢  ");
        release(wide);
    }
}
