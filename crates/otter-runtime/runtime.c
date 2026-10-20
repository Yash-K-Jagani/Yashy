#include "otter_runtime.h"

#include <ctype.h>
#include <errno.h>
#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if defined(_WIN32)
#include <fcntl.h>
#include <io.h>
#endif

/* `otter_str_data`, defined with the string type further down, gives the bytes
 * of a string to the C functions that want a `const char*`. Declared here
 * because the printing and diagnostic functions come before it. */
static const char *otter_str_data(const otter_str *s);

/* Thread-local storage, which the runtime uses wherever a `static` would
 * otherwise be shared mutable state:
 *
 * - the string arena, because a bump allocator has no lock and two threads
 *   taking the same block is a data race (see the string section);
 * - the stdout-mode flag below, because the switch it guards is idempotent and
 *   repeating it per thread is cheaper than sharing a flag. */

#if defined(_MSC_VER)
/* MSVC (and clang with the MSVC target) spell thread-local storage this way. */
#define OTTER_THREAD_LOCAL __declspec(thread)
#else
#define OTTER_THREAD_LOCAL __thread
#endif

/* Otter strings are byte strings, so `print_str("\n")` has to write exactly one
 * 0x0A byte. Windows opens stdout in text mode, where the C library rewrites
 * every 0x0A as 0x0D 0x0A before it reaches the operating system -- the byte the
 * program asked for is not the byte that comes out, and a program writing
 * anything other than text comes out corrupted. Switching stdout to binary mode
 * leaves the choice of line ending to the program, which is where it belongs.
 *
 * The switch happens once, before the first write. `_setmode` is idempotent, so
 * the flag is only there to keep it off the hot path -- and it is thread-local
 * rather than shared, because an unsynchronized `static` written by two threads
 * is a data race even when the work it guards is harmless to repeat. */
static void otter_stdout_binary(void) {
#if defined(_WIN32)
    static OTTER_THREAD_LOCAL int already_switched = 0;
    if (!already_switched) {
        already_switched = 1;
        (void)_setmode(_fileno(stdout), _O_BINARY);
    }
#endif
}

void otter_print_str(const otter_str *s) {
    if (s == NULL) {
        return;
    }
    otter_stdout_binary();
    fputs(otter_str_data(s), stdout);
}

void otter_print_i64(long long value) {
    otter_stdout_binary();
    printf("%lld", value);
}

void otter_print_f64(double value) {
    otter_stdout_binary();
    printf("%.17g", value);
}

/* ---- Live-allocation accounting --------------------------------------
 *
 * The two surfaces the compiler inserts ownership for are manual `alloc` /
 * `dealloc` and refcounted strings. Both go through `malloc`/`free` in this
 * file, so the runtime can count what is still live without a malloc
 * interposer or a debug CRT -- and that is what makes the leak tests runnable
 * on any machine the compiler itself runs on.
 *
 * Set `OTTER_LEAK_REPORT` to anything other than an empty string or `0` and
 * the runtime prints one line to stderr at process exit:
 *
 *     otter: leak report: 0 live allocations
 *
 * Exit is what makes the report cover the paths that matter: returning from
 * `main`, `otter_exit`, and the `exit` a panic takes all run `atexit`
 * handlers post-`main`, which is the only place "the program is over" is
 * known.
 *
 * Only whole allocations are counted, not bytes. A leak test asks "did every
 * block come back?", and a count answers exactly that; a byte total would
 * need the requested size stored beside every block, which is a change to the
 * allocator for a number nothing asserts. The arena's blocks are deliberately
 * not counted: an arena is released all at once by `arena_free_all`, and a
 * program that never calls it leaks by contract rather than by accident.
 *
 * The counters are updated with the platform's atomic add because
 * `otter_alloc` and strings are safe to use from any thread; the report
 * itself runs once, at exit, on the thread that called `exit`. */

static volatile long long otter_live_allocations = 0;
static int otter_leak_report_armed = 0;

#if defined(_MSC_VER)
#include <intrin.h>
static void otter_count_add(volatile long long *counter, long long delta) {
    (void)_InterlockedExchangeAdd64(counter, delta);
}
#else
static void otter_count_add(volatile long long *counter, long long delta) {
    (void)__sync_add_and_fetch(counter, delta);
}
#endif

