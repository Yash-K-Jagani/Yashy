#include "otter_runtime.h"

#include <ctype.h>
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if defined(_WIN32)
#include <fcntl.h>
#include <io.h>
#endif

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

void otter_print_str(const char *s) {
    if (s == NULL) {
        return;
    }
    otter_stdout_binary();
    fputs(s, stdout);
}

void otter_print_i64(long long value) {
    otter_stdout_binary();
    printf("%lld", value);
}

void otter_print_f64(double value) {
    otter_stdout_binary();
    printf("%.17g", value);
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
    return (void *)aligned;
}

void otter_free(void *ptr) {
    if (ptr == NULL) {
        return;
    }
    free(((void **)ptr)[-1]);
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
 * A `string` value in generated code is an `i8*`: a pointer to a
 * NUL-terminated byte sequence (M0-M2's representation; ledger L4 records the
 * interim). The operations that *produce* a string -- concatenation and the
 * conversions -- allocate from one internal arena, so building strings in a
 * loop does not pay a `malloc` per call. Nothing built here is ever freed
 * individually, because a built string has no owner to release: the value is a
 * bare `i8*` with no refcount and no `release` operation in the language, so
 * there is no way for a program to say it is finished with one (ledger L1's M3
 * note, ledger L4). Holding it until the process exits is the correct reading of
 * that -- and it is a leak, not a corruption, which is why it is acceptable
 * while the refcounted representation is still pending.
 *
 * The internal arena is created on first use and never destroyed. A program
 * that never builds a string never allocates one.
 *
 * It is **thread-local**, one arena per thread, and that is a correctness
 * requirement rather than an optimization. `otter_string_arena` used to be a
 * single shared arena published through a plain `static` pointer, so two threads
 * building strings at once could both take the same block -- the arena's bump
 * pointer is unsynchronized -- and the symptom was one string's bytes appearing
 * where another's should be. The test suite started failing about one run in
 * five as soon as enough parallel tests built strings. Per-thread state removes
 * the race instead of guarding it, and costs no lock on the path to a string.
 *
 * A string built on one thread and read on another is still valid: the bytes
 * live in the building thread's arena, which is never freed. The cost is one
 * arena per thread that builds strings, each holding up to a block until the
 * process exits -- the same "held until exit" as before, multiplied by the
 * number of threads. */

static OTTER_THREAD_LOCAL otter_arena *otter_string_arena_slot = NULL;

static otter_arena *otter_string_arena(void) {
    if (otter_string_arena_slot == NULL) {
        otter_string_arena_slot = otter_arena_new();
    }
    return otter_string_arena_slot;
}

/* Copies `len` bytes out of `bytes` and NUL-terminates the copy, in the string
 * arena. Returns NULL only when the arena cannot grow. */
static const char *otter_str_copy_into_arena(const char *bytes, size_t len) {
    char *out = (char *)otter_arena_alloc(otter_string_arena(), (long long)(len + 1), 1);
    if (out == NULL) {
        return NULL;
    }
    if (len > 0 && bytes != NULL) {
        memcpy(out, bytes, len);
    }
    out[len] = '\0';
    return out;
}

long long otter_str_len(const char *s) {
    return s == NULL ? 0 : (long long)strlen(s);
}

long long otter_str_byte_at(const char *s, long long index) {
    if (s == NULL || index < 0) {
        return 0;
    }
    size_t len = strlen(s);
    if ((size_t)index >= len) {
        return 0;
    }
    return (long long)(unsigned char)s[index];
}

const char *otter_str_concat(const char *a, const char *b) {
    size_t len_a = a == NULL ? 0 : strlen(a);
    size_t len_b = b == NULL ? 0 : strlen(b);
    char *out = (char *)otter_arena_alloc(otter_string_arena(), (long long)(len_a + len_b + 1), 1);
    if (out == NULL) {
        return NULL;
    }
    if (len_a > 0) {
        memcpy(out, a, len_a);
    }
    if (len_b > 0) {
        memcpy(out + len_a, b, len_b);
    }
    out[len_a + len_b] = '\0';
    return out;
}

int otter_str_eq(const char *a, const char *b) {
    if (a == NULL || b == NULL) {
        return a == b;
    }
    return strcmp(a, b) == 0;
}

const char *otter_str_from_i64(long long value) {
    char buffer[32];
    int written = snprintf(buffer, sizeof(buffer), "%lld", value);
    if (written < 0 || (size_t)written >= sizeof(buffer)) {
        return NULL;
    }
    return otter_str_copy_into_arena(buffer, (size_t)written);
}

const char *otter_str_from_f64(double value) {
    char buffer[40];
    int written = snprintf(buffer, sizeof(buffer), "%.17g", value);
    if (written < 0 || (size_t)written >= sizeof(buffer)) {
        return NULL;
    }
    return otter_str_copy_into_arena(buffer, (size_t)written);
}

const char *otter_str_from_bool(long long value) {
    const char *text = value != 0 ? "true" : "false";
    return otter_str_copy_into_arena(text, value != 0 ? 4u : 5u);
}

/* ---- Input ------------------------------------------------------------
 *
 * `otter_read_line` reads one line from standard input and returns it without
 * its line terminator, built in the string arena like every other produced
 * string (so it is never individually freed). At end of input it returns the
 * empty string: the language has no way to spell "no line" yet, and an empty
 * line and the end of input reading the same is a limitation, not a choice --
 * a checked form needs `panic` or an `Option` to be reachable from a runtime
 * call, and M4 is where those arrive.
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

const char *otter_read_line(void) {
    size_t capacity = 128;
    size_t length = 0;
    char *buffer = (char *)malloc(capacity);
    if (buffer == NULL) {
        return otter_str_copy_into_arena("", 0);
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
    const char *line = otter_str_copy_into_arena(buffer, length);
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

long long otter_parse_i64(const char *s) {
    if (s == NULL) {
        return 0;
    }
    errno = 0;
    char *end = NULL;
    long long value = strtoll(s, &end, 10);
    if (end == s || errno == ERANGE) {
        return 0;
    }
    if (!otter_only_space_remains(end)) {
        return 0;
    }
    return value;
}

double otter_parse_f64(const char *s) {
    if (s == NULL) {
        return 0.0;
    }
    errno = 0;
    char *end = NULL;
    double value = strtod(s, &end);
    if (end == s || errno == ERANGE) {
        return 0.0;
    }
    if (!otter_only_space_remains(end)) {
        return 0.0;
    }
    return value;
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
 * It does **not** unwind, and so a `defer` body does **not** run before a panic.
 * This is M4's first half: the plan's next task replaces this body with a throw
 * that generated landing pads catch, and the deferred calls move onto that path.
 * Until then the process dies. The limitation is stated rather than hidden, and
 * dying is the honest reading of the alternative -- before `panic` existed a
 * program with an unrecoverable condition printed a wrong answer and exited
 * zero, which is worse than either.
 *
 * `stdout` is flushed first, so a program that printed before it panicked does
 * not lose that output to a buffer that would never be flushed. `exit` is used
 * rather than `abort` for the same reason: it flushes both streams, and a caller
 * reading the exit status still sees the failure.
 *
 * `file` is the path the compiler was handed and may be relative; `line` and
 * `column` are one-based and count characters, like every other diagnostic. */
void otter_panic(const char *message, const char *file, long long line, long long column) {
    fflush(stdout);
    fprintf(
        stderr,
        "%s:%lld:%lld: panic: %s\n",
        file != NULL ? file : "<unknown>",
        line,
        column,
        message != NULL ? message : ""
    );
    fflush(stderr);
    exit(1);
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
 * statement that a `defer` body does not run before a fault is about all three. */

static void otter_fault(
    const char *what,
    const char *op,
    long long lhs,
    long long rhs,
    const char *file,
    long long line,
    long long column
) {
    fflush(stdout);
    fprintf(
        stderr,
        "%s:%lld:%lld: panic: %s: %lld %s %lld\n",
        file != NULL ? file : "<unknown>",
        line,
        column,
        what,
        lhs,
        op != NULL ? op : "?",
        rhs
    );
    fflush(stderr);
    exit(1);
}

void otter_panic_overflow(
    const char *op,
    long long lhs,
    long long rhs,
    const char *file,
    long long line,
    long long column
) {
    otter_fault("integer overflow", op, lhs, rhs, file, line, column);
}

void otter_panic_division(
    const char *op,
    long long lhs,
    long long rhs,
    const char *file,
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
