/* The function.ot loop calling the function directly: the same work, without
 * the function pointer. The difference between this row and the `function` row
 * is the price of calling through a value.
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
