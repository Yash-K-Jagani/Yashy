//! A type's size, alignment and field offsets.
//!
//! One walk, three callers: `sizeof`, `alignof` and `offsetof` in the checker,
//! and the backend's own size table. The checker may not depend on the backend
//! (the backend depends on it), so the walk lives here, in the crate both of
//! them already share.
//!
//! # What the walk knows
//!
//! A scalar is a table entry. A struct is its fields in declaration order,
//! each aligned to its own alignment, and the whole rounded up to the largest
//! one. An enum is its tag followed by every variant's payload, flattened in
//! declaration order -- the shape the backend emits as one literal struct. A
//! `repr(C)` enum's tag is a C `int`; every other enum's tag is one byte.
//!
//! A declared type is looked up through [`Decls`] because the walk cannot see
//! the program. [`NoDecls`] is the answer for a caller with no declaration
//! table: a struct then has no layout this module can report, and [`layout`]
//! returns `None` rather than inventing one.

use crate::ty::TypeKind;

/// A type's layout: how many bytes it occupies, what it aligns to, and where
/// its fields start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The number of bytes one value occupies, padding included.
    pub size: usize,
    /// The alignment the type's address must have.
    pub align: usize,
    /// The byte offset of each field, in declaration order. Empty for a type
    /// with no fields of its own: a scalar, a tuple, an array or an enum.
    pub offsets: Vec<usize>,
}

/// What the walk needs to know about a program's declared types.
///
/// A struct's layout depends on its fields' types, and a field may name another
/// declared struct, so the lookup has to recurse through the same table the
/// caller holds. Keeping it behind a trait is what lets the checker answer from
/// its own signature table and the backend answer from its module.
pub trait Decls {
    /// The declaration named `name`, reduced to what a layout needs.
    fn named(&self, name: &str) -> Option<NamedType>;
}

/// A declared type, reduced to the fields a layout walk needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NamedType {
    /// A `struct`'s fields, in declaration order.
    ///
    /// The representation is not part of this shape: Yashy has no bit-fields,
    /// unions or packed fields, so C's rules and Yashy's own are the same walk
    /// for every struct the language can declare.
    Struct { fields: Vec<TypeKind> },
    /// An `enum`'s payloads, one list per variant, in declaration order.
    ///
    /// `repr_c` is the one thing about a representation that changes a layout:
    /// a C `enum` is an `int`, so its tag is four bytes rather than one.
    Enum {
        payloads: Vec<Vec<TypeKind>>,
        repr_c: bool,
    },
}

/// A [`Decls`] that knows no declarations.
///
/// A caller that has no declaration table still wants the scalar answers, and
/// gets `None` for a declared type instead of a guess. Guessing here would make
/// `sizeof(Point)` a number that depends on which caller asked.
pub struct NoDecls;

impl Decls for NoDecls {
    fn named(&self, _name: &str) -> Option<NamedType> {
        None
    }
}

