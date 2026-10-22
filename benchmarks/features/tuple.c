#include <stdint.h>
#include <stdio.h>

typedef struct {
    int64_t lo;
    int64_t hi;
} pair;

static pair min_max(int64_t a, int64_t b) {
    pair p;
    if (a < b) {
        p.lo = a;
        p.hi = b;
    } else {
        p.lo = b;
        p.hi = a;
    }
    return p;
}

int main(void) {
    const int64_t rounds = 50000000;
    int64_t total = 0;
    for (int64_t i = 0; i < rounds; i++) {
        pair p = min_max(i, rounds - i);
        total += p.lo * 3 + p.hi;
    }
    printf("%lld\n", (long long)total);
    return 0;
}
