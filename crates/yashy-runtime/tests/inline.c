/* Exercises the header's `static inline` fast paths by calling them directly.
 *
 * They have no external symbol, so a Rust test cannot link to them; the point
 * of compiling this file with the real runtime is that the body the compiler
 * inlines here is the body every C caller gets. The checks pin the contract
 * each entry point is licensed by: an unchecked slice read has no guard, the
 * small allocator keeps the alignment and the live-block count, and the map
 * fast path rejects a call that cannot hit before entering the table.
 */

#include "yashy_runtime.h"

#include <stdio.h>

static int failures = 0;

static void check(int condition, const char *what) {
    if (!condition) {
        fprintf(stderr, "FAIL: %s\n", what);
        failures++;
    }
}

static void check_slice_get_unchecked(void) {
    yashy_slice slice = {0};
    long long first = 11;
    long long second = 22;
    long long out = 0;

    yashy_slice_push(&slice, &first, (long long)sizeof first);
    yashy_slice_push(&slice, &second, (long long)sizeof second);

    yashy_slice_get_unchecked(&slice, 0, &out, (long long)sizeof out);
    check(out == 11, "unchecked read at index 0 returns the first element");
    yashy_slice_get_unchecked(&slice, 1, &out, (long long)sizeof out);
    check(out == 22, "unchecked read at index 1 returns the second element");

    yashy_slice_free(&slice);
}

static void check_alloc_small(void) {
    const long long aligns[4] = {1, 8, 16, 64};
    void *blocks[4];
    long long before = yashy_live_allocations_now();
    int index;

    for (index = 0; index < 4; index++) {
        blocks[index] = yashy_alloc_small(24, aligns[index]);
        check(blocks[index] != NULL, "a small allocation succeeds");
        check(((uintptr_t)blocks[index] % (uintptr_t)aligns[index]) == 0,
              "a small allocation honours its alignment");
        memset(blocks[index], 0, 24);
    }
    check(yashy_live_allocations_now() == before + 4,
          "each successful small allocation counts once");

    for (index = 0; index < 4; index++) {
        yashy_free(blocks[index]);
    }
    check(yashy_live_allocations_now() == before,
          "freeing the small allocations returns the count");

    /* Outside the window, `yashy_alloc_small` delegates to `yashy_alloc`.
     * An alignment smaller than a pointer is normalised up to pointer size, so
     * 3 gives a pointer-aligned block; 12 is pointer-sized or larger and not a
     * power of two, so it is repaired to 16. */
    {
        void *weird = yashy_alloc_small(8, 3);
        check(weird != NULL, "a non-power-of-two alignment still allocates");
        check(((uintptr_t)weird % sizeof(void *)) == 0,
              "the checked allocator normalises the small alignment");
        yashy_free(weird);
    }
    {
        void *odd = yashy_alloc_small(8, 12);
        check(odd != NULL, "a large non-power-of-two alignment still allocates");
        check(((uintptr_t)odd % 16) == 0,
              "the checked allocator repairs it to 16");
        yashy_free(odd);
    }

    /* A non-positive size is one byte, exactly as `yashy_alloc` promises. */
    {
        void *tiny = yashy_alloc_small(0, 8);
        check(tiny != NULL, "a zero-byte request still yields a block");
        yashy_free(tiny);
    }
}

static void check_map_get_fast(void) {
    yashy_map *map = yashy_map_new(0, (long long)sizeof(long long),
                                  (long long)sizeof(long long));
    long long key = 7;
    long long missing = 8;
    long long value = 99;
    long long got = 0;

    check(map != NULL, "a map is created");
    yashy_map_insert(map, &key, &value);

    check(yashy_map_get_fast(map, &key, &got) == 1, "a present key is found");
    check(got == 99, "the found value is copied out");
    check(yashy_map_get_fast(map, &missing, &got) == 0,
          "a missing key is a miss");
    check(yashy_map_get_fast(NULL, &key, &got) == 0,
          "a null map is a miss, not a crash");
    check(yashy_map_get_fast(map, NULL, &got) == 0,
          "a null key is a miss");
    check(yashy_map_get_fast(map, &key, NULL) == 0,
          "a null out slot is a miss");

    yashy_map_free(map);
}

int main(void) {
    check_slice_get_unchecked();
    check_alloc_small();
    check_map_get_fast();

    if (failures != 0) {
        fprintf(stderr, "%d inline fast-path check(s) failed\n", failures);
        return 1;
    }
    printf("inline fast paths ok\n");
    return 0;
}