static void otter_leak_report_at_exit(void) {
    fprintf(stderr, "otter: leak report: %lld live allocations\n", otter_live_allocations);
    fflush(stderr);
}

/* Registers the report the first time anything countable is allocated. Doing it
 * lazily rather than from a constructor keeps the C side free of per-compiler
 * startup attributes; a program that allocates nothing has nothing to report.
 * The first allocation in a program is on its main thread in every case the
 * language can express today, so the unguarded flag is not a race worth a lock
 * on the allocation path. */
static void otter_leak_arm(void) {
    if (otter_leak_report_armed) {
        return;
    }
    otter_leak_report_armed = 1;
    const char *requested = getenv("OTTER_LEAK_REPORT");
    if (requested != NULL && requested[0] != '\0' && requested[0] != '0') {
        (void)atexit(otter_leak_report_at_exit);
    }
}

/* ---- Heap allocation -------------------------------------------------
 *
 * `otter_alloc` over-allocates by one pointer plus up to `align - 1` bytes so
 * it can hand back a correctly aligned address on any platform, and it stashes
 * the address `malloc` returned immediately in front of the aligned block.
 * `otter_free` walks back to that stash. The alternative -- `aligned_alloc` on
 * POSIX and `_aligned_malloc`/`_aligned_free` on Windows -- would make the two
 * functions a matched pair that cannot be told apart from `malloc`/`free`, and
 * a program that mixed them would corrupt the heap instead of leaking. One
 * allocator, one free path, wrong in neither direction.
 *
 * The memory is deliberately not zeroed: Otter has no zero-fill guarantee, and
 * a program that wants zeroes can say so. */

/* Rounds a requested alignment to something usable: a power of two no smaller
 * than a pointer. A zero, a negative, or a non-power-of-two is treated as
 * "pointer alignment" rather than trusted, so a bad constant cannot make the
 * allocator hand back a misaligned block. */
static size_t otter_normalise_align(long long align) {
    size_t result = align > 0 ? (size_t)align : sizeof(void *);
    if (result < sizeof(void *)) {
        result = sizeof(void *);
    }
    if ((result & (result - 1)) != 0) {
        result = 16;
    }
    return result;
}

void *otter_alloc(long long size, long long align) {
    otter_leak_arm();
    size_t alignment = otter_normalise_align(align);
    size_t wanted = size > 0 ? (size_t)size : 1;
    size_t total = wanted + alignment + sizeof(void *);
    /* The addition can wrap; a request that large is a failure, not a small
     * allocation. */
    if (total < wanted) {
        return NULL;
    }
    unsigned char *raw = (unsigned char *)malloc(total);
    if (raw == NULL) {
        return NULL;
    }
    uintptr_t base = (uintptr_t)raw + sizeof(void *);
    uintptr_t aligned = (base + (alignment - 1)) & ~(uintptr_t)(alignment - 1);
    ((void **)aligned)[-1] = raw;
    otter_count_add(&otter_live_allocations, 1);
    return (void *)aligned;
}

void otter_free(void *ptr) {
    if (ptr == NULL) {
        return;
    }
    free(((void **)ptr)[-1]);
    otter_count_add(&otter_live_allocations, -1);
}

/* ---- Arenas ----------------------------------------------------------
 *
 * An arena is a bump allocator over a linked list of blocks. Allocation is a
 * pointer increment; the only way to release is `otter_arena_free_all`, which
 * walks the list. There is no per-allocation free, because an arena exists to
 * make that impossible to get wrong.
 *
 * An arena is **not** safe to use from two threads at once: `head` and `used`
 * are written with no synchronization, which keeps allocation to a pointer
 * increment. A caller that shares one arena across threads must serialize the
 * calls itself. The runtime's own string arena does not share one -- see below --
 * and the generated `alloc`/`dealloc` go to `malloc`/`free`, which are
 * thread-safe. */

/* The block header is padded to this many bytes so the payload that follows it
 * starts on a 16-byte boundary -- enough for the widest scalar and for a
 * 128-bit vector load, which the backend may emit. */
#define OTTER_ARENA_HEADER 32u

/* A fresh block is at least this large, so a program that allocates in small
 * pieces still allocates memory in page-sized pieces. */
#define OTTER_ARENA_BLOCK_SIZE (64u * 1024u)

struct otter_arena_block {
    struct otter_arena_block *next;
    size_t capacity;
    size_t used;
};

struct otter_arena {
    struct otter_arena_block *head;
};

