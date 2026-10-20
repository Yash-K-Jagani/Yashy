/* The thread, mutex and channel primitives, exercised as a real C program with
 * real OS threads. `tests/threads.rs` builds this file with the same compiler
 * `build.rs` uses and requires a zero exit status.
 *
 * Three properties matter beyond "it works":
 *
 * - four threads adding to one counter under a mutex total exactly the sum, so
 *   the lock both excludes and is not deadlocking;
 * - eight threads pushing 1000 integers each through a channel of capacity 16
 *   deliver every value, so a full channel blocks a sender instead of dropping
 *   or overwriting a slot;
 * - closing a channel releases a blocked sender with 0 rather than hanging,
 *   which is the one case the runtime must never deadlock on.
 *
 * The argument passed to `otter_thread_spawn` is *copied* by the runtime, so a
 * struct that wants to report back carries a pointer to state the spawning
 * thread still owns. The copied struct itself is not shared.
 */

#include "otter_runtime.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define CHECK(condition)                                                       \
    do {                                                                       \
        if (!(condition)) {                                                    \
            fprintf(stderr, "%s:%d: check failed: %s\n", __FILE__, __LINE__,   \
                    #condition);                                               \
            exit(1);                                                           \
        }                                                                      \
    } while (0)

/* ---- 1: a mutex around a shared counter -------------------------------- */

typedef struct counter_args {
    otter_mutex *mutex;
    long long *counter;
    long long increments;
} counter_args;

static void count_up(void *raw) {
    counter_args *args = (counter_args *)raw;
    for (long long i = 0; i < args->increments; i++) {
        otter_mutex_lock(args->mutex);
        (*args->counter)++;
        otter_mutex_unlock(args->mutex);
    }
}

static void four_threads_add_under_a_mutex(void) {
    enum { THREADS = 4, INCREMENTS = 10000 };
    otter_mutex *mutex = otter_mutex_new();
    CHECK(mutex != NULL);

    long long counter = 0;
    counter_args args = {mutex, &counter, INCREMENTS};
    int handles[THREADS];
    for (int i = 0; i < THREADS; i++) {
        handles[i] = otter_thread_spawn(count_up, &args, (long long)sizeof args);
        CHECK(handles[i] != 0);
    }
    for (int i = 0; i < THREADS; i++) {
        CHECK(otter_thread_join(handles[i]) == 0);
    }

    CHECK(counter == (long long)THREADS * INCREMENTS);
    otter_mutex_free(mutex);
}

/* ---- 2: many senders, one receiver ------------------------------------- */

typedef struct sender_args {
    otter_chan *chan;
    long long count;
    int *failed;
} sender_args;

static void send_values(void *raw) {
    sender_args *args = (sender_args *)raw;
    for (long long i = 0; i < args->count; i++) {
        if (otter_chan_send(args->chan, &i, (long long)sizeof i) != 1) {
            *args->failed = 1;
            return;
        }
    }
}

static void senders_and_receiver(void) {
    enum { SENDERS = 8, PER_SENDER = 1000, CAPACITY = 16 };
    otter_chan *chan = otter_chan_new(CAPACITY);
    CHECK(chan != NULL);

    int failures[SENDERS];
    sender_args args[SENDERS];
    int handles[SENDERS];
    for (int i = 0; i < SENDERS; i++) {
        failures[i] = 0;
        args[i].chan = chan;
        args[i].count = PER_SENDER;
        args[i].failed = &failures[i];
        handles[i] =
            otter_thread_spawn(send_values, &args[i], (long long)sizeof args[i]);
        CHECK(handles[i] != 0);
    }

    long long sum = 0;
    for (int i = 0; i < SENDERS * PER_SENDER; i++) {
        long long value = -1;
        CHECK(otter_chan_recv(chan, &value, (long long)sizeof value) == 1);
        sum += value;
    }
    for (int i = 0; i < SENDERS; i++) {
        CHECK(otter_thread_join(handles[i]) == 0);
        CHECK(failures[i] == 0);
    }

    /* Each sender sends 0 .. PER_SENDER-1, so the total is SENDERS times the
     * triangular number, with no value lost or duplicated. */
    long long expected =
        (long long)SENDERS * (PER_SENDER - 1) * PER_SENDER / 2;
    CHECK(sum == expected);

    otter_chan_close(chan);
    otter_chan_free(chan);
}

/* ---- 3: closing releases a blocked sender ------------------------------ */

typedef struct blocked_sender {
    otter_chan *chan;
    otter_chan *ready;
    long long value;
    char *result;
} blocked_sender;

static void send_then_block(void *raw) {
    blocked_sender *args = (blocked_sender *)raw;
    /* Announce that this thread is about to enter `send`. The ready channel has
     * capacity 1 and is drained exactly once, so this send never blocks. */
    long long one = 1;
    CHECK(otter_chan_send(args->ready, &one, (long long)sizeof one) == 1);
    *args->result =
        otter_chan_send(args->chan, &args->value, (long long)sizeof args->value);
}

static void closing_unblocks_a_sender(void) {
    otter_chan *chan = otter_chan_new(1);
    otter_chan *ready = otter_chan_new(1);
    CHECK(chan != NULL && ready != NULL);

    /* Fill the single slot. The next send cannot complete until a receive or a
     * close, which is what makes the sender block. */
    long long first = 111;
    CHECK(otter_chan_send(chan, &first, (long long)sizeof first) == 1);

    char result = (char)7; /* a value neither send outcome can produce by accident */
    blocked_sender args = {chan, ready, 222, &result};
    int handle =
        otter_thread_spawn(send_then_block, &args, (long long)sizeof args);
    CHECK(handle != 0);

    /* Wait until the sender is committed to the blocking send (it has sent on
     * `ready`), then close. Whichever of "blocked in send" and "about to call
     * send" the sender is in, send must answer 0. */
    long long announced = 0;
    CHECK(otter_chan_recv(ready, &announced, (long long)sizeof announced) == 1);
    otter_chan_close(chan);

    CHECK(otter_thread_join(handle) == 0);
    CHECK(result == 0);

    /* The one buffered slot is still readable after close; the channel only
     * reports "closed" once it is empty. */
    long long drained = 0;
    CHECK(otter_chan_recv(chan, &drained, (long long)sizeof drained) == 1);
    CHECK(drained == first);
    CHECK(otter_chan_recv(chan, &drained, (long long)sizeof drained) == 0);

    otter_chan_close(ready);
    otter_chan_free(ready);
    otter_chan_free(chan);
}

int main(void) {
    four_threads_add_under_a_mutex();
    senders_and_receiver();
    closing_unblocks_a_sender();
    return 0;
}
