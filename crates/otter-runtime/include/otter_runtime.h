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
 * the generated code is concerned, so the IR treats it as `i8*`. */
typedef struct otter_arena otter_arena;
otter_arena *otter_arena_new(void);
void *otter_arena_alloc(otter_arena *arena, long long size, long long align);
void otter_arena_free_all(otter_arena *arena);

#ifdef __cplusplus
}
#endif

#endif /* OTTER_RUNTIME_H */
