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

/* The refcounted string value, spelled out in the strings section below; the
 * tag is declared here because the print and diagnostic functions take one. */
typedef struct otter_str otter_str;

/* Writes the NUL-terminated string to stdout, byte for byte. Nothing else: no
 * newline, and no line-ending translation (see the note in runtime.c). */
void otter_print_str(const otter_str *s);

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
 * A `string` value in generated code is an `i8*` pointing at a `otter_str`:
 *
 *     %otter_str = type { i64, i64, i64, i8* }   ; refcount, len, cap, data
 *
 * The refcount is 1 when a string is created and copied, decremented when a
 * reference is dropped, and the string is freed when it reaches zero. A
 * string with refcount -1 is *static*: it is never freed (string literals).
 *
 * This replaces the M3 interim (bare `char*` NUL-terminated, leaking until
 * exit). Every string operation now takes and returns `otter_str*` and
 * manages the refcount, so a built string has an owner that releases it.
 *
 * `otter_str_from_bool` takes an `i64` rather than an `i1`: C has no one-bit
 * type, so `i1` would have to be spelled `int` here and the two declarations
 * would disagree about how many bytes the callee may read. Generated code
 * zero-extends the `i1` to `i64` instead, so there is nothing to disagree about. */
long long otter_str_len(const otter_str *s);
long long otter_str_byte_at(const otter_str *s, long long index);
otter_str *otter_str_concat(const otter_str *a, const otter_str *b);

/* The bytes in `[start, end)` as a new owned string. The range is clamped:
 * `start < 0` is 0, `end > len` is the length, and an inverted range is empty. */
otter_str *otter_str_slice(const otter_str *s, long long start, long long end);
otter_str *otter_str_from_i64(long long value);
otter_str *otter_str_from_f64(double value);
otter_str *otter_str_from_bool(long long value);
int otter_str_eq(const otter_str *a, const otter_str *b);

/* Content hash used by the map runtime for `string` keys (FNV-1a). A null
 * string hashes like the empty string; generated code never calls this. */
unsigned long long otter_str_hash(const otter_str *s);

/* The C boundary (spec §8.1). `otter_str_cstr` returns a fresh, owned,
 * NUL-terminated copy of the bytes, allocated with `otter_alloc` so the caller
 * frees it with `otter_free`. `otter_str_ptr` returns the string's own data
 * pointer and transfers nothing: it is only for an `unsafe` call, which tells
 * the compiler C may keep the pointer after returning.
 *
 *     declare i8* @otter_str_cstr(i8*)
 *     declare i8* @otter_str_ptr(i8*)
 */
char *otter_str_cstr(const otter_str *s);
const char *otter_str_ptr(const otter_str *s);

/* Reads one line from standard input without its terminator, as a string the
 * caller *owns* (refcount 1); returns the empty string at end of input. The
 * parsers return zero when their argument is not a number (see the input
 * section of runtime.c for why that is the whole story today). */
otter_str *otter_read_line(void);
long long otter_parse_i64(const otter_str *s);
double otter_parse_f64(const otter_str *s);

/* ---- Files and the environment ----------------------------------------
 *
 * `otter_read_file` returns the file's bytes as an owned string, or the empty
 * string on failure (the empty string is also an empty file). `otter_write_file`
 * writes exactly `len` bytes and returns 1 on success, 0 on failure.
 * `otter_getenv` returns the value as an owned string, or the empty string when
 * the name is unset. Every `otter_str*` these return is owned: release it. */
otter_str *otter_read_file(const char *path);
long long otter_write_file(const char *path, const char *bytes, long long len);
otter_str *otter_getenv(const char *name);

/* Reference counting. `otter_str_retain` adds one to the count and returns its
 * argument; `otter_str_release` drops one and frees the string when the count
 * reaches zero. A static string (refcount -1, a literal) is never freed, so
 * both are no-ops on one -- which is what lets a binding keep a literal as
 * cheaply as it keeps a built string. */
otter_str *otter_str_retain(otter_str *s);
void otter_str_release(otter_str *s);

/* The atomic pair, for a string that can be shared with a spawned thread.
 * Same contract as the plain pair; the count is updated with atomic
 * operations, so concurrent retains and releases neither lose an update nor
 * free the bytes while another thread still holds a reference. */
