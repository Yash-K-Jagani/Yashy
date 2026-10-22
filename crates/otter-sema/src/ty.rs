//! Types used by the type checker.
//!
//! A `Type` carries a `TypeKind` plus the span where it was written or
//! inferred. Two `Type`s are equal when their kinds are equal, regardless of
//! span: a type error points at the span of the expression that went wrong,
//! not at the span where a matching type was first declared.

use otter_common::Span;

/// The shape of a value. Every variant except `Never` is a real type a
/// program can contain; `Never` is the type of expressions that never
/// evaluate (diverging calls, unreachable code) and is never written by the
/// user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeKind {
    I64,
    F64,
    Bool,
    Str,
    Unit,
    /// A declared struct, named by its declaration name.
    Struct(String),
    /// A declared enum, named by its declaration name.
    Enum(String),
    Option(Box<Type>),
    Result(Box<Type>, Box<Type>),
    /// A fixed-size array `[T; N]`: storage in the enclosing frame.
    ///
    /// Copyable and non-linear: it owns no heap, and copying it copies the
    /// elements. `N` is part of the type, so `[i64; 3]` and `[i64; 4]` are
    /// different types.
    FixedArray(Box<Type>, usize),
    /// A borrowed view `[T]` of a run of elements.
    ///
    /// A view owns nothing -- it is a length and a pointer into an array that
    /// something else owns -- so it is copyable and never freed. It cannot be
    /// returned from the function that made it, which is what keeps the
    /// pointer from outliving its storage.
    View(Box<Type>),
    /// A growable, linearly owned sequence of the element type, written `[]T`.
    ///
    /// Linear for the same reason a raw pointer is: exactly one binding owns
    /// the backing array, and `free_slice` gives it up. The element type is
    /// part of the type, so `[]i64` and `[]string` are different types even
    /// though both lower to the same three-word header.
    Slice(Box<Type>),
    /// A linearly owned table from a key type to a value type, written
    /// `map[K]V`.
    ///
    /// Linear like a slice: the binding owns the table, `map_free` gives it
    /// up, and `map_insert`/`map_get` borrow it. The runtime hashes and
    /// compares keys by kind, so the key is restricted to `i64`, `bool`, and
    /// `string`; values are restricted to numbers and `bool` for now, because
    /// an owned value would need a destructor on removal.
    Map(Box<Type>, Box<Type>),
    /// A mutex handle, written `Mutex`.
    ///
    /// Linear, because the runtime handle has to be freed exactly once and
    /// copying it would let two owners free the same lock. The runtime never
    /// hands the handle's internals back, so generated code sees one opaque
    /// pointer.
    Mutex,
    /// A bounded channel of `T`, written `Channel[T]`.
    ///
    /// Linear like a mutex: the binding owns the channel, and `chan_free` gives
    /// it up. A spawned thread receives a *borrowed* handle, which is the one
    /// way a handle reaches a second thread without a copy.
    Channel(Box<Type>),
    /// A raw owning pointer to `T`, written `*mut T`. Linear (see
    /// [`crate::ownership`]): at most one binding owns it at a time.
    Ptr(Box<Type>),
    /// A signed C integer of the given bit width: `i8`, `i16`, `i32`.
    ///
    /// `i64` does not appear here: it is the same type as [`TypeKind::I64`],
    /// so a C `int64_t` and an Otter `i64` meet without a cast.
    CInt(u32),
    /// An unsigned C integer: `u8`, `u16`, `u32`, `u64`.
    CUInt(u32),
    /// A C floating-point type of the given bit width: `f32`.
    ///
    /// `f64` is [`TypeKind::F64`], for the same reason `i64` is.
    CFloat(u32),
    /// C `void`. Only legal as an `extern` declaration's return type.
    CVoid,
    /// A C pointer: `*T`, `*mut T`, or `*const T` inside an `extern`
    /// declaration. Linear, because C code may keep or free what it points at
    /// and the checker cannot see across the boundary.
    CPtr(Box<Type>),
    /// C `size_t`, spelled `usize`. An unsigned machine word; lowered as `i64`
    /// on the platforms Otter targets.
    CSize,
    /// A first-class function value: parameters and return type, written
    /// `fn(T, U) -> R`.
    ///
    /// A top-level function name has this type, and so does a binding or
    /// parameter that holds one. The backend represents every value of this
    /// type as a `{ i8* code, i8* env }` pair; a non-capturing function leaves
    /// the environment null.
    Func(Vec<Type>, Box<Type>),
    /// A tuple `(A, B, ...)`: an anonymous product of values.
    ///
    /// The elements are a list, so `(i64, bool)` and `(bool, i64)` are
    /// different types -- position is part of the type, which is the whole
    /// difference between a tuple and a struct with unnamed fields. Tuples are
    /// copied by copying their bytes, so an element that owns a reference has
    /// no meaning here yet; the checker keeps the elements to the built-in
    /// value types and to other tuples of them.
    Tuple(Vec<Type>),
    /// No value: the expression diverges. Not written by users.
    Never,
}