static size_t otter_align_up(size_t value, size_t align) {
    return (value + align - 1) & ~(align - 1);
}

otter_arena *otter_arena_new(void) {
    struct otter_arena *arena = (struct otter_arena *)malloc(sizeof(struct otter_arena));
    if (arena == NULL) {
        return NULL;
    }
    arena->head = NULL;
    return arena;
}

static struct otter_arena_block *otter_arena_block_new(size_t capacity) {
    size_t total = OTTER_ARENA_HEADER + capacity;
    if (total < capacity) {
        return NULL;
    }
    struct otter_arena_block *block = (struct otter_arena_block *)malloc(total);
    if (block == NULL) {
        return NULL;
    }
    block->next = NULL;
    block->capacity = capacity;
    block->used = 0;
    return block;
}

void *otter_arena_alloc(otter_arena *arena, long long size, long long align) {
    if (arena == NULL) {
        return NULL;
    }
    size_t alignment = otter_normalise_align(align);
    size_t wanted = size > 0 ? otter_align_up((size_t)size, alignment) : alignment;
    if (wanted < (size_t)size) {
        return NULL;
    }

    struct otter_arena_block *block = arena->head;
    if (block != NULL) {
        uintptr_t base = (uintptr_t)block + OTTER_ARENA_HEADER;
        uintptr_t cursor = (base + block->used + (alignment - 1)) & ~(uintptr_t)(alignment - 1);
        size_t offset = (size_t)(cursor - base);
        if (offset <= block->capacity && wanted <= block->capacity - offset) {
            block->used = offset + wanted;
            return (void *)cursor;
        }
    }

    /* No room in the current block: link a fresh one. It is sized to hold this
     * allocation even when the request exceeds the default block size, plus the
     * slack the alignment adjustment might spend. */
    size_t capacity = OTTER_ARENA_BLOCK_SIZE;
    if (wanted > SIZE_MAX - alignment) {
        return NULL;
    }
    if (capacity < wanted + alignment) {
        capacity = wanted + alignment;
    }
    struct otter_arena_block *fresh = otter_arena_block_new(capacity);
    if (fresh == NULL) {
        return NULL;
    }
    fresh->next = arena->head;
    arena->head = fresh;

    uintptr_t base = (uintptr_t)fresh + OTTER_ARENA_HEADER;
    uintptr_t cursor = (base + (alignment - 1)) & ~(uintptr_t)(alignment - 1);
    fresh->used = (size_t)(cursor - base) + wanted;
    return (void *)cursor;
}

void otter_arena_free_all(otter_arena *arena) {
    if (arena == NULL) {
        return;
    }
    struct otter_arena_block *block = arena->head;
    while (block != NULL) {
        struct otter_arena_block *next = block->next;
        free(block);
        block = next;
    }
    arena->head = NULL;
}

/* ---- Strings -----------------------------------------------------------
 *
 * A `string` value in generated code is an `i8*` pointing at an `otter_str`:
 *
 *     %otter_str = type { i64, i64, i64, i8* }   ; refcount, len, cap, data
 *
 * The refcount is 1 when a string is created and copied, decremented when a
 * reference is dropped, and the string is freed when it reaches zero. A string
 * with refcount -1 is *static*: a literal baked into the binary, never freed.
 * Generated code writes that -1; the runtime only ever *creates* owned
 * strings, from concatenation, the numeric conversions, and input.
 *
 * The operations that produce a string own the result -- it comes back with
 * refcount 1 and the caller is expected to drop it when finished. Nothing in
 * generated code drops one yet: automatic release, which pairs a retain with
 * each binding and a release with each scope, is the second half of this task
 * and lands on top of this representation. Until then a built string is held
 * until the process exits, which is a leak and not a corruption. */

struct otter_str {
    long long refcount;   /* 1 = owned, -1 = static (never freed) */
    long long len;        /* number of bytes, excluding NUL */
    long long cap;        /* allocated capacity, excluding NUL */
    char *data;           /* NUL-terminated byte buffer */
};

