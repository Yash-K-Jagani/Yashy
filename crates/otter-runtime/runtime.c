#include "otter_runtime.h"

#include <stdio.h>

void otter_print_str(const char *s) {
    if (s == NULL) {
        return;
    }
    fputs(s, stdout);
}

void otter_print_i64(long long value) {
    printf("%lld", value);
}

void otter_print_f64(double value) {
    printf("%.17g", value);
}
