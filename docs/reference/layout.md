# Layout

## One table

The compiler computes size, alignment and field offsets once, in the HIR layout
walk, and every surface reads that table. `sizeof(T)`, `alignof(T)` and
`offsetof(T, f)` fold from it; `yashyc inspect --layout` renders it; code
generation lays out aggregates from the same rules. A disagreement between two
of those is a compiler bug.

**LAYOUT-1.** A packed field that would be misaligned is reported at the field
as [E0607](../errors/E0607.md), never loaded unaligned on purpose.

**TYPE-8.** `sizeof` of a type whose layout is unknown is
[E0107](../errors/E0107.md); the checker refuses to guess a size.

**TYPE-9.** `offsetof` on a type without `repr(C)` is
[E0108](../errors/E0108.md), because a field order the compiler may reorder has
no stable offset.

## `repr` is a clause

`repr(C)`, `repr(packed)`, `repr(align(N))`, `repr(transparent)` and an integer
enum repr are clauses on a struct, enum or object — not attributes and not an
attribute system growing `#[...]` spelling. `repr(C)` is never reordered: the
source order is the ABI order.

A non-`repr(C)` struct may have its fields reordered to remove padding when
that is free to do so. The reordering is a compiler choice and must not be
observable outside `offsetof`, which is only allowed on `repr(C)` types.

## Const arguments

A type may carry a compile-time integer:
`struct Buffer[const N: int] { data: [u8; N] }`. Each value names one
monomorphic type, so `Buffer[16]` and `Buffer[64]` have separate layouts and
count separately against the instantiation budget. A function writes its const
argument at the call — `first[4]([1, 2, 3, 4])` — because a value that appears
only in a result cannot be inferred.

Both `sizeof(Buffer[16])` and `size_of[Buffer[16]]()` ask the same table. A
const argument must be a literal or a name in scope at compile time; a run-time
value is [E1524](../errors/E1524.md), not a silent zero. A declared `[T; 0]`
is legal and contributes zero bytes.

## Numeric spellings

The C spellings `u8`, `i32` and their siblings are licensed only by `repr(C)`.
Outside a `repr(C)` type, integer fields use Yashy's `int`. This keeps the
language's single integer type the default and the fixed-width types an
explicit ABI choice.
