/* Maps: an open-addressing table with linear probing.
 *
 * The table is deliberately opaque to generated code: every operation arrives
 * as a call, so the layout here can change without touching the IR or the
 * goldens. The only thing the compiler fixes is the *key kind* passed to
 * `otter_map_new`:
 *
 *     0 = i64      hash the eight bytes, compare with memcmp
 *     1 = bool     hash the single byte, compare with memcmp
 *     2 = string   hash the bytes through `otter_str_hash`, compare with
 *                  `otter_str_eq`, and retain/release the stored handle
 *
 * Keys and values live in parallel byte arrays with a stride, which is what
 * lets one C implementation serve every scalar element type. `states` holds
 * EMPTY = 0, OCCUPIED = 1, TOMBSTONE = 2. Tombstones keep probing chains
 * correct after a removal; the table rehashes into a fresh (tombstone-free)
 * array once occupied + tombstoned entries pass 70% of capacity.
 *
 * Every allocation goes through `otter_alloc`/`otter_free` so the leak report
 * sees the table's blocks. A failed allocation leaves the table unchanged:
 * `insert` has no error channel, and an unchanged table is the only state the
 * caller could observe (the same rule `push` follows for slices).
 */

#include "otter_runtime.h"

#include <limits.h>
#include <string.h>

enum {
    OTTER_MAP_EMPTY = 0,
    OTTER_MAP_OCCUPIED = 1,
    OTTER_MAP_TOMBSTONE = 2
};

