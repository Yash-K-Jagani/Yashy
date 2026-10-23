/* Runtime helpers called by every Yashy program.
 *
 * The generated LLVM IR declares and calls these symbols by name, so the names
 * here are part of the contract between the code generator and the runtime:
 *
 *     declare void @yashy_print_str(i8*)
 *     declare void @yashy_print_i64(i64)
 *     declare void @yashy_print_f64(double)
 *
 * The Yashy-facing spellings are print_str / print_i64 / print_f64; the
 * `yashy_` prefix keeps them from colliding with anything in libc.
 */
#ifndef YASHY_RUNTIME_H
#define YASHY_RUNTIME_H

#ifdef __cplusplus
extern "C" {
#endif

/* The refcounted string value, spelled out in the strings section below; the
 * tag is declared here because the print and diagnostic functions take one. */
typedef struct yashy_str yashy_str;

/* Writes the NUL-terminated string to stdout, byte for byte. Nothing else: no
 * newline, and no line-ending translation (see the note in runtime.c). */
void yashy_print_str(const yashy_str *s);

/* Writes a decimal integer to stdout, no newline. */
void yashy_print_i64(long long value);

/* Writes a double round-trippably (%.17g) to stdout, no newline. */
void yashy_print_f64(double value);

/* ---- Heap allocation ----
 *
 *     declare i8* @yashy_alloc(i64, i64)
 *     declare void @yashy_free(i8*)
 *
 * `yashy_alloc(size, align)` returns `size` bytes aligned to `align` (rounded
 * up to a power of two no smaller than a pointer), or NULL if the allocation
 * fails. The memory is uninitialised. `yashy_free` releases a block and
 * tolerates NULL. Every block from `yashy_alloc` must reach `yashy_free`
 * exactly once; the compiler's linearity checker is what enforces it. */
void *yashy_alloc(long long size, long long align);
void yashy_free(void *ptr);

/* ---- Arenas ----
 *
 *     declare i8* @yashy_arena_new()
 *     declare i8* @yashy_arena_alloc(i8*, i64, i64)
 *     declare void @yashy_arena_free_all(i8*)
 *
 * An arena hands out many allocations and releases them all at once. Individual
 * arena allocations are never freed: `yashy_arena_free_all` frees every block
 * and the header in one call. An arena value is an ordinary pointer as far as
 * the generated code is concerned, so the IR treats it as `i8*`.
 *
 * An arena is **single-threaded**: allocation is an unsynchronized pointer
 * increment, so two threads calling `yashy_arena_alloc` on one arena race. A
 * program that shares an arena across threads must serialize the calls. The
 * runtime's internal string arena is thread-local for exactly this reason, and
 * `yashy_alloc`/`yashy_free` go to `malloc`/`free`, which are thread-safe. */
typedef struct yashy_arena yashy_arena;
yashy_arena *yashy_arena_new(void);
void *yashy_arena_alloc(yashy_arena *arena, long long size, long long align);
void yashy_arena_free_all(yashy_arena *arena);

/* ---- Closure environments ----
 *
 *     declare i8* @yashy_env_new(i64, i64, i64)
 *     declare i8* @yashy_env_retain(i8*)
 *     declare void @yashy_env_release(i8*)
 *
 * A closure carries the values it captured in an *environment*: a block the
 * closure's code reads through. The environment outlives the function that
 * made the closure, so it is reference counted: the closure value holds one
 * reference, a copy of the value holds another, and the block is freed when
 * the last one lets go.
 *
 * The generated code builds the block; the runtime owns the count and the
 * release of what the block captured. The layout is:
 *
 *   [0]   long long   the reference count
 *   [8]   long long   how many captured values need releasing
 *   [16]  the release table: `count` pairs of (kind, pointer)
 *   ...   the captured values themselves
 *
 * The table says what to release when the count reaches zero: a captured
 * `string` is a reference the environment owns, and the table names it. A
 * captured number owns nothing, and its table entry says so.
 *
 * The header is `YASHY_ENV_HEADER_SIZE` bytes and a table entry is
 * `YASHY_ENV_SLOT_SIZE` bytes; the generated code uses the same two numbers
 * to find the captured values, and the two sides are pinned by the tests on
 * each side.
 */
#define YASHY_ENV_HEADER_SIZE 16
#define YASHY_ENV_SLOT_SIZE 16

/* A new environment with `slot_count` captured values in a payload of
 * `payload_size` bytes, and one reference. The captured values start
 * `YASHY_ENV_HEADER_SIZE + slot_count * YASHY_ENV_SLOT_SIZE` bytes into the
 * block. NULL if the allocation fails. */
void *yashy_env_new(long long slot_count, long long payload_size, long long align);

/* One more reference to `env`, and the environment itself. NULL stays NULL. */
void *yashy_env_retain(void *env);

/* One fewer reference to `env`. When the last one goes, the captured strings
 * are released and the block is freed. NULL is left alone. */
void yashy_env_release(void *env);

/* ---- Strings ----
 *
 *     declare i64 @yashy_str_len(i8*)
 *     declare i64 @yashy_str_byte_at(i8*, i64)
 *     declare i8* @yashy_str_concat(i8*, i8*)
 *     declare i8* @yashy_str_from_i64(i64)
 *     declare i8* @yashy_str_from_f64(double)
 *     declare i8* @yashy_str_from_bool(i64)
 *     declare i1  @yashy_str_eq(i8*, i8*)
 *
 * A `string` value in generated code is an `i8*` pointing at a `yashy_str`:
 *
 *     %yashy_str = type { i64, i64, i64, i8* }   ; refcount, len, cap, data
 *
 * The refcount is 1 when a string is created and copied, decremented when a
 * reference is dropped, and the string is freed when it reaches zero. A
 * string with refcount -1 is *static*: it is never freed (string literals).
 *
 * This replaces the M3 interim (bare `char*` NUL-terminated, leaking until
 * exit). Every string operation now takes and returns `yashy_str*` and
 * manages the refcount, so a built string has an owner that releases it.
 *
 * `yashy_str_from_bool` takes an `i64` rather than an `i1`: C has no one-bit
 * type, so `i1` would have to be spelled `int` here and the two declarations
 * would disagree about how many bytes the callee may read. Generated code
 * zero-extends the `i1` to `i64` instead, so there is nothing to disagree about. */
long long yashy_str_len(const yashy_str *s);
long long yashy_str_byte_at(const yashy_str *s, long long index);
yashy_str *yashy_str_concat(const yashy_str *a, const yashy_str *b);

/* The bytes in `[start, end)` as a new owned string. The range is clamped:
 * `start < 0` is 0, `end > len` is the length, and an inverted range is empty. */
yashy_str *yashy_str_slice(const yashy_str *s, long long start, long long end);
yashy_str *yashy_str_from_i64(long long value);
yashy_str *yashy_str_from_f64(double value);
yashy_str *yashy_str_from_bool(long long value);
/* The one-byte string the byte spells: `'\xFF'` is a length-1 string holding
 * 0xFF, with no encoding applied. */
yashy_str *yashy_str_from_char(unsigned char value);
/* The byte an `int` names, or a failure status and a message when it is outside
 * 0..=255. Truncating instead would turn a mistake into a plausible character. */
unsigned char yashy_char_from_int(long long value);
int yashy_str_eq(const yashy_str *a, const yashy_str *b);

/* The debug form of a string (M15 Task 20): the text in double quotes, with
 * `"`, `\`, the common control characters and the remaining control bytes
 * escaped. Bytes at or above 0x80 are copied unchanged, so UTF-8 stays
 * readable. The result is owned by the caller, like every other `yashy_str_*`
 * constructor. */
yashy_str *yashy_str_debug(const yashy_str *s);

/* String methods (M12 Task 5). These are byte operations: case mapping is
 * ASCII only and every separator or line boundary is one byte. `get` writes the
 * byte through `out` and returns 1 for a hit, 0 for an out-of-range index.
 * `index_of` returns the first matching index or -1; `contains`, `starts_with`
 * and `ends_with` are the same content search in a different shape. The trim
 * and case helpers and `replace`/`repeat` return a fresh owned string, so the
 * caller releases it. `split` and `lines` are declared after `yashy_slice`
 * because they fill a slice header. */
long long yashy_str_get(const yashy_str *s, long long index, long long *out);
long long yashy_str_index_of(const yashy_str *s, const yashy_str *needle);
int yashy_str_contains(const yashy_str *s, const yashy_str *needle);
int yashy_str_starts_with(const yashy_str *s, const yashy_str *prefix);
int yashy_str_ends_with(const yashy_str *s, const yashy_str *suffix);
yashy_str *yashy_str_trim(const yashy_str *s);
yashy_str *yashy_str_trim_start(const yashy_str *s);
yashy_str *yashy_str_trim_end(const yashy_str *s);
yashy_str *yashy_str_to_upper(const yashy_str *s);
yashy_str *yashy_str_to_lower(const yashy_str *s);
yashy_str *yashy_str_replace(const yashy_str *s, const yashy_str *old,
                             const yashy_str *new_text);
yashy_str *yashy_str_repeat(const yashy_str *s, long long count);

/* Content hash used by the map runtime for `string` keys (FNV-1a). A null
 * string hashes like the empty string; generated code never calls this. */
unsigned long long yashy_str_hash(const yashy_str *s);

/* The C boundary (spec §8.1). `yashy_str_cstr` returns a fresh, owned,
 * NUL-terminated copy of the bytes, allocated with `yashy_alloc` so the caller
 * frees it with `yashy_free`. `yashy_str_ptr` returns the string's own data
 * pointer and transfers nothing: it is only for an `unsafe` call, which tells
 * the compiler C may keep the pointer after returning.
 *
 *     declare i8* @yashy_str_cstr(i8*)
 *     declare i8* @yashy_str_ptr(i8*)
 */
char *yashy_str_cstr(const yashy_str *s);
const char *yashy_str_ptr(const yashy_str *s);

/* Reads one line from standard input without its terminator, as a string the
 * caller *owns* (refcount 1); returns the empty string at end of input. The
 * parsers return zero when their argument is not a number (see the input
 * section of runtime.c for why that is the whole story today). */
yashy_str *yashy_read_line(void);
long long yashy_parse_i64(const yashy_str *s);
double yashy_parse_f64(const yashy_str *s);

/* ---- Files and the environment ----------------------------------------
 *
 * `yashy_read_file` returns the file's bytes as an owned string, or the empty
 * string on failure (the empty string is also an empty file). `yashy_write_file`
 * writes exactly `len` bytes and returns 1 on success, 0 on failure.
 * `yashy_getenv` returns the value as an owned string, or the empty string when
 * the name is unset. Every `yashy_str*` these return is owned: release it. */
yashy_str *yashy_read_file(const char *path);
long long yashy_write_file(const char *path, const char *bytes, long long len);
yashy_str *yashy_getenv(const char *name);

/* Reference counting. `yashy_str_retain` adds one to the count and returns its
 * argument; `yashy_str_release` drops one and frees the string when the count
 * reaches zero. A static string (refcount -1, a literal) is never freed, so
 * both are no-ops on one -- which is what lets a binding keep a literal as
 * cheaply as it keeps a built string. */
yashy_str *yashy_str_retain(yashy_str *s);
void yashy_str_release(yashy_str *s);

/* The atomic pair, for a string that can be shared with a spawned thread.
 * Same contract as the plain pair; the count is updated with atomic
 * operations, so concurrent retains and releases neither lose an update nor
 * free the bytes while another thread still holds a reference. */
yashy_str *yashy_str_retain_atomic(yashy_str *s);
void yashy_str_release_atomic(yashy_str *s);

/* The current reference count: 1 for a live owned string, -1 for a static
 * literal, 0 for NULL. For the runtime's own concurrency test. */
long long yashy_str_refcount(const yashy_str *s);

/* ---- Slices ----
 *
 *     declare void @yashy_slice_push({ i8*, i64, i64 }*, i8*, i64)
 *     declare void @yashy_slice_free({ i8*, i64, i64 }*)
 *
 * A slice value is three words -- data pointer, length, capacity -- and the
 * generated header has exactly the layout of `yashy_slice`. The header is
 * passed by pointer because `push` may reallocate the backing array and must
 * update the length and capacity in the caller's binding. `element_size` is the
 * element's stride in bytes; it is a compile-time constant at every call site.
 *
 * `push` grows from 0 to 4 elements and then doubles, copies the old bytes, and
 * frees the old block with `yashy_free`, so a slice owns exactly one live block
 * at a time (which keeps the leak report meaningful). `yashy_slice_free`
 * releases that block and zeroes the header; a null data pointer is fine.
 *
 * Neither function reports an allocation failure: the runtime has no way to
 * return one from `push`, and `yashy_alloc` already yields NULL. A failed push
 * leaves the slice unchanged, which is the only state a caller could observe. */
typedef struct yashy_slice {
    void *data;
    long long len;
    long long cap;
} yashy_slice;
void yashy_slice_push(yashy_slice *slice, const void *element, long long element_size);
void yashy_slice_free(yashy_slice *slice);

/* Frees a `[]string`: each element is an `yashy_str*` the slice owns, so every
 * handle is released before the backing array is. A static string is left
 * alone by `yashy_str_release`, and a null slice is a no-op. The header is
 * zeroed, like `yashy_slice_free`. */
void yashy_slice_free_strings(yashy_slice *slice);

/* String split and line splitting. Both fill a fresh `[]string`: every piece is
 * a new owned string, so `yashy_slice_free_strings` releases the whole result.
 * An empty separator makes `split` return the source as one piece. `lines`
 * splits on `\n`, strips one trailing `\r`, and omits the empty element after a
 * trailing newline. */
void yashy_str_split(yashy_slice *out, const yashy_str *s,
                     const yashy_str *separator);
void yashy_str_lines(yashy_slice *out, const yashy_str *s);

/* `yashy_slice_get` copies the element at `index` into `out` and returns 1, or
 * returns 0 and leaves `out` untouched when `index` is negative or at least
 * `len`. `yashy_slice_pop` is the same for the last element, after decrementing
 * `len`; an empty slice returns 0. `yashy_slice_set` copies `element` over the
 * element at `index` and returns 1, or returns 0 and leaves the slice unchanged
 * when the index is out of range. All three take the stride so the runtime stays
 * independent of the element type. A null slice is a miss, not a crash.
 *
 * Indexing is how the language reads a slice, and it never traps: the 0/1
 * answer becomes an `Option`, so "out of range" is a value rather than a
 * process ending. `set` is the same rule with a `bool` answer. */
long long yashy_slice_get(yashy_slice *slice, long long index, void *out, long long element_size);
long long yashy_slice_pop(yashy_slice *slice, void *out, long long element_size);
long long yashy_slice_set(yashy_slice *slice, long long index, const void *element,
                          long long element_size);

/* `yashy_slice_insert` accepts `0..=len` and shifts the tail right, growing
 * through the same path as `push`. `yashy_slice_remove` accepts `0..len` and
 * shifts the tail left after copying the removed element into `out`. Both
 * return 1 on success and 0, leaving the array unchanged, for a bad index or a
 * failed grow. `clear` sets the length to zero but keeps the backing array and
 * its capacity; the `_strings` variant releases each owned handle first.
 * `reverse` swaps bytes and needs only the stride. `sort_i64` sorts in place
 * and is the only ordering entry point in this phase. */
long long yashy_slice_insert(yashy_slice *slice, long long index, const void *element,
                             long long element_size);
long long yashy_slice_remove(yashy_slice *slice, long long index, void *out,
                             long long element_size);
void yashy_slice_clear(yashy_slice *slice);
void yashy_slice_clear_strings(yashy_slice *slice);
void yashy_slice_reverse(yashy_slice *slice, long long element_size);
void yashy_slice_sort_i64(yashy_slice *slice);

/* `yashy_slice_index_of_*` returns the first matching index or -1. The type is
 * part of the entry point because equality is not one operation: `f64` compares
 * numerically, `bool` compares the stored byte, and `string` compares contents.
 * `yashy_slice_extend` appends every element of `other` and then frees `other`;
 * the `_strings` variant retains each appended handle first. `yashy_slice_slice`
 * copies the clamped `start..end` range into a fresh array; the `_strings`
 * variant retains each copied handle. The result owns its backing array, and an
 * allocation failure yields the empty slice. */
long long yashy_slice_index_of_i64(yashy_slice *slice, long long value);
long long yashy_slice_index_of_f64(yashy_slice *slice, double value);
long long yashy_slice_index_of_bool(yashy_slice *slice, long long value);
long long yashy_slice_index_of_string(yashy_slice *slice, const yashy_str *value);
void yashy_slice_extend(yashy_slice *slice, yashy_slice *other,
                        long long element_size);
void yashy_slice_extend_strings(yashy_slice *slice, yashy_slice *other);
void yashy_slice_slice(yashy_slice *out, yashy_slice *source, long long start,
                       long long end, long long element_size);
void yashy_slice_slice_strings(yashy_slice *out, yashy_slice *source,
                               long long start, long long end);

/* ---- Maps ----
 *
 *     declare i8* @yashy_map_new(i64, i64, i64)
 *     declare void @yashy_map_free(i8*)
 *     declare void @yashy_map_insert(i8*, ptr, ptr)
 *     declare i64 @yashy_map_get(i8*, ptr, ptr)
 *     declare i64 @yashy_map_remove(i8*, ptr, ptr)
 *     declare i64 @yashy_map_len(i8*)
 *     declare i64 @yashy_map_contains_key(i8*, ptr)
 *     declare void @yashy_map_keys({ i8*, i64, i64 }*, i8*)
 *     declare void @yashy_map_values({ i8*, i64, i64 }*, i8*)
 *
 * A map value is an opaque pointer to an open-addressing table. `map_new`
 * records the *key kind* once -- 0 = `i64`, 1 = `bool`, 2 = `string` -- plus
 * the two element strides, and every later call is just the table, a key slot
 * and a value slot. The table allocates its arrays on the first insert.
 *
 * `yashy_map_insert` copies the key and the value. A `string` key is a handle:
 * the table retains it, so the map owns its own reference and the caller's
 * binding is untouched. Overwriting an existing key keeps the stored key (and
 * its reference) and replaces only the value. `yashy_map_remove` copies the
 * value out, releases a stored string key, and leaves a tombstone so probing
 * stays correct; `yashy_map_free` releases every string key and every array.
 *
 * `get` and `remove` return 1 with the value in `out`, or 0 and leave `out`
 * alone. The caller always zeroes `out` first, so the bytes behind a 0 are
 * defined even though they must not be read.
 *
 * `insert` reports nothing, like `push`: on a failed allocation the table is
 * left unchanged, which is the only state the caller could observe.
 *
 * `yashy_map_contains_key` answers 1 or 0 without copying the value out.
 * `yashy_map_keys` and `yashy_map_values` fill a fresh slice header, in the
 * order the table holds its entries -- which is the table's own order, not
 * the order the keys were inserted in. The key array owns a reference of its
 * own for every `string` key it copies, so the map can be freed without
 * pulling the strings out from under the array. The value array owns nothing:
 * a map value is always a number or a `bool`. */
typedef struct yashy_map yashy_map;
yashy_map *yashy_map_new(long long key_kind, long long key_size, long long value_size);
void yashy_map_free(yashy_map *map);
void yashy_map_insert(yashy_map *map, const void *key, const void *value);
long long yashy_map_get(const yashy_map *map, const void *key, void *out);
long long yashy_map_remove(yashy_map *map, const void *key, void *out);
long long yashy_map_len(const yashy_map *map);
long long yashy_map_contains_key(const yashy_map *map, const void *key);
void yashy_map_keys(yashy_slice *out, const yashy_map *map);
void yashy_map_values(yashy_slice *out, const yashy_map *map);

/* ---- Threads, mutexes and channels ------------------------------------
 *
 *     declare i32 @yashy_thread_spawn(i8*, i8*, i64)
 *     declare i32 @yashy_thread_join(i32)
 *     declare i8* @yashy_mutex_new()
 *     declare void @yashy_mutex_lock(i8*)
 *     declare void @yashy_mutex_unlock(i8*)
 *     declare void @yashy_mutex_free(i8*)
 *     declare i8* @yashy_chan_new(i64)
 *     declare i8 @yashy_chan_send(i8*, i8*, i64)
 *     declare i8 @yashy_chan_recv(i8*, i8*, i64)
 *     declare void @yashy_chan_close(i8*)
 *     declare void @yashy_chan_free(i8*)
 *
 * `yashy_thread_spawn` copies `arg_size` bytes of `arg` into storage the new
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
typedef struct yashy_mutex yashy_mutex;
typedef struct yashy_chan yashy_chan;

/* The C spelling of the entry point. Generated code passes the same pointer as
 * an `i8*`; a function pointer and an object pointer share one representation
 * on every target Yashy supports, so the two declarations are ABI-identical. */
typedef void (*yashy_thread_entry)(void *argument);
int yashy_thread_spawn(yashy_thread_entry entry, const void *arg, long long arg_size);
int yashy_thread_join(int handle);

yashy_mutex *yashy_mutex_new(void);
void yashy_mutex_lock(yashy_mutex *mutex);
void yashy_mutex_unlock(yashy_mutex *mutex);
void yashy_mutex_free(yashy_mutex *mutex);

yashy_chan *yashy_chan_new(long long capacity);
char yashy_chan_send(yashy_chan *chan, const void *item, long long size);
char yashy_chan_recv(yashy_chan *chan, void *out_item, long long size);
void yashy_chan_close(yashy_chan *chan);
void yashy_chan_free(yashy_chan *chan);

/* ---- Diagnostics ----
 *
 *     declare void @yashy_panic(i8*, i8*, i64, i64)
 *
 * Writes `<file>:<line>:<column>: panic: <message>` to standard error and ends
 * the process with a failure status. It never returns, so generated IR puts
 * `unreachable` after the call: nothing in a function that panics can be
 * reached.
 *
 * It does not unwind, so a `defer` body does not run before a panic yet. The
 * diagnostics section of runtime.c is where that limitation and its replacement
 * are written down. */
void yashy_panic(const yashy_str *message, const yashy_str *file, long long line, long long column);

/* ---- Exiting ----
 *
 *     declare void @yashy_exit(i64)
 *
 * Ends the process with `code` as its exit status, after flushing both streams.
 * This is what `os.exit(code)` lowers to; it never returns, so generated IR puts
 * `unreachable` after the call. An `i64` rather than the `i32` the operating
 * system reads because `i64` is the only integer Yashy has; the cast happens
 * here, once. */
void yashy_exit(long long code);

/* ---- Arithmetic faults ----
 *
 *     declare void @yashy_panic_overflow(i8*, i64, i64, i8*, i64, i64)
 *     declare void @yashy_panic_division(i8*, i64, i64, i8*, i64, i64)
 *
 * The two ways a checked integer operation stops a program: `op` is its symbol
 * (`+`, `-`, `*`, `/`, `%`), `lhs` and `rhs` are the operands, and the rest is
 * the position the checker resolved. Generated code branches here when the check
 * fails, so neither returns and both are followed by `unreachable`.
 *
 * `yashy_panic_division` names the fault from the operands: a zero divisor is
 * `division by zero`, and `INT64_MIN / -1` -- the only other division that has no
 * answer in an `i64` -- is an `integer overflow`.
 *
 * `-i64::MIN` reports through `yashy_panic_overflow` with `0` as the left operand
 * and `-` as the symbol, because that is what negation is. */
void yashy_panic_overflow(
    const yashy_str *op,
    long long lhs,
    long long rhs,
    const yashy_str *file,
    long long line,
    long long column
);
void yashy_panic_division(
    const yashy_str *op,
    long long lhs,
    long long rhs,
    const yashy_str *file,
    long long line,
    long long column
);

#ifdef __cplusplus
}
#endif

#endif /* YASHY_RUNTIME_H */