/// A type with the span where it appears. Equality ignores spans so that two
/// types written in different places still compare equal; the span is used
/// only to point diagnostics at the right source text.
#[derive(Clone, Debug)]
pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}

impl PartialEq for Type {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
    }
}

impl Eq for Type {}

impl Type {
    pub fn eq_ignoring_span(&self, other: &Type) -> bool {
        self.kind == other.kind
    }
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            TypeKind::I64 => write!(f, "i64"),
            TypeKind::F64 => write!(f, "f64"),
            TypeKind::Bool => write!(f, "bool"),
            TypeKind::Str => write!(f, "string"),
            TypeKind::Unit => write!(f, "()"),
            TypeKind::Struct(name) => write!(f, "{name}"),
            TypeKind::Enum(name) => write!(f, "{name}"),
            TypeKind::Option(inner) => write!(f, "Option[{inner}]"),
            TypeKind::Result(ok, err) => write!(f, "Result[{ok}, {err}]"),
            TypeKind::Slice(element) => write!(f, "[]{element}"),
            TypeKind::FixedArray(element, length) => write!(f, "[{element}; {length}]"),
            TypeKind::View(element) => write!(f, "[{element}]"),
            TypeKind::Map(key, value) => write!(f, "map[{key}]{value}"),
            TypeKind::Mutex => write!(f, "Mutex"),
            TypeKind::Channel(element) => write!(f, "Channel[{element}]"),
            TypeKind::Ptr(pointee) => write!(f, "*mut {pointee}"),
            TypeKind::CInt(width) => write!(f, "i{width}"),
            TypeKind::CUInt(width) => write!(f, "u{width}"),
            TypeKind::CFloat(width) => write!(f, "f{width}"),
            TypeKind::CVoid => write!(f, "void"),
            TypeKind::CPtr(pointee) => write!(f, "*{pointee}"),
            TypeKind::CSize => write!(f, "usize"),
            TypeKind::Func(params, ret) => {
                write!(f, "fn(")?;
                let mut first = true;
                for p in params {
                    if !first {
                        write!(f, ", ")?;
                    }
                    first = false;
                    write!(f, "{p}")?;
                }
                write!(f, ") -> {ret}")
            }
            TypeKind::Never => write!(f, "!"),
            TypeKind::Tuple(elements) => {
                write!(f, "(")?;
                let mut first = true;
                for element in elements {
                    if !first {
                        write!(f, ", ")?;
                    }
                    first = false;
                    write!(f, "{element}")?;
                }
                if elements.len() == 1 {
                    write!(f, ",")?;
                }
                write!(f, ")")
            }
        }
    }
}

impl TypeKind {
    /// The type a C spelling names, or `None` when the word is not one of the
    /// spellings the FFI edge adds.
    ///
    /// The widths Otter already had keep their own kinds: `i64` is
    /// [`TypeKind::I64`] and `f64` is [`TypeKind::F64`], not `CInt(64)` /
    /// `CFloat(64)`. A C `int64_t` and an Otter `i64` are the same machine
    /// value, and two kinds for it would turn every boundary into a cast
    /// without adding a single check.
    pub fn from_c_spelling(name: &str) -> Option<TypeKind> {
        Some(match name {
            "i8" => TypeKind::CInt(8),
            "i16" => TypeKind::CInt(16),
            "i32" => TypeKind::CInt(32),
            "i64" => TypeKind::I64,
            "u8" => TypeKind::CUInt(8),
            "u16" => TypeKind::CUInt(16),
            "u32" => TypeKind::CUInt(32),
            "u64" => TypeKind::CUInt(64),
            "f32" => TypeKind::CFloat(32),
            "f64" => TypeKind::F64,
            "usize" => TypeKind::CSize,
            "void" => TypeKind::CVoid,
            _ => return None,
        })
    }

    /// Whether `name` may appear only inside an `extern` declaration.
    ///
    /// `i64` and `f64` are deliberately absent: they are ordinary Otter types
    /// that the FFI edge happens to spell the same way.
    pub fn is_c_only_spelling(name: &str) -> bool {
        matches!(
            name,
            "i8" | "i16" | "i32" | "u8" | "u16" | "u32" | "u64" | "f32" | "usize" | "void"
        )
    }
}

/// Mutability of a binding. `Const` is the default; `Var` is explicit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mutability {
    Const,
    Var,
}

impl Mutability {
    pub fn is_mutable(self) -> bool {
        matches!(self, Mutability::Var)
    }
}

/// A type that never changes kind under unification; used internally when the
/// checker needs to say "this expression is known to diverge".
pub fn never(span: Span) -> Type {
    Type {
        kind: TypeKind::Never,
        span,
    }
}
