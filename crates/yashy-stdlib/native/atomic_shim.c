/* Yashy-facing atomic slots for i64 operations.
 *
 * The Yashy API names an ordering explicitly; the integer code is mapped to a
 * C11 memory_order here. There is no implicit sequential consistency: an
 * operation with no ordering cannot be emitted by the Yashy wrapper.
 */
#include <stdatomic.h>
#include <stdint.h>

#define YASHY_ATOMIC_SLOTS 512
static _Atomic int64_t yashy_atomic_i64[YASHY_ATOMIC_SLOTS];
static atomic_int yashy_atomic_next = 0;

static memory_order yashy_order(int code) {
    switch (code) {
        case 0: return memory_order_relaxed;
        case 1: return memory_order_acquire;
        case 2: return memory_order_release;
        case 3: return memory_order_acq_rel;
        default: return memory_order_seq_cst;
    }
}

int yashy_atomic_i64_new(int64_t value) {
    int id = atomic_fetch_add_explicit(&yashy_atomic_next, 1, memory_order_relaxed);
    if (id < 0 || id >= YASHY_ATOMIC_SLOTS) return -1;
    atomic_store_explicit(&yashy_atomic_i64[id], value, memory_order_seq_cst);
    return id;
}

int64_t yashy_atomic_i64_load(int id, int order) {
    return atomic_load_explicit(&yashy_atomic_i64[id], yashy_order(order));
}

void yashy_atomic_i64_store(int id, int64_t value, int order) {
    atomic_store_explicit(&yashy_atomic_i64[id], value, yashy_order(order));
}

int64_t yashy_atomic_i64_fetch_add(int id, int64_t value, int order) {
    return atomic_fetch_add_explicit(&yashy_atomic_i64[id], value, yashy_order(order));
}

int64_t yashy_atomic_i64_compare_exchange(
    int id, int64_t expected, int64_t desired, int success, int failure
) {
    (void)failure;
    int64_t observed = expected;
    atomic_compare_exchange_strong_explicit(
        &yashy_atomic_i64[id], &observed, desired, yashy_order(success), yashy_order(failure)
    );
    return observed;
}