/* Creates a new owned string from `len` bytes at `bytes`. */
static otter_str *otter_str_new(const char *bytes, size_t len) {
    otter_leak_arm();
    otter_str *s = (otter_str *)malloc(sizeof(otter_str));
    if (s == NULL) {
        return NULL;
    }
    s->refcount = 1;
    s->len = (long long)len;
    s->cap = (long long)len;
    s->data = (char *)malloc(len + 1);
    if (s->data == NULL) {
        free(s);
        return NULL;
    }
    if (len > 0 && bytes != NULL) {
        memcpy(s->data, bytes, len);
    }
    s->data[len] = '\0';
    otter_count_add(&otter_live_allocations, 1);
    return s;
}

/* Returns the data pointer for passing to C functions that expect `const char*`. */
static const char *otter_str_data(const otter_str *s) {
    return s == NULL ? "" : s->data;
}

/* A fresh, NUL-terminated copy for a safe C call. The copy is made with
 * `otter_alloc`, so it obeys the same alignment and accounting rules as every
 * other heap value and the caller frees it with `otter_free`. C may keep this
 * pointer for as long as it likes; it owns nothing of Otter's. */
char *otter_str_cstr(const otter_str *s) {
    size_t len = s == NULL ? 0 : (size_t)s->len;
    char *copy = (char *)otter_alloc((long long)(len + 1), 1);
    if (copy == NULL) {
        return NULL;
    }
    if (len > 0) {
        memcpy(copy, s->data, len);
    }
    copy[len] = '\0';
    return copy;
}

/* The string's own bytes, for an `unsafe` call. The pointer is valid for as
 * long as the string is alive, which is exactly the promise `unsafe` waives. */
const char *otter_str_ptr(const otter_str *s) {
    return otter_str_data(s);
}

/* Keeps a reference to `s`: the refcount goes up by one. A static string
 * (refcount -1) is untouched, because it is never freed and has no count to
 * keep. Returns `s` so a caller can write `x = otter_str_retain(x)`. */
otter_str *otter_str_retain(otter_str *s) {
    if (s != NULL && s->refcount > 0) {
        s->refcount++;
    }
    return s;
}

/* Drops a reference to `s`. The count goes down by one, and when it reaches
 * zero the bytes and the header are freed. Static strings (refcount -1) and
 * NULL are left alone. */
void otter_str_release(otter_str *s) {
    if (s == NULL || s->refcount <= 0) {
        return;
    }
    s->refcount--;
    if (s->refcount == 0) {
        free(s->data);
        free(s);
        otter_count_add(&otter_live_allocations, -1);
    }
}

long long otter_str_len(const otter_str *s) {
    return s == NULL ? 0 : s->len;
}

long long otter_str_byte_at(const otter_str *s, long long index) {
    if (s == NULL || index < 0 || index >= s->len) {
        return 0;
    }
    return (long long)(unsigned char)s->data[index];
}

otter_str *otter_str_concat(const otter_str *a, const otter_str *b) {
    size_t len_a = a == NULL ? 0 : (size_t)a->len;
    size_t len_b = b == NULL ? 0 : (size_t)b->len;
    otter_str *out = otter_str_new(NULL, len_a + len_b);
    if (out == NULL) {
        return NULL;
    }
    if (len_a > 0) {
        memcpy(out->data, a->data, len_a);
    }
    if (len_b > 0) {
        memcpy(out->data + len_a, b->data, len_b);
    }
    return out;
}

int otter_str_eq(const otter_str *a, const otter_str *b) {
    if (a == NULL || b == NULL) {
        return a == b;
    }
    if (a->len != b->len) {
        return 0;
    }
    return a->len == 0 || memcmp(a->data, b->data, (size_t)a->len) == 0;
}

unsigned long long otter_str_hash(const otter_str *s) {
    /* FNV-1a over the bytes. The map runtime calls this through the header;
     * generated code never does. A null string hashes like the empty one, which
     * makes a map lookup with a missing string a miss rather than a crash. */
    unsigned long long hash = 14695981039346656037ULL;
    if (s == NULL || s->data == NULL) {
        return hash;
    }
    for (long long i = 0; i < s->len; i++) {
        hash ^= (unsigned char)s->data[i];
        hash *= 1099511628211ULL;
    }
    return hash;
}

otter_str *otter_str_from_i64(long long value) {
    char buffer[32];
    int written = snprintf(buffer, sizeof(buffer), "%lld", value);
    if (written < 0 || (size_t)written >= sizeof(buffer)) {
        return NULL;
    }
    return otter_str_new(buffer, (size_t)written);
}

