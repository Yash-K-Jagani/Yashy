/* The slice.ot loop with a hand-grown C array: double the capacity on fill. */
#include <stdio.h>
#include <stdlib.h>

int main(void) {
    long long n = 10000000;
    long long *xs = NULL;
    size_t len = 0;
    size_t cap = 0;

    for (long long i = 0; i < n; i++) {
        if (len == cap) {
            cap = cap > 0 ? cap * 2 : 4;
            xs = realloc(xs, cap * sizeof *xs);
            if (xs == NULL) {
                return 1;
            }
        }
        xs[len++] = i;
    }

    long long sum = 0;
    for (size_t i = 0; i < len; i++) {
        sum += xs[i];
    }
    printf("%lld\n", sum);
    free(xs);
    return 0;
}
