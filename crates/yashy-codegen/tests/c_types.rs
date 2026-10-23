//! The LLVM side of the C type spellings.
//!
//! Every width here is an ABI promise: a declaration that says `i32` and a
//! lowering that emits `i64` links cleanly and corrupts the call. The mapping
//! is therefore pinned one width at a time.

use yashy_common::Span;
use yashy_sema::{Type, TypeKind};

use yashy_codegen::llvm_type_string;

fn c_ptr() -> Type {
    Type {
        kind: TypeKind::CPtr(Box::new(Type {
            kind: TypeKind::CVoid,
            span: Span::SYNTHETIC,
        })),
        span: Span::SYNTHETIC,
    }
}

#[test]
fn llvm_widths_match_the_declaration() {
    assert_eq!(llvm_type_string(&TypeKind::CInt(8)), "i8");
    assert_eq!(llvm_type_string(&TypeKind::CInt(16)), "i16");
    assert_eq!(llvm_type_string(&TypeKind::CInt(32)), "i32");
    assert_eq!(llvm_type_string(&TypeKind::CInt(64)), "i64");
    assert_eq!(llvm_type_string(&TypeKind::CUInt(8)), "i8");
    assert_eq!(llvm_type_string(&TypeKind::CUInt(16)), "i16");
    assert_eq!(llvm_type_string(&TypeKind::CUInt(32)), "i32");
    assert_eq!(llvm_type_string(&TypeKind::CUInt(64)), "i64");
    assert_eq!(llvm_type_string(&TypeKind::CFloat(32)), "float");
    assert_eq!(llvm_type_string(&TypeKind::CFloat(64)), "double");
    assert_eq!(llvm_type_string(&TypeKind::CSize), "i64");
    assert_eq!(llvm_type_string(&TypeKind::CVoid), "void");
    assert_eq!(llvm_type_string(&TypeKind::CPtr(Box::new(c_ptr()))), "i8*");
}

#[test]
fn the_c_scalars_keep_their_yashy_counterparts() {
    // The shared widths are aliases, not new representations: a boundary that
    // says `i64` and a body that computes `i64` must lower the same way.
    assert_eq!(llvm_type_string(&TypeKind::I64), llvm_type_string(&TypeKind::CInt(64)));
    assert_eq!(llvm_type_string(&TypeKind::F64), llvm_type_string(&TypeKind::CFloat(64)));
}