otter_str *otter_str_from_f64(double value) {
    char buffer[40];
    int written = snprintf(buffer, sizeof(buffer), "%.17g", value);
    if (written < 0 || (size_t)written >= sizeof(buffer)) {
        return NULL;
    }
    return otter_str_new(buffer, (size_t)written);
}

otter_str *otter_str_from_bool(long long value) {
    const char *text = value != 0 ? "true" : "false";
    return otter_str_new(text, value != 0 ? 4u : 5u);
}

/* ---- Input ------------------------------------------------------------
 *
 * `otter_read_line` reads one line from standard input and returns it without
 * its line terminator, as a refcounted string (owned by the caller). At end of
 * input it returns the empty string: the language has no way to spell "no line"
 * yet, and an empty line and the end of input reading the same is a limitation,
 * not a choice -- a checked form needs `panic` or an `Option` to be reachable
 * from a runtime call, and M4 is where those arrive.
 *
 * A trailing 0x0D is stripped along with the 0x0A. On Windows the C library
 * folds CRLF to LF before `fgetc` sees it; this is for a CRLF that arrives from
 * somewhere else -- a pipe, or the same program on Linux.
 *
 * The parsers return zero when the text is not an integer (or number), and the
 * check is over the *whole* string: `parse_i64("12x")` is zero, not twelve.
 * Reading the leading digits and ignoring the rest is what `strtoll` does on its
 * own, and it turns a malformed line into a plausible-looking wrong number --
 * the failure a user is least likely to notice. Overflow folds into the same
 * zero rather than saturating, so the contract is exactly "the integer `s`
 * denotes, or zero", with nothing in between. It is still a real answer and a
 * wrong answer both; M4's checked variant replaces it once `panic` exists. */

otter_str *otter_read_line(void) {
    size_t capacity = 128;
    size_t length = 0;
    char *buffer = (char *)malloc(capacity);
    if (buffer == NULL) {
        return otter_str_new("", 0);
    }

    int byte;
    while ((byte = fgetc(stdin)) != EOF && byte != '\n') {
        if (length + 1 >= capacity) {
            size_t grown = capacity * 2;
            char *moved = (char *)realloc(buffer, grown);
            if (moved == NULL) {
                /* Return what was read rather than losing the line: the buffer
                 * still holds `length` valid bytes. */
                break;
            }
            buffer = moved;
            capacity = grown;
        }
        buffer[length++] = (char)byte;
    }

    if (length > 0 && buffer[length - 1] == '\r') {
        length -= 1;
    }
    otter_str *line = otter_str_new(buffer, length);
    free(buffer);
    return line;
}

/* Whether only whitespace is left in `s` -- the second half of "the whole
 * string is a number". `strtoll` stops at the first character it cannot read, so
 * the position it stopped at is the only evidence that anything was left over. */
static int otter_only_space_remains(const char *s) {
    while (*s != '\0') {
        if (!isspace((unsigned char)*s)) {
            return 0;
        }
        s += 1;
    }
    return 1;
}

long long otter_parse_i64(const otter_str *s) {
    const char *text = otter_str_data(s);
    errno = 0;
    char *end = NULL;
    long long value = strtoll(text, &end, 10);
    if (end == text || errno == ERANGE) {
        return 0;
    }
    if (!otter_only_space_remains(end)) {
        return 0;
    }
    return value;
}

double otter_parse_f64(const otter_str *s) {
    const char *text = otter_str_data(s);
    errno = 0;
    char *end = NULL;
    double value = strtod(text, &end);
    if (end == text || errno == ERANGE) {
        return 0.0;
    }
    if (!otter_only_space_remains(end)) {
        return 0.0;
    }
    return value;
}

/* ---- Slices ----
 *
 * See the header for the contract. Two decisions are worth spelling out here:
 *
 * - Growth doubles from a capacity of four. Starting at four avoids a malloc
 *   per element for the common one-at-a-time build, and doubling keeps the
 *   total copy work linear.
 * - A failed grow leaves the slice exactly as it was. There is no error channel
 *   in the ABI, and faking one by aborting would make an allocation failure
 *   look like a language fault; the caller sees the element simply not arrive.
 */
