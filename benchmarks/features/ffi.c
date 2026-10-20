/* The ffi.ot loop in C: 100 million calls to getpid. */
#include <stdio.h>

/* Declared rather than included: mingw spells the header differently from
   glibc, and this benchmark only needs the one symbol. */
extern int getpid(void);

int main(void) {
    long long n = 100000000;
    long long i = 0;
    long long sum = 0;
    while (i < n) {
        (void)getpid();
        sum += 1;
        i += 1;
    }
    printf("%lld\n", sum);
    return 0;
}
