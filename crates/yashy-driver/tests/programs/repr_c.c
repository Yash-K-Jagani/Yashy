/* The C half of the `repr(C)` differential test.
 *
 * `repr_c.ya` declares the same struct and prints the same five numbers from
 * Yashy's compile-time queries. The test compiles this file with `clang`, runs
 * both programs, and compares their output: if the two disagree, a Yashy
 * program passing this struct to a C function is reading the wrong bytes.
 *
 * This file includes no headers, and declares `printf` itself. A header would
 * make the test depend on a C SDK being installed and discoverable, which the
 * Yashy build does not need (the runtime is embedded in the compiler), so a
 * machine that can build Yashy could fail here for an unrelated reason. The
 * three queries are clang's own builtins; `__alignof__` and
 * `__builtin_offsetof` are the pre-header spellings of `_Alignof` and
 * `offsetof`. */
int printf(const char *format, ...);

struct Point {
    unsigned char tag;
    int x;
    int y;
};

int main(void) {
    printf("%zu\n", sizeof(struct Point));
    printf("%zu\n", __alignof__(struct Point));
    printf("%zu\n", __builtin_offsetof(struct Point, tag));
    printf("%zu\n", __builtin_offsetof(struct Point, x));
    printf("%zu\n", __builtin_offsetof(struct Point, y));
    return 0;
}