void otter_slice_push(otter_slice *slice, const void *element, long long element_size) {
    if (slice == NULL || element == NULL || element_size <= 0) {
        return;
    }
    if (slice->len >= slice->cap) {
        long long cap = slice->cap > 0 ? slice->cap * 2 : 4;
        if (cap > LLONG_MAX / element_size) {
            return;
        }
        char *data = (char *)otter_alloc(cap * element_size, 8);
        if (data == NULL) {
            return;
        }
        if (slice->len > 0 && slice->data != NULL) {
            memcpy(data, slice->data, (size_t)(slice->len * element_size));
        }
        otter_free(slice->data);
        slice->data = data;
        slice->cap = cap;
    }
    memcpy((char *)slice->data + (size_t)(slice->len * element_size), element,
           (size_t)element_size);
    slice->len += 1;
}

void otter_slice_free(otter_slice *slice) {
    if (slice == NULL) {
        return;
    }
    otter_free(slice->data);
    slice->data = NULL;
    slice->len = 0;
    slice->cap = 0;
}

/* Releases every element of a `[]string`, then the array itself.
 *
 * A slice of strings owns one reference per element: `push` and the literal
 * retain each handle, and this is the only place that drops them. A static
 * string has a zero refcount, so `otter_str_release` leaves it alone and
 * `[]string { "a", "b" }` costs nothing to free. The header is zeroed exactly
 * as `otter_slice_free` does, so a second free through a stale header is a
 * no-op rather than a double free. */
void otter_slice_free_strings(otter_slice *slice) {
    if (slice == NULL) {
        return;
    }
    if (slice->data != NULL) {
        otter_str **elements = (otter_str **)slice->data;
        for (long long index = 0; index < slice->len; index++) {
            otter_str_release(elements[index]);
        }
    }
    otter_free(slice->data);
    slice->data = NULL;
    slice->len = 0;
    slice->cap = 0;
}

/* ---- Slice access ------------------------------------------------------
 *
 *     declare i64 @otter_slice_get(ptr, i64, ptr, i64)
 *     declare i64 @otter_slice_pop(ptr, ptr, i64)
 *
 * Both answer 0 or 1 and copy the element into `out` only on 1. `get` tests
 * the index against `len` *signed*: a negative index is a miss, not the same
 * address as `len + index`, which is the wraparound a C program would read by
 * accident. `pop` decrements first, so the value it copies is the one that
 * just left the slice and a second call cannot see it again.
 *
 * The runtime never traps and never writes on a miss. Generated code turns the
 * flag into an `Option` tag, so "out of range" is a value the program can
 * match, and the payload slot it loads from holds a zero the caller stored.
 *
 * A null slice, a null `out`, an empty slice and a non-positive stride are all
 * the same miss. `slice->data` is null exactly when nothing was ever pushed,
 * which `len == 0` already covers, but the check keeps the pointer arithmetic
 * below free of a null base. */

long long otter_slice_get(otter_slice *slice, long long index, void *out,
                          long long element_size) {
    if (slice == NULL || out == NULL || element_size <= 0 || index < 0 ||
        index >= slice->len || slice->data == NULL) {
        return 0;
    }
    memcpy(out, (const char *)slice->data + (size_t)(index * element_size),
           (size_t)element_size);
    return 1;
}

long long otter_slice_pop(otter_slice *slice, void *out, long long element_size) {
    if (slice == NULL || out == NULL || element_size <= 0 || slice->len <= 0 ||
        slice->data == NULL) {
        return 0;
    }
    slice->len -= 1;
    memcpy(out, (const char *)slice->data + (size_t)(slice->len * element_size),
           (size_t)element_size);
    return 1;
}

/* ---- Diagnostics -------------------------------------------------------
 *
 *     declare void @otter_panic(i8*, i8*, i64, i64)
 *
 * `otter_panic` is where a program stops. It writes the position the panic was
 * written at and the message to standard error, then ends the process with a
 * failure status. It never returns, so generated code puts `unreachable` after
 * every call.
 *
 * It does **not** unwind. The generated code runs the `defer` bodies registered
 * in the *panicking function's* open scopes before it calls this, so that
 * function's cleanup happens; a `defer` registered in a caller does not run,
 * because there are no landing pads yet. That is M4's first half: the plan's
 * next task replaces this body with a throw that generated landing pads catch,
 * and the deferred calls move onto that path. Until then the process dies. The
 * limitation is stated rather than hidden, and dying is the honest reading of
 * the alternative -- before `panic` existed a program with an unrecoverable
 * condition printed a wrong answer and exited zero, which is worse than either.
 *
 * `stdout` is flushed first, so a program that printed before it panicked does
 * not lose that output to a buffer that would never be flushed. `exit` is used
 * rather than `abort` for the same reason: it flushes both streams, and a caller
 * reading the exit status still sees the failure.
 *
 * `file` is the path the compiler was handed and may be relative; `line` and
 * `column` are one-based and count characters, like every other diagnostic. */
