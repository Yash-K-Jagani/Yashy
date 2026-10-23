/* Concurrent retain/release on one shared string, as a real C program.
 *
 * `tests/racy_strings.rs` builds this file with the same compiler `build.rs`
 * uses and requires a zero exit status. Eight threads each keep the shared
 * string alive and let it go 100,000 times; the count must come back to the
 * one reference `main` still holds.
 *
 * A lost update is the failure this is built to catch: if two threads'
 * decrements overlap, the count reaches zero while a third still holds the
 * string, and the next read touches freed memory. The final
 * `yashy_str_refcount` check catches the other direction, a count that drifted
 * upward. Both are silent under a plain `++`/`--`; only the atomic pair keeps
 * them exact.
 *
 * The argument struct passed to `yashy_thread_spawn` is copied by the runtime,
 * so each thread gets its own copy and the shared pointer inside it is the one
 * value actually shared.
 */

#include "yashy_runtime.h"

#include <stdio.h>
#include <stdlib.h>

#define CHECK(condition)                                                       \
    do {                                                                       \
        if (!(condition)) {                                                    \
            fprintf(stderr, "%s:%d: check failed: %s\n", __FILE__, __LINE__,   \
                    #condition);                                               \
            exit(1);                                                           \
        }                                                                      \
    } while (0)

typedef struct racy_args {
    yashy_str *shared;
    long long iterations;
} racy_args;

static void churn(void *raw) {
    racy_args *args = (racy_args *)raw;
    for (long long i = 0; i < args->iterations; i++) {
        yashy_str *held = yashy_str_retain_atomic(args->shared);
        CHECK(held == args->shared);
        /* Read through the reference while it is held: a count that dropped to
         * zero too early would make this a use-after-free, caught here as a
         * wrong length (or a crash) rather than silently. */
        CHECK(yashy_str_len(held) == 1);
        yashy_str_release_atomic(held);
    }
}

int main(void) {
    yashy_str *shared = yashy_str_from_i64(7);
    CHECK(shared != NULL);
    CHECK(yashy_str_refcount(shared) == 1);

    enum { THREADS = 8, ITERATIONS = 100000 };
    racy_args args[THREADS];
    long long handles[THREADS];
    for (int i = 0; i < THREADS; i++) {
        args[i].shared = shared;
        args[i].iterations = ITERATIONS;
        handles[i] = yashy_thread_spawn(churn, &args[i], (long long)sizeof(racy_args));
        CHECK(handles[i] > 0);
    }
    for (int i = 0; i < THREADS; i++) {
        CHECK(yashy_thread_join((int)handles[i]) == 0);
    }

    CHECK(yashy_str_refcount(shared) == 1);
    CHECK(yashy_str_len(shared) == 1);
    yashy_str_release(shared);
    puts("racy strings ok");
    return 0;
}
