/* Maps: an open-addressing table with linear probing.
 *
 * The table is deliberately opaque to generated code: every operation arrives
 * as a call, so the layout here can change without touching the IR or the
 * goldens. The only thing the compiler fixes is the *key kind* passed to
 * `yashy_map_new`:
 *
 *     0 = i64      hash the eight bytes, compare with memcmp
 *     1 = bool     hash the single byte, compare with memcmp
 *     2 = string   hash the bytes through `yashy_str_hash`, compare with
 *                  `yashy_str_eq`, and retain/release the stored handle
 *
 * Keys and values live in parallel byte arrays with a stride, which is what
 * lets one C implementation serve every scalar element type. `states` holds
 * EMPTY = 0, OCCUPIED = 1, TOMBSTONE = 2. Tombstones keep probing chains
 * correct after a removal; the table rehashes into a fresh (tombstone-free)
 * array once occupied + tombstoned entries pass 70% of capacity.
 *
 * Every allocation goes through `yashy_alloc`/`yashy_free` so the leak report
 * sees the table's blocks. A failed allocation leaves the table unchanged:
 * `insert` has no error channel, and an unchanged table is the only state the
 * caller could observe (the same rule `push` follows for slices).
 */

#include "yashy_runtime.h"

#include <limits.h>
#include <string.h>

enum {
    YASHY_MAP_EMPTY = 0,
    YASHY_MAP_OCCUPIED = 1,
    YASHY_MAP_TOMBSTONE = 2
};

struct yashy_map {
    long long key_kind;
    long long key_size;
    long long value_size;
    long long len;
    long long cap;
    long long tombstones;
    unsigned char *states;
    unsigned char *keys;
    unsigned char *values;
};

/* Allocate the three arrays for `cap` slots and install them. The table must
 * currently have no arrays (`cap == 0`) or have had its old ones detached. */
static int yashy_map_install_arrays(yashy_map *map, long long cap) {
    unsigned char *states;
    unsigned char *keys;
    unsigned char *values;

    if (cap <= 0 || (cap & (cap - 1)) != 0) {
        return 0;
    }
    if (cap > LLONG_MAX / map->key_size || cap > LLONG_MAX / map->value_size) {
        return 0;
    }
    states = (unsigned char *)yashy_alloc(cap, 8);
    keys = (unsigned char *)yashy_alloc(cap * map->key_size, 8);
    values = (unsigned char *)yashy_alloc(cap * map->value_size, 8);
    if (states == NULL || keys == NULL || values == NULL) {
        yashy_free(states);
        yashy_free(keys);
        yashy_free(values);
        return 0;
    }
    memset(states, YASHY_MAP_EMPTY, (size_t)cap);
    map->states = states;
    map->keys = keys;
    map->values = values;
    map->cap = cap;
    map->tombstones = 0;
    return 1;
}

static unsigned long long yashy_map_hash(const yashy_map *map, const void *key) {
    if (map->key_kind == 2) {
        const yashy_str *handle = *(const yashy_str *const *)key;
        return yashy_str_hash(handle);
    }
    if (map->key_kind == 1) {
        return *(const unsigned char *)key != 0 ? 0x9e3779b97f4a7c15ULL : 0ULL;
    }
    {
        /* splitmix64's finalizer, which spreads adjacent integer keys well. */
        unsigned long long value = 0;
        memcpy(&value, key, sizeof value);
        value ^= value >> 30;
        value *= 0xbf58476d1ce4e5b9ULL;
        value ^= value >> 27;
        value *= 0x94d049bb133111ebULL;
        value ^= value >> 31;
        return value;
    }
}

static int yashy_map_key_eq(const yashy_map *map, const void *a, const void *b) {
    if (map->key_kind == 2) {
        const yashy_str *left = *(const yashy_str *const *)a;
        const yashy_str *right = *(const yashy_str *const *)b;
        return yashy_str_eq(left, right);
    }
    return memcmp(a, b, (size_t)map->key_size) == 0;
}

/* The slot `key` occupies, or the slot it would occupy.
 *
 * `*found` says which: 1 means the key is at the returned slot; 0 means the
 * slot is EMPTY or a TOMBSTONE that can take it. A full table of
 * occupied/tombstoned slots has no answer and returns -1, but the load factor
 * keeps that from happening. */