otter_str *otter_str_retain_atomic(otter_str *s);
void otter_str_release_atomic(otter_str *s);

/* The current reference count: 1 for a live owned string, -1 for a static
 * literal, 0 for NULL. For the runtime's own concurrency test. */
long long otter_str_refcount(const otter_str *s);

/* ---- Slices ----
 *
 *     declare void @otter_slice_push({ i8*, i64, i64 }*, i8*, i64)
 *     declare void @otter_slice_free({ i8*, i64, i64 }*)
 *
 * A slice value is three words -- data pointer, length, capacity -- and the
 * generated header has exactly the layout of `otter_slice`. The header is
 * passed by pointer because `push` may reallocate the backing array and must
 * update the length and capacity in the caller's binding. `element_size` is the
 * element's stride in bytes; it is a compile-time constant at every call site.
 *
 * `push` grows from 0 to 4 elements and then doubles, copies the old bytes, and
 * frees the old block with `otter_free`, so a slice owns exactly one live block
 * at a time (which keeps the leak report meaningful). `otter_slice_free`
 * releases that block and zeroes the header; a null data pointer is fine.
 *
 * Neither function reports an allocation failure: the runtime has no way to
 * return one from `push`, and `otter_alloc` already yields NULL. A failed push
 * leaves the slice unchanged, which is the only state a caller could observe. */
typedef struct otter_slice {
    void *data;
    long long len;
    long long cap;
} otter_slice;
void otter_slice_push(otter_slice *slice, const void *element, long long element_size);
void otter_slice_free(otter_slice *slice);

/* Frees a `[]string`: each element is an `otter_str*` the slice owns, so every
 * handle is released before the backing array is. A static string is left
 * alone by `otter_str_release`, and a null slice is a no-op. The header is
 * zeroed, like `otter_slice_free`. */
void otter_slice_free_strings(otter_slice *slice);

/* `otter_slice_get` copies the element at `index` into `out` and returns 1, or
 * returns 0 and leaves `out` untouched when `index` is negative or at least
 * `len`. `otter_slice_pop` is the same for the last element, after decrementing
 * `len`; an empty slice returns 0. Both take the stride so the runtime stays
 * independent of the element type. A null slice is a miss, not a crash.
 *
 * Indexing is how the language reads a slice, and it never traps: the 0/1
 * answer becomes an `Option`, so "out of range" is a value rather than a
 * process ending. */
long long otter_slice_get(otter_slice *slice, long long index, void *out, long long element_size);
long long otter_slice_pop(otter_slice *slice, void *out, long long element_size);

/* ---- Maps ----
 *
 *     declare i8* @otter_map_new(i64, i64, i64)
 *     declare void @otter_map_free(i8*)
 *     declare void @otter_map_insert(i8*, ptr, ptr)
 *     declare i64 @otter_map_get(i8*, ptr, ptr)
 *     declare i64 @otter_map_remove(i8*, ptr, ptr)
 *     declare i64 @otter_map_len(i8*)
 *
 * A map value is an opaque pointer to an open-addressing table. `map_new`
 * records the *key kind* once -- 0 = `i64`, 1 = `bool`, 2 = `string` -- plus
 * the two element strides, and every later call is just the table, a key slot
 * and a value slot. The table allocates its arrays on the first insert.
 *
 * `otter_map_insert` copies the key and the value. A `string` key is a handle:
 * the table retains it, so the map owns its own reference and the caller's
 * binding is untouched. Overwriting an existing key keeps the stored key (and
 * its reference) and replaces only the value. `otter_map_remove` copies the
 * value out, releases a stored string key, and leaves a tombstone so probing
 * stays correct; `otter_map_free` releases every string key and every array.
 *
 * `get` and `remove` return 1 with the value in `out`, or 0 and leave `out`
 * alone. The caller always zeroes `out` first, so the bytes behind a 0 are
 * defined even though they must not be read.
 *
 * `insert` reports nothing, like `push`: on a failed allocation the table is
 * left unchanged, which is the only state the caller could observe. */
