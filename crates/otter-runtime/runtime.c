#include "otter_runtime.h"

#include <stdio.h>

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