struct otter_map {
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
static int otter_map_install_arrays(otter_map *map, long long cap) {
    unsigned char *states;
    unsigned char *keys;
    unsigned char *values;

    if (cap <= 0 || (cap & (cap - 1)) != 0) {
        return 0;
    }
    if (cap > LLONG_MAX / map->key_size || cap > LLONG_MAX / map->value_size) {
        return 0;
    }
    states = (unsigned char *)otter_alloc(cap, 8);
    keys = (unsigned char *)otter_alloc(cap * map->key_size, 8);
    values = (unsigned char *)otter_alloc(cap * map->value_size, 8);
    if (states == NULL || keys == NULL || values == NULL) {
        otter_free(states);
        otter_free(keys);
        otter_free(values);
        return 0;
    }
    memset(states, OTTER_MAP_EMPTY, (size_t)cap);
    map->states = states;
    map->keys = keys;
    map->values = values;
    map->cap = cap;
    map->tombstones = 0;
    return 1;
}

static unsigned long long otter_map_hash(const otter_map *map, const void *key) {
    if (map->key_kind == 2) {
        const otter_str *handle = *(const otter_str *const *)key;
        return otter_str_hash(handle);
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

static int otter_map_key_eq(const otter_map *map, const void *a, const void *b) {
    if (map->key_kind == 2) {
        const otter_str *left = *(const otter_str *const *)a;
        const otter_str *right = *(const otter_str *const *)b;
        return otter_str_eq(left, right);
    }
    return memcmp(a, b, (size_t)map->key_size) == 0;
}

/* The slot `key` occupies, or the slot it would occupy.
 *
 * `*found` says which: 1 means the key is at the returned slot; 0 means the
 * slot is EMPTY or a TOMBSTONE that can take it. A full table of
 * occupied/tombstoned slots has no answer and returns -1, but the load factor
 * keeps that from happening. */
static long long otter_map_slot(const otter_map *map, const void *key, int *found) {
    unsigned long long hash;
    unsigned long long mask;
    long long first_tombstone = -1;
    long long probe;

    *found = 0;
    if (map->cap <= 0) {
        return -1;
    }
    hash = otter_map_hash(map, key);
    mask = (unsigned long long)map->cap - 1;
    for (probe = 0; probe < map->cap; probe++) {
        long long slot = (long long)((hash + (unsigned long long)probe) & mask);
        unsigned char state = map->states[slot];
        if (state == OTTER_MAP_EMPTY) {
            return first_tombstone >= 0 ? first_tombstone : slot;
        }
        if (state == OTTER_MAP_TOMBSTONE) {
            if (first_tombstone < 0) {
                first_tombstone = slot;
            }
            continue;
        }
        if (otter_map_key_eq(map, map->keys + slot * map->key_size, key)) {
            *found = 1;
            return slot;
        }
    }
    return first_tombstone;
}

static int otter_map_grow(otter_map *map) {
    otter_map grown;
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
    if (!otter_map_install_arrays(&grown, new_cap)) {
        return 0;
    }
    for (index = 0; index < map->cap; index++) {
        int found = 0;
        long long slot;
        if (map->states[index] != OTTER_MAP_OCCUPIED) {
            continue;
        }
        slot = otter_map_slot(&grown, map->keys + index * map->key_size, &found);
        if (slot < 0) {
            /* Unreachable: the new table has at least twice the slots of the
             * old one and only the occupied entries move. Keep the old table
             * rather than returning a half-filled one. */
            otter_free(grown.states);
            otter_free(grown.keys);
            otter_free(grown.values);
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
        grown.states[slot] = OTTER_MAP_OCCUPIED;
        grown.len++;
    }
    otter_free(map->states);
    otter_free(map->keys);
    otter_free(map->values);
    map->states = grown.states;
    map->keys = grown.keys;
    map->values = grown.values;
    map->cap = grown.cap;
    map->len = grown.len;
    map->tombstones = 0;
    return 1;
}

otter_map *otter_map_new(long long key_kind, long long key_size, long long value_size) {
    otter_map *map;

    if (key_kind != 0 && key_kind != 1 && key_kind != 2) {
        return NULL;
    }
    if (key_size <= 0 || value_size <= 0) {
        return NULL;
    }
    map = (otter_map *)otter_alloc((long long)sizeof(otter_map), 8);
    if (map == NULL) {
        return NULL;
    }
    memset(map, 0, sizeof *map);
    map->key_kind = key_kind;
    map->key_size = key_size;
    map->value_size = value_size;
    return map;
}

void otter_map_free(otter_map *map) {
    long long index;

    if (map == NULL) {
        return;
    }
    if (map->key_kind == 2 && map->keys != NULL) {
        for (index = 0; index < map->cap; index++) {
            if (map->states[index] != OTTER_MAP_OCCUPIED) {
                continue;
            }
            otter_str_release(
                *(otter_str *const *)(map->keys + index * map->key_size)
            );
        }
    }
    otter_free(map->states);
    otter_free(map->keys);
    otter_free(map->values);
    otter_free(map);
}

void otter_map_insert(otter_map *map, const void *key, const void *value) {
    int found = 0;
    long long slot;

    if (map == NULL || key == NULL || value == NULL) {
        return;
    }
    if (map->cap == 0 || (map->len + map->tombstones + 1) * 10 >= map->cap * 7) {
        if (!otter_map_grow(map)) {
            return;
        }
    }
    slot = otter_map_slot(map, key, &found);
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
    if (map->states[slot] == OTTER_MAP_TOMBSTONE) {
        map->tombstones--;
    }
    memcpy(map->keys + slot * map->key_size, key, (size_t)map->key_size);
    if (map->key_kind == 2) {
        otter_str_retain(*(otter_str *const *)key);
    }
    memcpy(
        map->values + slot * map->value_size,
        value,
        (size_t)map->value_size
    );
    map->states[slot] = OTTER_MAP_OCCUPIED;
    map->len++;
}

long long otter_map_get(const otter_map *map, const void *key, void *out) {
    int found = 0;
    long long slot;

    if (map == NULL || key == NULL || out == NULL || map->cap == 0) {
        return 0;
    }
    slot = otter_map_slot(map, key, &found);
    if (!found || slot < 0) {
        return 0;
    }
    memcpy(out, map->values + slot * map->value_size, (size_t)map->value_size);
    return 1;
}

long long otter_map_remove(otter_map *map, const void *key, void *out) {
    int found = 0;
    long long slot;

    if (map == NULL || key == NULL || out == NULL || map->cap == 0) {
        return 0;
    }
    slot = otter_map_slot(map, key, &found);
    if (!found || slot < 0) {
        return 0;
    }
    memcpy(out, map->values + slot * map->value_size, (size_t)map->value_size);
    if (map->key_kind == 2) {
        otter_str_release(
            *(otter_str *const *)(map->keys + slot * map->key_size)
        );
    }
    map->states[slot] = OTTER_MAP_TOMBSTONE;
    map->tombstones++;
    map->len--;
    return 1;
}

long long otter_map_len(const otter_map *map) {
    return map == NULL ? 0 : map->len;
}