static long long yashy_map_slot(const yashy_map *map, const void *key, int *found) {
    unsigned long long hash;
    unsigned long long mask;
    long long first_tombstone = -1;
    long long probe;

    *found = 0;
    if (map->cap <= 0) {
        return -1;
    }
    hash = yashy_map_hash(map, key);
    mask = (unsigned long long)map->cap - 1;
    for (probe = 0; probe < map->cap; probe++) {
        long long slot = (long long)((hash + (unsigned long long)probe) & mask);
        unsigned char state = map->states[slot];
        if (state == YASHY_MAP_EMPTY) {
            return first_tombstone >= 0 ? first_tombstone : slot;
        }
        if (state == YASHY_MAP_TOMBSTONE) {
            if (first_tombstone < 0) {
                first_tombstone = slot;
            }
            continue;
        }
        if (yashy_map_key_eq(map, map->keys + slot * map->key_size, key)) {
            *found = 1;
            return slot;
        }
    }
    return first_tombstone;
}

static int yashy_map_grow(yashy_map *map) {
    yashy_map grown;
    long long new_cap;
    long long index;

    new_cap = map->cap == 0 ? 8 : map->cap * 2;
    if (new_cap <= map->cap) {
        return 0;
    }
    grown = *map;
    grown.len = 0;
    grown.cap = 0;
    grown.tombstones = 0;
    grown.states = NULL;
    grown.keys = NULL;
    grown.values = NULL;
    if (!yashy_map_install_arrays(&grown, new_cap)) {
        return 0;
    }
    for (index = 0; index < map->cap; index++) {
        int found = 0;
        long long slot;
        if (map->states[index] != YASHY_MAP_OCCUPIED) {
            continue;
        }
        slot = yashy_map_slot(&grown, map->keys + index * map->key_size, &found);
        if (slot < 0) {
            /* Unreachable: the new table has at least twice the slots of the
             * old one and only the occupied entries move. Keep the old table
             * rather than returning a half-filled one. */
            yashy_free(grown.states);
            yashy_free(grown.keys);
            yashy_free(grown.values);
            return 0;
        }
        memcpy(
            grown.keys + slot * map->key_size,
            map->keys + index * map->key_size,
            (size_t)map->key_size
        );
        memcpy(
            grown.values + slot * map->value_size,
            map->values + index * map->value_size,
            (size_t)map->value_size
        );
        grown.states[slot] = YASHY_MAP_OCCUPIED;
        grown.len++;
    }
    yashy_free(map->states);
    yashy_free(map->keys);
    yashy_free(map->values);
    map->states = grown.states;
    map->keys = grown.keys;
    map->values = grown.values;
    map->cap = grown.cap;
    map->len = grown.len;
    map->tombstones = 0;
    return 1;
}

yashy_map *yashy_map_new(long long key_kind, long long key_size, long long value_size) {
    yashy_map *map;

    if (key_kind != 0 && key_kind != 1 && key_kind != 2) {
        return NULL;
    }
    if (key_size <= 0 || value_size <= 0) {
        return NULL;
    }
    map = (yashy_map *)yashy_alloc((long long)sizeof(yashy_map), 8);
    if (map == NULL) {
        return NULL;
    }
    memset(map, 0, sizeof *map);
    map->key_kind = key_kind;
    map->key_size = key_size;
    map->value_size = value_size;
    return map;
}

void yashy_map_free(yashy_map *map) {
    long long index;

    if (map == NULL) {
        return;
    }
    if (map->key_kind == 2 && map->keys != NULL) {
        for (index = 0; index < map->cap; index++) {
            if (map->states[index] != YASHY_MAP_OCCUPIED) {
                continue;
            }
            yashy_str_release(
                *(yashy_str *const *)(map->keys + index * map->key_size)
            );
        }
    }
    yashy_free(map->states);
    yashy_free(map->keys);
    yashy_free(map->values);
    yashy_free(map);
}

void yashy_map_insert(yashy_map *map, const void *key, const void *value) {
    int found = 0;
    long long slot;

    if (map == NULL || key == NULL || value == NULL) {
        return;
    }
    if (map->cap == 0 || (map->len + map->tombstones + 1) * 10 >= map->cap * 7) {
        if (!yashy_map_grow(map)) {
            return;
        }
    }
    slot = yashy_map_slot(map, key, &found);
    if (slot < 0) {
        return;
    }
    if (found) {
        /* The stored key and its reference stay; only the value changes. */
        memcpy(
            map->values + slot * map->value_size,
            value,
            (size_t)map->value_size
        );
        return;
    }
    if (map->states[slot] == YASHY_MAP_TOMBSTONE) {
        map->tombstones--;
    }
    memcpy(map->keys + slot * map->key_size, key, (size_t)map->key_size);
    if (map->key_kind == 2) {
        yashy_str_retain(*(yashy_str *const *)key);
    }
    memcpy(
        map->values + slot * map->value_size,
        value,
        (size_t)map->value_size
    );
    map->states[slot] = YASHY_MAP_OCCUPIED;
    map->len++;
}

