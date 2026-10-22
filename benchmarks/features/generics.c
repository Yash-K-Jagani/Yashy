/* C has no generic functions, so the honest comparison is the code the
 * compiler emits for the one instantiation: a plain function. This is the
 * "generic row" compiled away, and it should be as fast as Otter's.
 */
#include <stdio.h>

static long long scale(long long x, long long k) {
    return x * k + x % 3;
}

int main(void) {
    const long long n = 100000000;
    const long long k = 1000;
    long long sum = 0;
    for (long long i = 0; i < n; i++) {
        sum += scale(i, k);
    }
    printf("%lld\n", sum);
    return 0;
}
