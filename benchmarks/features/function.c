/* The function.ot loop with a C function pointer.
 *
 * The pointer is hidden from the optimizer with an empty asm barrier, the C
 * spelling of "the compiler may not know what this points at". Without the
 * barrier the whole loop collapses back into a direct call and the row would
 * measure the wrong thing.
 */
#include <stdio.h>

typedef long long (*binop)(long long, long long);

static long long scale(long long x, long long k) {
    return x * k + x % 3;
}

__attribute__((noinline)) static long long apply(binop f, long long x, long long k) {
    return f(x, k);
}

int main(void) {
    const long long n = 100000000;
    const long long k = 1000;
    binop f = scale;
    __asm__ volatile("" : "+r"(f));

    long long sum = 0;
    for (long long i = 0; i < n; i++) {
        sum += apply(f, i, k);
    }
    printf("%lld\n", sum);
    return 0;
}
