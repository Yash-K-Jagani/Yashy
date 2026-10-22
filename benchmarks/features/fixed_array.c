// The C counterpart of fixed_array.ot: a stack array, summed 50,000,000 times.
// Every element add updates the outer accumulator, so the inner loop cannot be
// folded away.
#include <stdint.h>
#include <stdio.h>

int main(void) {
    const int64_t grid[8] = {1, 2, 3, 4, 5, 6, 7, 8};
    const int64_t rounds = 50000000;
    int64_t total = 0;
    for (int64_t i = 0; i < rounds; i++) {
        for (int64_t j = 0; j < 8; j++) {
            total += grid[j];
        }
    }
    printf("%lld\n", (long long)total);
    return 0;
}