typedef struct otter_map otter_map;
otter_map *otter_map_new(long long key_kind, long long key_size, long long value_size);
void otter_map_free(otter_map *map);
void otter_map_insert(otter_map *map, const void *key, const void *value);
long long otter_map_get(const otter_map *map, const void *key, void *out);
long long otter_map_remove(otter_map *map, const void *key, void *out);
long long otter_map_len(const otter_map *map);

/* ---- Threads, mutexes and channels ------------------------------------
 *
 *     declare i32 @otter_thread_spawn(i8*, i8*, i64)
 *     declare i32 @otter_thread_join(i32)
 *     declare i8* @otter_mutex_new()
 *     declare void @otter_mutex_lock(i8*)
 *     declare void @otter_mutex_unlock(i8*)
 *     declare void @otter_mutex_free(i8*)
 *     declare i8* @otter_chan_new(i64)
 *     declare i8 @otter_chan_send(i8*, i8*, i64)
 *     declare i8 @otter_chan_recv(i8*, i8*, i64)
 *     declare void @otter_chan_close(i8*)
 *     declare void @otter_chan_free(i8*)
 *
 * `otter_thread_spawn` copies `arg_size` bytes of `arg` into storage the new
 * thread owns, because the spawning frame may be gone before the thread starts.
 * The entry receives a pointer to that copy and must not keep it after it
 * returns. The handle is a positive index, or 0 when the thread could not be
 * created; it must be joined exactly once, and joining waits for the body and
 * releases the copy. A second join of the same handle answers 1, as does a
 * handle that was never returned by `spawn`.
 *
 * A mutex and a channel are opaque handles. Freeing either while another thread
 * still holds, waits for, or sends on it is a use-after-free, exactly as it
 * would be for a string: the language's linear typing is what rules it out.
 *
 * A channel is a bounded queue of fixed-size slots. The slot size is fixed by
 * the first send or receive; a later call with a different size answers 0
 * instead of overwriting a slot, which can only mean the checker and the
 * runtime disagree. `send` blocks while the channel is full and answers 1 when
 * the slot was queued, 0 when the channel is closed. `recv` blocks while the
 * channel is empty and answers 1 when a slot was copied out; it answers 0 only
 * once the channel is closed *and* drained, so a close never discards a value
 * that was already sent. `close` never blocks, wakes every waiter, and may be
 * called more than once. A capacity below 1 is treated as 1: a zero-capacity
 * rendezvous needs a handshake this ring does not model, and treating it as a
 * one-slot channel is the only answer that cannot deadlock.
 */
typedef struct otter_mutex otter_mutex;
typedef struct otter_chan otter_chan;

/* The C spelling of the entry point. Generated code passes the same pointer as
 * an `i8*`; a function pointer and an object pointer share one representation
 * on every target Otter supports, so the two declarations are ABI-identical. */
typedef void (*otter_thread_entry)(void *argument);
int otter_thread_spawn(otter_thread_entry entry, const void *arg, long long arg_size);
int otter_thread_join(int handle);

otter_mutex *otter_mutex_new(void);
void otter_mutex_lock(otter_mutex *mutex);
void otter_mutex_unlock(otter_mutex *mutex);
void otter_mutex_free(otter_mutex *mutex);

otter_chan *otter_chan_new(long long capacity);
char otter_chan_send(otter_chan *chan, const void *item, long long size);
char otter_chan_recv(otter_chan *chan, void *out_item, long long size);
void otter_chan_close(otter_chan *chan);
void otter_chan_free(otter_chan *chan);

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
void otter_panic(const otter_str *message, const otter_str *file, long long line, long long column);

/* ---- Exiting ----
 *
 *     declare void @otter_exit(i64)
 *
 * Ends the process with `code` as its exit status, after flushing both streams.
 * This is what `os.exit(code)` lowers to; it never returns, so generated IR puts
 * `unreachable` after the call. An `i64` rather than the `i32` the operating
 * system reads because `i64` is the only integer Otter has; the cast happens
 * here, once. */
void otter_exit(long long code);

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
    const otter_str *op,
    long long lhs,
    long long rhs,
    const otter_str *file,
    long long line,
    long long column
);
void otter_panic_division(
    const otter_str *op,
    long long lhs,
    long long rhs,
    const otter_str *file,
    long long line,
    long long column
);

#ifdef __cplusplus
}
#endif

#endif /* OTTER_RUNTIME_H */
