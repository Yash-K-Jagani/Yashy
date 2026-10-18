/* Benchmark 2: integer loop with a data-dependent branch. See loop.ot. */
#include <stdio.h>

static long long work(long long n) {
    long long acc = 0;
    for (long long i = 0; i < n; i++) {
        if (i % 2 == 0) {
            acc = acc + i;
        } else {
            acc = acc + i * 2;
        }
    }
    return acc;
}

int main(void) {
    printf("%lld\n", work(200000000));
    return 0;
}