/// The layout of `ty`, or `None` when a declared type it mentions is not in
/// `decls`.
pub fn layout(ty: &TypeKind, decls: &dyn Decls) -> Option<Layout> {
    Some(match ty {
        TypeKind::Struct(name) => {
            let NamedType::Struct { fields } = decls.named(name)? else {
                return None;
            };
            fields_layout(&fields, decls)?
        }
        TypeKind::Enum(name) => {
            let NamedType::Enum { payloads, repr_c } = decls.named(name)? else {
                return None;
            };
            // The tag is a C `int` under `repr(C)` and one byte otherwise, and
            // every payload is appended in declaration order, which is exactly
            // the aggregate the backend defines for the type.
            let mut fields = vec![if repr_c {
                TypeKind::CInt(32)
            } else {
                TypeKind::CInt(8)
            }];
            for payload in &payloads {
                fields.extend(payload.iter().cloned());
            }
            fields_layout(&fields, decls)?
        }
        TypeKind::Option(inner) => {
            let payload = layout(&inner.kind, decls)?;
            let tag = layout(&TypeKind::CInt(8), decls)?;
            fields_layout_from(vec![tag, payload])
        }
        TypeKind::Result(ok, err) => {
            let tag = layout(&TypeKind::CInt(8), decls)?;
            let ok = layout(&ok.kind, decls)?;
            let err = layout(&err.kind, decls)?;
            fields_layout_from(vec![tag, ok, err])
        }
        TypeKind::Tuple(elements) => {
            let fields = elements
                .iter()
                .map(|element| layout(&element.kind, decls))
                .collect::<Option<Vec<_>>>()?;
            fields_layout_from(fields)
        }
        TypeKind::FixedArray(element, length) => {
            let element = layout(&element.kind, decls)?;
            // An array has no padding between elements beyond the element's
            // own stride, and its alignment is the element's.
            let stride = align_up(element.size, element.align);
            Layout {
                size: stride.saturating_mul(*length),
                align: element.align.max(1),
                offsets: Vec::new(),
            }
        }
        TypeKind::Str
        | TypeKind::Ptr(_)
        | TypeKind::CPtr(_)
        | TypeKind::Map(_, _)
        | TypeKind::Mutex
        | TypeKind::Channel(_) => Layout {
            size: 8,
            align: 8,
            offsets: Vec::new(),
        },
        TypeKind::Slice(_) => Layout {
            size: 24,
            align: 8,
            offsets: Vec::new(),
        },
        TypeKind::View(_) => Layout {
            size: 16,
            align: 8,
            offsets: Vec::new(),
        },
        // A function value is a code pointer and an environment pointer.
        TypeKind::Func(_, _) => Layout {
            size: 16,
            align: 8,
            offsets: Vec::new(),
        },
        TypeKind::Unit | TypeKind::Never | TypeKind::CVoid => Layout {
            size: 0,
            align: 1,
            offsets: Vec::new(),
        },
        TypeKind::I64 | TypeKind::F64 | TypeKind::CSize => Layout {
            size: 8,
            align: 8,
            offsets: Vec::new(),
        },
        TypeKind::CInt(width) | TypeKind::CUInt(width) => match width {
            8 => Layout {
                size: 1,
                align: 1,
                offsets: Vec::new(),
            },
            16 => Layout {
                size: 2,
                align: 2,
                offsets: Vec::new(),
            },
            _ => Layout {
                size: 4,
                align: 4,
                offsets: Vec::new(),
            },
        },
        TypeKind::CFloat(32) => Layout {
            size: 4,
            align: 4,
            offsets: Vec::new(),
        },
        TypeKind::CFloat(_) => Layout {
            size: 8,
            align: 8,
            offsets: Vec::new(),
        },
        TypeKind::Bool | TypeKind::Char => Layout {
            size: 1,
            align: 1,
            offsets: Vec::new(),
        },
    })
}

/// Lay out a list of field types by the natural-alignment rule.
fn fields_layout(fields: &[TypeKind], decls: &dyn Decls) -> Option<Layout> {
    let fields = fields
        .iter()
        .map(|field| layout(field, decls))
        .collect::<Option<Vec<_>>>()?;
    Some(fields_layout_from(fields))
}

/// The shared walk: each field starts at the next offset that satisfies its
/// alignment, and the total is rounded up to the largest alignment.
fn fields_layout_from(fields: Vec<Layout>) -> Layout {
    let mut size = 0usize;
    let mut align = 1usize;
    let mut offsets = Vec::with_capacity(fields.len());
    for field in fields {
        let field_align = field.align.max(1);
        size = align_up(size, field_align);
        offsets.push(size);
        size = size.saturating_add(field.size);
        align = align.max(field_align);
    }
    Layout {
        size: align_up(size, align),
        align,
        offsets,
    }
}

/// Round `size` up to a multiple of `align`.
fn align_up(size: usize, align: usize) -> usize {
    let align = align.max(1);
    size.div_ceil(align) * align
}
