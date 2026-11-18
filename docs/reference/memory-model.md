# Memory model

## Allocation is zeroed

**OWN-ZERO.** Every byte a safe `alloc` hands out is zero. A program cannot make
its output differ between runs by reading uninitialised memory, because there is
no such state on the safe path. The allocator memsets every byte before it
returns a block, on both the normal and the small-allocation path.

An explicit uninitialised allocation is deliberately absent: it is an `unsafe`
operation and belongs with the `unsafe` block work. A spelling that is not
implemented is not silently safe.

## There is no `Drop`

**OWN-6.** Release is explicit. There is no destructor that runs at scope exit;
`defer` releases at scope exit and `.free()` releases a value the program owns.
Arrays, maps, channels, mutexes and built strings are released with `.free()`,
and a value that is still owned at the point it is dropped is a
[leak](../errors/E1000.md) (**OWN-2**).

## Pointers have two kinds

**ALIAS-1.** An owning `*mut T` can be transferred but not silently copied: a
second copy of the same ownership without `move` is
[E1005](../errors/E1005.md). An FFI `*const T` or `*T` is a borrowed address
that the compiler does not own; dereferencing one outside an `unsafe` block is
not permitted by the effect rules.

**OWN-3.** A partial move out of a struct leaves the rest unusable; the checker
reports [E1002](../errors/E1002.md) rather than letting a later field read see
a hole.

## Arenas are scoped

**OWN-4.** A value allocated in an `arena { }` block cannot outlive that block;
an attempt to return one is [E1003](../errors/E1003.md).

**OWN-5.** An arena is released once. Calling `.free()` on an already released
arena is [E1004](../errors/E1004.md), not a double free.

## Provenance and `noalias`

The optimizer may assume that two distinct owning pointers to live allocations
do not alias, because ownership makes such a pair unrepresentable in safe code.
This is a permission the verifier grants, not a fact the programmer promises.
The aliasing rules that cannot yet be checked statically are listed on the
[aliasing page](aliasing.md).
