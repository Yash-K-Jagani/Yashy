//! The field-order helper: one permutation, used by the layout walk, the
//! emitted aggregate and the report.
//!
//! These tests use only primitive field types, so `NoDecls` is enough to pin
//! the rule without involving a program.

use yashy_hir::layout::{field_order, layout, layout_fields, NoDecls};
use yashy_hir::ty::TypeKind;
use yashy_parser::ast::ReprKind;

fn types() -> Vec<TypeKind> {
    vec![TypeKind::CInt(8), TypeKind::I64, TypeKind::CInt(8)]
}

#[test]
fn a_plain_struct_emits_largest_fields_first() {
    let decls = NoDecls;
    let fields = types();
    assert_eq!(field_order(&fields, None, &decls), vec![1, 0, 2]);
}

#[test]
fn a_declared_representation_keeps_declaration_order() {
    let decls = NoDecls;
    let fields = types();
    for repr in [
        ReprKind::C,
        ReprKind::Packed,
        ReprKind::Align(16),
        ReprKind::Transparent,
        ReprKind::Int(8),
    ] {
        assert_eq!(
            field_order(&fields, Some(repr), &decls),
            vec![0, 1, 2],
            "{repr:?} is a promise about bytes"
        );
    }
}

#[test]
fn reordering_removes_padding_without_changing_declaration_offsets() {
    let decls = NoDecls;
    let fields = types();
    let order = field_order(&fields, None, &decls);
    let natural = layout_fields(&fields, &decls, &[0, 1, 2]).expect("a layout exists");
    let chosen = layout_fields(&fields, &decls, &order).expect("a layout exists");
    assert_eq!((natural.size, chosen.size), (24, 16));
    // Offsets stay keyed by declaration index, so `offsetof` and a field read
    // do not change meaning when the aggregate is reordered.
    assert_eq!(natural.offsets, vec![0, 8, 16]);
    assert_eq!(chosen.offsets, vec![8, 0, 9]);
    assert_eq!(chosen.align, 8);
}

#[test]
fn equal_sized_fields_keep_their_declaration_order() {
    let decls = NoDecls;
    let fields = vec![TypeKind::I64, TypeKind::I64, TypeKind::CInt(8)];
    assert_eq!(field_order(&fields, None, &decls), vec![0, 1, 2]);
}

#[test]
fn the_layout_walk_uses_the_same_order() {
    // A standalone scalar's walk is unchanged: the helper is only a struct's
    // concern, and a declared type with NoDecls still has no guessed layout.
    assert_eq!(layout(&TypeKind::I64, &NoDecls).unwrap().size, 8);
}
