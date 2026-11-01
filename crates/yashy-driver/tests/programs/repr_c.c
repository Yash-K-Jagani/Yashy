/* The C half of the `repr(C)` differential test.
 *
 * `repr_c.ya` declares the same struct and prints the same five numbers from
 * Yashy's compile-time queries. The test compiles this file with `clang`, runs
 * both programs, and compares their output: if the two disagree, a Yashy
 * program passing this struct to a C function is reading the wrong bytes.
 */
#include <stdalign.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>

struct Point {
    uint8_t tag;
    int32_t x;
    int32_t y;
};

int main(void) {
    printf("%zu\n", sizeof(struct Point));
    printf("%zu\n", _Alignof(struct Point));
    printf("%zu\n", offsetof(struct Point, tag));
    printf("%zu\n", offsetof(struct Point, x));
    printf("%zu\n", offsetof(struct Point, y));
    return 0;
}
