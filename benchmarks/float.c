/* Benchmark 3: floating-point dependency chain. See float.ot. */
#include <stdio.h>

static double work(long long n) {
    double x = 1.0;
    for (long long i = 0; i < n; i++) {
        x = x * 1.0000001 + 0.0000001;
    }
    return x;
}

int main(void) {
    printf("%.17g\n", work(100000000));
    return 0;
}
