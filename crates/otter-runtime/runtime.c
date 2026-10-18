#include "otter_runtime.h"

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if defined(_WIN32)
#include <fcntl.h>
#include <io.h>
#endif

/* Otter strings are byte strings, so `print_str("\n")` has to write exactly one
 * 0x0A byte. Windows opens stdout in text mode, where the C library rewrites
 * every 0x0A as 0x0D 0x0A before it reaches the operating system -- the byte the
 * program asked for is not the byte that comes out, and a program writing
 * anything other than text comes out corrupted. Switching stdout to binary mode
 * leaves the choice of line ending to the program, which is where it belongs.
 *
 * The switch happens once, before the first write. `_setmode` is idempotent, so
 * the flag is only there to keep it off the hot path. */
static void otter_stdout_binary(void) {
#if defined(_WIN32)
    static int already_switched = 0;
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
 * make that impossible to get wrong. */

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
 * that never builds a string never allocates one. */

static otter_arena *otter_string_arena(void) {
    static otter_arena *arena = NULL;
    if (arena == NULL) {
        arena = otter_arena_new();
    }
    return arena;
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
