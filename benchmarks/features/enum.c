/* The enum.ot loop with a C tagged struct: build, then switch on the tag. */
#include <stdio.h>

typedef struct {
    int tag;
    long long n;
} Msg;

static Msg make(long long i) {
    long long r = i % 3;
    Msg m;
    if (r == 0) {
        m.tag = 0;
        m.n = i;
    } else if (r == 1) {
        m.tag = 1;
        m.n = i;
    } else {
        m.tag = 2;
        m.n = 0;
    }
    return m;
}

static long long score(Msg m) {
    switch (m.tag) {
    case 0:
        return m.n;
    case 1:
        return m.n * 2;
    default:
        return 0;
    }
}

int main(void) {
    long long n = 50000000;
    long long sum = 0;
    for (long long i = 0; i < n; i++) {
        sum += score(make(i));
    }
    printf("%lld\n", sum);
    return 0;
}
