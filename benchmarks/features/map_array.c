/* The map_array.ot loop in C: build an array, map it into a second array,
 * and sum the second one. */
#include <stdio.h>
#include <stdlib.h>

#define N 10000000

int main(void) {
    long long *xs = malloc(sizeof(long long) * N);
    long long *doubled = malloc(sizeof(long long) * N);
    if (xs == NULL || doubled == NULL) {
        return 1;
    }
    for (long long i = 0; i < N; i++) {
        xs[i] = i;
    }
    for (long long i = 0; i < N; i++) {
        doubled[i] = xs[i] * 2;
    }
    long long sum = 0;
    for (long long i = 0; i < N; i++) {
        sum += doubled[i];
    }
    printf("%lld\n", sum);
    free(xs);
    free(doubled);
    return 0;
}
