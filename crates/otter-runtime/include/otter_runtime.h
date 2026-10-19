/* Runtime helpers called by every Otter program.
 *
 * The generated LLVM IR declares and calls these symbols by name, so the names
 * here are part of the contract between the code generator and the runtime:
 *
 *     declare void @otter_print_str(i8*)
 *     declare void @otter_print_i64(i64)
 *     declare void @otter_print_f64(double)
 *
 * The Otter-facing spellings are print_str / print_i64 / print_f64; the
 * `otter_` prefix keeps them from colliding with anything in libc.
 */
#ifndef OTTER_RUNTIME_H
#define OTTER_RUNTIME_H

#ifdef __cplusplus
extern "C" {
#endif

/* Writes the NUL-terminated string to stdout, byte for byte. Nothing else: no
 * newline, and no line-ending translation (see the note in runtime.c). */
void otter_print_str(const char *s);

/* Writes a decimal integer to stdout, no newline. */
void otter_print_i64(long long value);

/* Writes a double round-trippably (%.17g) to stdout, no newline. */
void otter_print_f64(double value);

/* ---- Heap allocation ----
 *
 *     declare i8* @otter_alloc(i64, i64)
 *     declare void @otter_free(i8*)
 *
 * `otter_alloc(size, align)` returns `size` bytes aligned to `align` (rounded
 * up to a power of two no smaller than a pointer), or NULL if the allocation
 * fails. The memory is uninitialised. `otter_free` releases a block and
 * tolerates NULL. Every block from `otter_alloc` must reach `otter_free`
 * exactly once; the compiler's linearity checker is what enforces it. */
void *otter_alloc(long long size, long long align);
void otter_free(void *ptr);

/* ---- Arenas ----
 *
 *     declare i8* @otter_arena_new()
 *     declare i8* @otter_arena_alloc(i8*, i64, i64)
 *     declare void @otter_arena_free_all(i8*)
 *
 * An arena hands out many allocations and releases them all at once. Individual
 * arena allocations are never freed: `otter_arena_free_all` frees every block
 * and the header in one call. An arena value is an ordinary pointer as far as
 * the generated code is concerned, so the IR treats it as `i8*`.
 *
 * An arena is **single-threaded**: allocation is an unsynchronized pointer
 * increment, so two threads calling `otter_arena_alloc` on one arena race. A
 * program that shares an arena across threads must serialize the calls. The
 * runtime's internal string arena is thread-local for exactly this reason, and
 * `otter_alloc`/`otter_free` go to `malloc`/`free`, which are thread-safe. */
typedef struct otter_arena otter_arena;
otter_arena *otter_arena_new(void);
void *otter_arena_alloc(otter_arena *arena, long long size, long long align);
void otter_arena_free_all(otter_arena *arena);

/* ---- Strings ----
 *
 *     declare i64 @otter_str_len(i8*)
 *     declare i64 @otter_str_byte_at(i8*, i64)
 *     declare i8* @otter_str_concat(i8*, i8*)
 *     declare i8* @otter_str_from_i64(i64)
 *     declare i8* @otter_str_from_f64(double)
 *     declare i8* @otter_str_from_bool(i64)
 *     declare i1  @otter_str_eq(i8*, i8*)
 *
 * A `string` value in generated code is an `i8*`: a pointer to a NUL-terminated
 * byte sequence. That is M0-M2's representation, kept for M3, and ledger L4
 * records the interim (the length-prefixed, refcounted `%otter_str` layout in
 * ledger L2 replaces it, at which point these signatures change with it).
 *
 * `otter_str_from_bool` takes an `i64` rather than an `i1`: C has no one-bit
 * type, so `i1` would have to be spelled `int` here and the two declarations
 * would disagree about how many bytes the callee may read. Generated code
 * zero-extends the `i1` to `i64` instead, so there is nothing to disagree about.
 *
 * The strings these functions *produce* live in one internal arena and are
 * released when the process exits. A built string has no owner to release -- the
 * value is a bare `char*` with no refcount and no release operation in the
 * language -- so leaking it until exit is the correct reading of that, and a
 * use-after-free is not. */
long long otter_str_len(const char *s);
long long otter_str_byte_at(const char *s, long long index);
const char *otter_str_concat(const char *a, const char *b);
const char *otter_str_from_i64(long long value);
const char *otter_str_from_f64(double value);
const char *otter_str_from_bool(long long value);
int otter_str_eq(const char *a, const char *b);

/* Reads one line from standard input without its terminator, built in the
 * string arena; returns the empty string at end of input. The parsers return
 * zero when their argument is not a number (see the input section of runtime.c
 * for why that is the whole story today). */
const char *otter_read_line(void);
long long otter_parse_i64(const char *s);
double otter_parse_f64(const char *s);

/* ---- Diagnostics ----
 *
 *     declare void @otter_panic(i8*, i8*, i64, i64)
 *
 * Writes `<file>:<line>:<column>: panic: <message>` to standard error and ends
 * the process with a failure status. It never returns, so generated IR puts
 * `unreachable` after the call: nothing in a function that panics can be
 * reached.
 *
 * It does not unwind, so a `defer` body does not run before a panic yet. The
 * diagnostics section of runtime.c is where that limitation and its replacement
 * are written down. */
void otter_panic(const char *message, const char *file, long long line, long long column);

/* ---- Arithmetic faults ----
 *
 *     declare void @otter_panic_overflow(i8*, i64, i64, i8*, i64, i64)
 *     declare void @otter_panic_division(i8*, i64, i64, i8*, i64, i64)
 *
 * The two ways a checked integer operation stops a program: `op` is its symbol
 * (`+`, `-`, `*`, `/`, `%`), `lhs` and `rhs` are the operands, and the rest is
 * the position the checker resolved. Generated code branches here when the check
 * fails, so neither returns and both are followed by `unreachable`.
 *
 * `otter_panic_division` names the fault from the operands: a zero divisor is
 * `division by zero`, and `INT64_MIN / -1` -- the only other division that has no
 * answer in an `i64` -- is an `integer overflow`.
 *
 * `-i64::MIN` reports through `otter_panic_overflow` with `0` as the left operand
 * and `-` as the symbol, because that is what negation is. */
void otter_panic_overflow(
    const char *op,
    long long lhs,
    long long rhs,
    const char *file,
    long long line,
    long long column
);
void otter_panic_division(
    const char *op,
    long long lhs,
    long long rhs,
    const char *file,
    long long line,
    long long column
);

#ifdef __cplusplus
}
#endif

#endif /* OTTER_RUNTIME_H */