void otter_panic(const otter_str *message, const otter_str *file, long long line, long long column) {
    fflush(stdout);
    fprintf(
        stderr,
        "%s:%lld:%lld: panic: %s\n",
        file != NULL ? file->data : "<unknown>",
        line,
        column,
        otter_str_data(message)
    );
    fflush(stderr);
    exit(1);
}

/* ---- Exiting -----------------------------------------------------------
 *
 *     declare void @otter_exit(i64)
 *
 * `os.exit(code)` ends the process with `code` as its exit status, which is the
 * one thing a program could not do for itself: `main`'s return value is the exit
 * status, but only for a program that reaches the end of `main` -- and an
 * `os.exit` in the middle of a helper would have to be threaded back through
 * every caller to have the same effect.
 *
 * It takes an `i64` rather than the `i32` the operating system reads, because
 * `i64` is the only integer type Otter has; the cast is here, once, instead of at
 * every call site. Both streams are flushed first for the same reason
 * `otter_panic` flushes: output written before the exit must not be lost to a
 * buffer that never gets flushed. */
void otter_exit(long long code) {
    fflush(stdout);
    fflush(stderr);
    exit((int)code);
}

/* ---- Arithmetic faults -------------------------------------------------
 *
 *     declare void @otter_panic_overflow(i8*, i64, i64, i8*, i64, i64)
 *     declare void @otter_panic_division(i8*, i64, i64, i8*, i64, i64)
 *
 * The two ways an integer operation stops a program. Integer arithmetic is
 * checked (spec 6.1): `a + b` that overflows, `a / 0`, `a % 0`, and negating the
 * smallest `i64` are faults rather than a wrapped result, a hardware trap, or
 * undefined behaviour. Generated code emits the comparison and the branch and
 * calls one of these from the block that is never taken on the happy path, so
 * what arrives here is the *operation* that failed: its symbol, its two
 * operands, and the position the checker resolved for it.
 *
 * The operands are passed rather than described, because the numbers are what
 * make the message worth printing:
 *
 *     main.ot:7:13: panic: integer overflow: 9223372036854775807 + 1
 *     main.ot:9:9: panic: division by zero: 5 / 0
 *
 * `otter_panic_division` decides which of its two cases it has from the operands
 * instead of being told: a zero divisor is division by zero, and the only other
 * way a division fails is `INT64_MIN / -1`, whose answer is one past `INT64_MAX`.
 * Generated code covers both with one branch, and one branch is one instruction
 * closer to not being there at all -- so the cheaper split is the comparison
 * here, on the cold path, rather than the second test on the hot one.
 *
 * Neither unwinds; both end the process, exactly as `otter_panic` does and for
 * the same reasons (`stdout` flushed first, `exit` rather than `abort`). The
 * same statement about the panicking function's own `defer` bodies applies to
 * all three: they run before the runtime call, and a caller's do not. */

static void otter_fault(
    const char *what,
    const otter_str *op,
    long long lhs,
    long long rhs,
    const otter_str *file,
    long long line,
    long long column
) {
    fflush(stdout);
    fprintf(
        stderr,
        "%s:%lld:%lld: panic: %s: %lld %s %lld\n",
        file != NULL ? file->data : "<unknown>",
        line,
        column,
        what,
        lhs,
        otter_str_data(op),
        rhs
    );
    fflush(stderr);
    exit(1);
}

void otter_panic_overflow(
    const otter_str *op,
    long long lhs,
    long long rhs,
    const otter_str *file,
    long long line,
    long long column
) {
    otter_fault("integer overflow", op, lhs, rhs, file, line, column);
}

void otter_panic_division(
    const otter_str *op,
    long long lhs,
    long long rhs,
    const otter_str *file,
    long long line,
    long long column
) {
    /* A negative divisor of -1 with the smallest `i64` is the overflow case; a
     * zero divisor is the other. `srem` is undefined for both, so both arrive
     * here. */
    otter_fault(
        rhs == 0 ? "division by zero" : "integer overflow",
        op,
        lhs,
        rhs,
        file,
        line,
        column
    );
}
