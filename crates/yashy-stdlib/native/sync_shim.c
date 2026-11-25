/* Reader/writer lock, condition variable and barrier slots.
 *
 * The shim keeps a fixed table of C11 synchronization objects. The Yashy API
 * only sees integer handles, so no program has to name a C pointer. A
 * condition-variable wait is always paired with a caller-side predicate loop;
 * the shim provides the signal, not the predicate.
 */
#include <threads.h>
#include <stdatomic.h>
#include <stdint.h>

#define YASHY_SYNC_SLOTS 256

typedef struct {
    mtx_t mutex;
    cnd_t changed;
    int readers;
    int writer;
} yashy_rwlock;

typedef struct {
    mtx_t mutex;
    cnd_t changed;
    unsigned long long generation;
} yashy_cond;

typedef struct {
    mtx_t mutex;
    cnd_t changed;
    int count;
    int parties;
    unsigned long long generation;
} yashy_barrier;

static yashy_rwlock yashy_rwlocks[YASHY_SYNC_SLOTS];
static yashy_cond yashy_conds[YASHY_SYNC_SLOTS];
static yashy_barrier yashy_barriers[YASHY_SYNC_SLOTS];

static atomic_int yashy_rw_next = 0;
static atomic_int yashy_cond_next = 0;
static atomic_int yashy_barrier_next = 0;

int yashy_rwlock_new(void) {
    int id = atomic_fetch_add(&yashy_rw_next, 1);
    mtx_init(&yashy_rwlocks[id].mutex, mtx_plain);
    cnd_init(&yashy_rwlocks[id].changed);
    yashy_rwlocks[id].readers = 0;
    yashy_rwlocks[id].writer = 0;
    return id;
}

void yashy_rwlock_rdlock(int id) {
    mtx_lock(&yashy_rwlocks[id].mutex);
    while (yashy_rwlocks[id].writer) {
        cnd_wait(&yashy_rwlocks[id].changed, &yashy_rwlocks[id].mutex);
    }
    yashy_rwlocks[id].readers++;
    mtx_unlock(&yashy_rwlocks[id].mutex);
}

void yashy_rwlock_wrlock(int id) {
    mtx_lock(&yashy_rwlocks[id].mutex);
    while (yashy_rwlocks[id].readers > 0 || yashy_rwlocks[id].writer) {
        cnd_wait(&yashy_rwlocks[id].changed, &yashy_rwlocks[id].mutex);
    }
    yashy_rwlocks[id].writer = 1;
    mtx_unlock(&yashy_rwlocks[id].mutex);
}

void yashy_rwlock_unlock(int id) {
    mtx_lock(&yashy_rwlocks[id].mutex);
    if (yashy_rwlocks[id].writer) {
        yashy_rwlocks[id].writer = 0;
    } else if (yashy_rwlocks[id].readers > 0) {
        yashy_rwlocks[id].readers--;
    }
    cnd_broadcast(&yashy_rwlocks[id].changed);
    mtx_unlock(&yashy_rwlocks[id].mutex);
}

int yashy_cond_new(void) {
    int id = atomic_fetch_add(&yashy_cond_next, 1);
    mtx_init(&yashy_conds[id].mutex, mtx_plain);
    cnd_init(&yashy_conds[id].changed);
    yashy_conds[id].generation = 0;
    return id;
}

unsigned long long yashy_cond_generation(int id) {
    mtx_lock(&yashy_conds[id].mutex);
    unsigned long long value = yashy_conds[id].generation;
    mtx_unlock(&yashy_conds[id].mutex);
    return value;
}

void yashy_cond_wait(int id, unsigned long long observed) {
    mtx_lock(&yashy_conds[id].mutex);
    while (yashy_conds[id].generation == observed) {
        cnd_wait(&yashy_conds[id].changed, &yashy_conds[id].mutex);
    }
    mtx_unlock(&yashy_conds[id].mutex);
}

void yashy_cond_signal(int id) {
    mtx_lock(&yashy_conds[id].mutex);
    yashy_conds[id].generation++;
    cnd_broadcast(&yashy_conds[id].changed);
    mtx_unlock(&yashy_conds[id].mutex);
}

int yashy_barrier_new(int parties) {
    int id = atomic_fetch_add(&yashy_barrier_next, 1);
    mtx_init(&yashy_barriers[id].mutex, mtx_plain);
    cnd_init(&yashy_barriers[id].changed);
    yashy_barriers[id].count = 0;
    yashy_barriers[id].parties = parties;
    yashy_barriers[id].generation = 0;
    return id;
}

unsigned long long yashy_barrier_wait(int id) {
    mtx_lock(&yashy_barriers[id].mutex);
    unsigned long long generation = yashy_barriers[id].generation;
    yashy_barriers[id].count++;
    if (yashy_barriers[id].count >= yashy_barriers[id].parties) {
        yashy_barriers[id].count = 0;
        yashy_barriers[id].generation++;
        cnd_broadcast(&yashy_barriers[id].changed);
    } else {
        while (yashy_barriers[id].generation == generation) {
            cnd_wait(&yashy_barriers[id].changed, &yashy_barriers[id].mutex);
        }
    }
    unsigned long long result = yashy_barriers[id].generation;
    mtx_unlock(&yashy_barriers[id].mutex);
    return result;
}
