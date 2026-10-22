/* 10,000,000 calls to a function that mutates its receiver through a pointer.
 *
 * The C equivalent of Otter's `self: *mut Counter`: the same struct, the same
 * address, the same increment in a loop. */
#include <stdio.h>

typedef struct {
    long long value;
} Counter;

static void bump(Counter *c) {
    c->value = c->value + 1;
}

static long long get(Counter c) {
    return c.value;
}

int main(void) {
    Counter c = {0};
    for (long long i = 0; i < 10000000; i++) {
        bump(&c);
    }
    printf("%lld\n", get(c));
    return 0;
}