long long yashy_map_get(const yashy_map *map, const void *key, void *out) {
    int found = 0;
    long long slot;

    if (map == NULL || key == NULL || out == NULL || map->cap == 0) {
        return 0;
    }
    slot = yashy_map_slot(map, key, &found);
    if (!found || slot < 0) {
        return 0;
    }
    memcpy(out, map->values + slot * map->value_size, (size_t)map->value_size);
    return 1;
}

long long yashy_map_remove(yashy_map *map, const void *key, void *out) {
    int found = 0;
    long long slot;

    if (map == NULL || key == NULL || out == NULL || map->cap == 0) {
        return 0;
    }
    slot = yashy_map_slot(map, key, &found);
    if (!found || slot < 0) {
        return 0;
    }
    memcpy(out, map->values + slot * map->value_size, (size_t)map->value_size);
    if (map->key_kind == 2) {
        yashy_str_release(
            *(yashy_str *const *)(map->keys + slot * map->key_size)
        );
    }
    map->states[slot] = YASHY_MAP_TOMBSTONE;
    map->tombstones++;
    map->len--;
    return 1;
}

long long yashy_map_len(const yashy_map *map) {
    return map == NULL ? 0 : map->len;
}

long long yashy_map_contains_key(const yashy_map *map, const void *key) {
    int found = 0;

    if (map == NULL || key == NULL || map->cap == 0) {
        return 0;
    }
    yashy_map_slot(map, key, &found);
    return found ? 1 : 0;
}

/* Fills `out` with the table's keys, in the order the table holds them.
 *
 * The copy is a plain byte copy of the stored keys, so the array's element
 * stride is the table's key stride and the two agree without the runtime
 * needing to know the key type. A `string` key is a handle: the array
 * retains it, so the array owns a reference of its own and the map can be
 * freed without pulling the strings out from under it.
 *
 * A failed allocation leaves `out` empty, which is the only state a caller
 * could observe -- the same rule `push` follows for slices. */
void yashy_map_keys(yashy_slice *out, const yashy_map *map) {
    long long index;
    long long count = 0;
    unsigned char *data;

    if (out == NULL) {
        return;
    }
    out->data = NULL;
    out->len = 0;
    out->cap = 0;
    if (map == NULL || map->cap == 0) {
        return;
    }
    for (index = 0; index < map->cap; index++) {
        if (map->states[index] == YASHY_MAP_OCCUPIED) {
            count++;
        }
    }
    if (count == 0) {
        return;
    }
    data = (unsigned char *)yashy_alloc(count * map->key_size, 8);
    if (data == NULL) {
        return;
    }
    count = 0;
    for (index = 0; index < map->cap; index++) {
        if (map->states[index] != YASHY_MAP_OCCUPIED) {
            continue;
        }
        if (map->key_kind == 2) {
            yashy_str_retain(
                *(yashy_str *const *)(map->keys + index * map->key_size)
            );
        }
        memcpy(
            data + count * map->key_size,
            map->keys + index * map->key_size,
            (size_t)map->key_size
        );
        count++;
    }
    out->data = data;
    out->len = count;
    out->cap = count;
}

/* Fills `out` with the table's values, in the same order as `yashy_map_keys`.
 *
 * A map value is always a number or a `bool`, so the copy owns nothing and
 * the table keeps every value it holds. */
void yashy_map_values(yashy_slice *out, const yashy_map *map) {
    long long index;
    long long count = 0;
    unsigned char *data;

    if (out == NULL) {
        return;
    }
    out->data = NULL;
    out->len = 0;
    out->cap = 0;
    if (map == NULL || map->cap == 0) {
        return;
    }
    for (index = 0; index < map->cap; index++) {
        if (map->states[index] == YASHY_MAP_OCCUPIED) {
            count++;
        }
    }
    if (count == 0) {
        return;
    }
    data = (unsigned char *)yashy_alloc(count * map->value_size, 8);
    if (data == NULL) {
        return;
    }
    count = 0;
    for (index = 0; index < map->cap; index++) {
        if (map->states[index] != YASHY_MAP_OCCUPIED) {
            continue;
        }
        memcpy(
            data + count * map->value_size,
            map->values + index * map->value_size,
            (size_t)map->value_size
        );
        count++;
    }
    out->data = data;
    out->len = count;
    out->cap = count;
}
