/* The one executor: single-threaded and thread-pool modes behind the same
 * `yashy_executor` type.
 *
 * A task is a frame (heap memory the generated code allocated with
 * `yashy_alloc`) plus a poll function. The poll function returns the frame
 * pointer when the task is ready and NULL while it is pending. Frames are
 * freed with `yashy_free` when the executor completes, drops or cancels a
 * task, so the runtime's live-allocation count observes every frame.
 *
 * The executor's own bookkeeping (queue, records) uses `malloc`/`free`
 * directly: it is plumbing, like channels and mutexes in runtime.c, and is
 * not part of the leak report that counts `yashy_alloc` blocks.
 *
 * Single-threaded mode (`threads == 0`): polls run inline on the caller of
 * `yashy_block_on`. Thread-pool mode (`threads > 0`): worker threads poll
 * spawned tasks; `yashy_block_on` still polls the main task inline and
 * then waits for the spawned queue to drain.
 *
 * Task-aware debugging: the executor keeps a registry of live tasks (id,
 * frame, poll fn, state) guarded by the same mutex that guards the queue.
 * `yashy_tasks_dump` walks it so `yashy tasks` and `run --debug` can name
 * every live task, and the frame's fifth header word (the async function's
 * name, written by the factory) is what the dump prints as the function —
 * the textual-IR analogue of the `task.<id>.<fn>` DWARF name a debugger
 * would show.
 *
 * The process-wide default executor takes its thread count from
 * `YASHY_EXECUTOR_THREADS` (unset or non-numeric means single-threaded),
 * which is how the conformance suite runs every program in both modes
 * without rebuilding.
 */

#include "yashy_runtime.h"

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int yashy_sandbox_active(void);

#if defined(_WIN32)
#ifndef _WIN32_WINNT
#define _WIN32_WINNT 0x0600
#endif
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <process.h>
#include <windows.h>
#else
#include <pthread.h>
#include <unistd.h>
#endif

/* The OS abstraction, spelled independently of runtime.c's static helpers:
 * this is a separate translation unit. */
#if defined(_WIN32)
typedef SRWLOCK yax_os_lock;
typedef CONDITION_VARIABLE yax_os_cond;
static int yax_lock_init(yax_os_lock *lock) { InitializeSRWLock(lock); return 1; }
static void yax_lock_destroy(yax_os_lock *lock) { (void)lock; }
static void yax_lock_acquire(yax_os_lock *lock) { AcquireSRWLockExclusive(lock); }
static void yax_lock_release(yax_os_lock *lock) { ReleaseSRWLockExclusive(lock); }
static int yax_cond_init(yax_os_cond *cond) { InitializeConditionVariable(cond); return 1; }
static void yax_cond_destroy(yax_os_cond *cond) { (void)cond; }
static void yax_cond_wait(yax_os_cond *cond, yax_os_lock *lock) {
    (void)SleepConditionVariableSRW(cond, lock, INFINITE, 0);
}
static void yax_cond_signal(yax_os_cond *cond) { WakeConditionVariable(cond); }
static void yax_cond_broadcast(yax_os_cond *cond) { WakeAllConditionVariable(cond); }
typedef unsigned (__stdcall *yax_thread_routine)(void *);
typedef HANDLE yax_thread_handle;
#else
typedef pthread_mutex_t yax_os_lock;
typedef pthread_cond_t yax_os_cond;
static int yax_lock_init(yax_os_lock *lock) { return pthread_mutex_init(lock, NULL) == 0; }
static void yax_lock_destroy(yax_os_lock *lock) { (void)pthread_mutex_destroy(lock); }
static void yax_lock_acquire(yax_os_lock *lock) { (void)pthread_mutex_lock(lock); }
static void yax_lock_release(yax_os_lock *lock) { (void)pthread_mutex_unlock(lock); }
static int yax_cond_init(yax_os_cond *cond) { return pthread_cond_init(cond, NULL) == 0; }
static void yax_cond_destroy(yax_os_cond *cond) { (void)pthread_cond_destroy(cond); }
static void yax_cond_wait(yax_os_cond *cond, yax_os_lock *lock) {
    (void)pthread_cond_wait(cond, lock);
}
static void yax_cond_signal(yax_os_cond *cond) { (void)pthread_cond_signal(cond); }
static void yax_cond_broadcast(yax_os_cond *cond) { (void)pthread_cond_broadcast(cond); }
typedef void *(*yax_thread_routine)(void *);
typedef pthread_t yax_thread_handle;
#endif

/* A live task. `id` is 0 until the record is registered; real ids start at 1
 * and never repeat. `state` is 0 (pending: queued or suspended, not inside a
 * poll) or 1 (running: inside its poll). The `name` is not copied here: it
 * lives in the frame's fifth header word (byte offset 32), written by the
 * async factory, and the dump reads it under the lock while the frame is
 * still live. */
typedef struct yax_task_record {
    yashy_poll_fn poll;
    void *frame;
    long long id;
    int state;
    /* Set for a task that came from `yashy_task_spawn`: it is counted in
     * `outstanding`, and whoever completes or cancels it settles that count
     * exactly once (the registry entry is retired at the same time). */
    int spawned;
} yax_task_record;

/* A structured-concurrency scope: children spawned while it is the executor's
 * active scope, in spawn order. Leaving the scope joins or cancels them in
 * reverse spawn order before execution continues past it. */
typedef struct yax_scope_child {
    yashy_poll_fn poll;
    void *frame;
    /* Set when the child's `await` handed its frame back: the caller read
     * the result and freed the frame, so this slot's `frame` is a dead
     * pointer and the join must skip it without reading anything. */
    int done;
} yax_scope_child;

typedef struct yashy_scope {
    yashy_executor *ex;
    yax_scope_child *children;
    long long count;
    long long capacity;
    struct yashy_scope *parent;
} yashy_scope;

/* A completed task frame the executor holds until its owner reads the
 * result. A `spawn`ed task belongs to the scope that spawned it, so the
 * scope's join is its last owner and frees it; a frame an `await` has
 * finished is handed back by `yashy_task_release`, which retires the mark and
 * frees the frame. Every mark therefore names a live frame, and only a live
 * frame is ever compared against it -- the scope slot's `done` flag, not a
 * mark, is what tells the join that a child's memory is gone. */
typedef struct yax_retained_frame {
    void *frame;
} yax_retained_frame;

struct yashy_executor {
    yax_os_lock lock;
    yax_os_cond has_work;
    yax_os_cond idle;
    yax_task_record *queue; /* FIFO ring buffer */
    long long head;
    long long count;
    long long capacity;
    yax_retained_frame *retained; /* completed frames awaiting their owner */
    long long retained_count;
    long long retained_capacity;
    long long outstanding; /* spawned, not yet completed or cancelled */
    long long completed;   /* spawned tasks that reached ready */
    int shutdown;
    int threads;
    yax_thread_handle *workers;
    yashy_scope *active_scope;
    yax_task_record *registry; /* live tasks, in no particular order */
    long long registry_count;
    long long registry_capacity;
    long long next_id; /* highest id handed out so far */
};

/* --- The live-task registry -------------------------------------------------
 * Everything below this heading assumes `ex->lock` is held; the public
 * helpers acquire it themselves. The mutex is never held across a poll, so a
 * poll that spawns a task (which takes the lock) cannot deadlock. */

static long long yax_registry_index(yashy_executor *ex, void *frame) {
    for (long long i = 0; i < ex->registry_count; i++) {
        if (ex->registry[i].frame == frame) {
            return i;
        }
    }
    return -1;
}

/* Add `frame` to the registry unless it is there already, and answer the id
 * it carries (0 when the grow failed: the task still runs, the dump simply
 * will not list it). */
static long long yax_register_locked(yashy_executor *ex, yashy_poll_fn poll, void *frame) {
    if (yax_registry_index(ex, frame) >= 0) {
        return ex->registry[yax_registry_index(ex, frame)].id;
    }
    if (ex->registry_count == ex->registry_capacity) {
        long long grown = ex->registry_capacity == 0 ? 64 : ex->registry_capacity * 2;
        yax_task_record *next =
            (yax_task_record *)realloc(ex->registry, (size_t)grown * sizeof *next);
        if (next == NULL) {
            return 0;
        }
        ex->registry = next;
        ex->registry_capacity = grown;
    }
    yax_task_record *entry = &ex->registry[ex->registry_count++];
    entry->poll = poll;
    entry->frame = frame;
    entry->id = ++ex->next_id;
    entry->state = 0;
    return entry->id;
}

static void yax_unregister(yashy_executor *ex, void *frame) {
    if (ex == NULL || frame == NULL) {
        return;
    }
    yax_lock_acquire(&ex->lock);
    long long index = yax_registry_index(ex, frame);
    if (index >= 0) {
        ex->registry[index] = ex->registry[ex->registry_count - 1];
        ex->registry_count--;
    }
    yax_lock_release(&ex->lock);
}

static void yax_set_state(yashy_executor *ex, void *frame, int state) {
    if (ex == NULL || frame == NULL) {
        return;
    }
    yax_lock_acquire(&ex->lock);
    long long index = yax_registry_index(ex, frame);
    if (index >= 0) {
        ex->registry[index].state = state;
    }
    yax_lock_release(&ex->lock);
}

/* A task reached ready and the executor now holds its frame until its owner
 * reads the result; see [`yax_retain`]. Declared here because a poll
 * publishes the mark as part of completing the task. */
static void yax_retain(yashy_executor *ex, void *frame);

/* Retire a frame's retained mark; see [`yax_forget`]. Declared here because
 * the join and the cancel path both retire a mark before freeing a frame. */
static void yax_forget(yashy_executor *ex, void *frame);

/* True when a completion mark is standing for this frame; see [`yax_retain`].
 * A retained frame has already reached done, so no driver may poll it again. */
static int yax_is_retained(yashy_executor *ex, void *frame);

/* Wait, briefly, until this frame can be taken exclusively: see
 * `yax_acquire_frame`. */
static int yax_acquire_frame(yashy_executor *ex, void *frame);

/* Poll one task with its state tracked: running across the poll, pending
 * when it suspends, and out of the registry once it is ready. A task that
 * was never registered (a bare `yashy_task_poll`) passes through with only
 * the state writes finding nothing.
 *
 * The running state is also the claim on the frame. A scope's join polls
 * its children directly while a pool worker may hold the same frame from
 * the queue, and two threads inside one state machine both read the resume
 * index as live and run its body twice -- a cancelled worker's `defer`s
 * would print twice. A frame already claimed answers NULL, which every
 * caller reads as "still pending" and retries once the other driver is
 * done.
 *
 * `from_queue` says the caller dequeued this task's record, so it owns the
 * next poll: the registry holds state 2 ("checked out") from the dequeue
 * until the claim is taken here. That is the window a free path must not
 * cross -- without it a record could be checked out by a worker and the
 * frame freed before the worker's first poll.
 *
 * `retain_on_ready` publishes the completion mark *before* the claim is
 * released, so no other driver can see a finished frame and conclude that
 * nobody has marked it yet. Callers that own the frame and free it
 * themselves (the join, `yashy_task_poll`) pass 0. */
static void *yax_poll_tracked(yashy_executor *ex, yashy_poll_fn poll, void *frame,
                              int retain_on_ready, int from_queue) {
    if (ex != NULL && frame != NULL && yax_is_retained(ex, frame)) {
        /* Another driver completed this task and published its mark. Polling
         * it again would run the body a second time: the generated poll
         * returns at once on a done resume index, but the executor must not
         * depend on that, and a hand-made frame (the C suite) does not do it.
         * Answer "ready" instead of polling. */
        return frame;
    }
    if (ex != NULL && frame != NULL) {
        yax_lock_acquire(&ex->lock);
        long long index = yax_registry_index(ex, frame);
        if (index >= 0) {
            int state = ex->registry[index].state;
            if (from_queue) {
                if (state == 1) {
                    /* Another driver is inside; this record is stale. */
                    yax_lock_release(&ex->lock);
                    return NULL;
                }
            } else if (state != 0) {
                yax_lock_release(&ex->lock);
                return NULL;
            }
            ex->registry[index].state = 1;
        }
        yax_lock_release(&ex->lock);
    }
    void *ready = poll == NULL ? NULL : poll(frame);
    if (ready != NULL) {
        if (retain_on_ready) {
            yax_retain(ex, frame);
        }
        yax_unregister(ex, frame);
    } else if (!from_queue) {
        /* A queue poller keeps its claim until it has requeued the record:
         * `yax_requeue` clears it in the same lock section that publishes the
         * record, so a free path never sees an unclaimed frame between the
         * two. A driver that will not requeue (the join, `task_poll`) frees
         * the claim here. */
        yax_set_state(ex, frame, 0);
    }
    return ready;
}

static int yax_queue_reserve(yashy_executor *ex, long long needed) {
    if (ex->capacity >= needed) {
        return 1;
    }
    long long grown = ex->capacity == 0 ? 64 : ex->capacity * 2;
    while (grown < needed) {
        grown *= 2;
    }
    yax_task_record *next = (yax_task_record *)malloc((size_t)grown * sizeof *next);
    if (next == NULL) {
        return 0;
    }
    for (long long i = 0; i < ex->count; i++) {
        next[i] = ex->queue[(ex->head + i) % ex->capacity];
    }
    free(ex->queue);
    ex->queue = next;
    ex->head = 0;
    ex->capacity = grown;
    return 1;
}

/* Requeue a task that is still pending. Unlike a spawn, this is the same
 * task again, so it must not count as a new outstanding one. */
static int yax_requeue(yashy_executor *ex, yashy_poll_fn poll, void *frame) {
    yax_lock_acquire(&ex->lock);
    if (ex->shutdown) {
        yax_lock_release(&ex->lock);
        return -1;
    }
    if (!yax_queue_reserve(ex, ex->count + 1)) {
        yax_lock_release(&ex->lock);
        return -1;
    }
    long long tail = (ex->head + ex->count) % ex->capacity;
    ex->queue[tail].poll = poll;
    ex->queue[tail].frame = frame;
    ex->queue[tail].id = 0;
    ex->queue[tail].state = 0;
    ex->count++;
    /* The record is back and the frame is claimable in one step: clearing the
     * claim first would open a window in which a free path takes the frame
     * for good while this thread is about to enqueue it. */
    long long index = yax_registry_index(ex, frame);
    if (index >= 0) {
        ex->registry[index].state = 0;
    }
    yax_cond_signal(&ex->has_work);
    yax_lock_release(&ex->lock);
    return 0;
}

static int yax_enqueue(yashy_executor *ex, yashy_poll_fn poll, void *frame) {
    yax_lock_acquire(&ex->lock);
    if (ex->shutdown) {
        yax_lock_release(&ex->lock);
        return -1;
    }
    if (!yax_queue_reserve(ex, ex->count + 1)) {
        yax_lock_release(&ex->lock);
        return -1;
    }
    long long tail = (ex->head + ex->count) % ex->capacity;
    ex->queue[tail].poll = poll;
    ex->queue[tail].frame = frame;
    ex->queue[tail].id = yax_register_locked(ex, poll, frame);
    ex->queue[tail].state = 0;
    ex->count++;
    ex->outstanding++;
    yax_cond_signal(&ex->has_work);
    yax_lock_release(&ex->lock);
    return 0;
}

/* Dequeue one task and check its frame out of the executor: answers 1 with
 * the record filled, 0 when the queue is empty, and 2 when the head record
 * names a frame another driver has claimed (the caller should retry later --
 * the record stays at the head so it is not lost). Checking the frame out
 * here, under the same lock as the dequeue, is what stops a free path from
 * taking a frame between a worker's dequeue and its first poll. */
static int yax_dequeue(yashy_executor *ex, yax_task_record *task_out) {
    yax_lock_acquire(&ex->lock);
    if (ex->count == 0) {
        yax_lock_release(&ex->lock);
        return 0;
    }
    void *frame = ex->queue[ex->head].frame;
    long long index = yax_registry_index(ex, frame);
    if (index >= 0 && ex->registry[index].state != 0) {
        yax_lock_release(&ex->lock);
        return 2;
    }
    *task_out = ex->queue[ex->head];
    ex->head = (ex->head + 1) % ex->capacity;
    ex->count--;
    if (index >= 0) {
        ex->registry[index].state = 2;
    }
    yax_lock_release(&ex->lock);
    return 1;
}

static void yax_finish_one(yashy_executor *ex) {
    yax_lock_acquire(&ex->lock);
    ex->outstanding--;
    if (ex->outstanding == 0) {
        yax_cond_broadcast(&ex->idle);
    }
    yax_lock_release(&ex->lock);
}

/* A task reached ready. The executor does not free its frame: whoever owns
 * the task does, after reading the result -- an `await` hands it back
 * (`yashy_task_release`) and, when nobody awaits the task, the scope's join
 * frees it. Until then the frame is retained here and freed by
 * `yashy_executor_free` when the executor itself goes away. A frame that is
 * already retained keeps its single mark: completion happens once. */
static void yax_retain(yashy_executor *ex, void *frame) {
    if (ex == NULL || frame == NULL) {
        return;
    }
    yax_lock_acquire(&ex->lock);
    for (long long i = 0; i < ex->retained_count; i++) {
        if (ex->retained[i].frame == frame) {
            yax_lock_release(&ex->lock);
            return;
        }
    }
    if (ex->retained_count == ex->retained_capacity) {
        long long grown = ex->retained_capacity == 0 ? 64 : ex->retained_capacity * 2;
        yax_retained_frame *next = (yax_retained_frame *)realloc(
            ex->retained, (size_t)grown * sizeof *next);
        if (next != NULL) {
            ex->retained = next;
            ex->retained_capacity = grown;
        } else {
            /* Without a retained slot the frame would leak from the
             * executor's view; free it now rather than lose track of it. */
            yashy_free(frame);
            yax_lock_release(&ex->lock);
            return;
        }
    }
    ex->retained[ex->retained_count].frame = frame;
    ex->retained_count++;
    yax_lock_release(&ex->lock);
}

#if defined(_WIN32)
static unsigned __stdcall yax_worker_loop(void *raw)
#else
static void *yax_worker_loop(void *raw)
#endif
{
    yashy_executor *ex = (yashy_executor *)raw;
    for (;;) {
        yax_task_record task;
        yax_lock_acquire(&ex->lock);
        while (ex->count == 0 && !ex->shutdown) {
            yax_cond_wait(&ex->has_work, &ex->lock);
        }
        int stopping = ex->shutdown && ex->count == 0;
        yax_lock_release(&ex->lock);
        if (stopping) {
#if defined(_WIN32)
            return 0;
#else
            return NULL;
#endif
        }
        int got = yax_dequeue(ex, &task);
        if (got == 2) {
            /* The head record names a frame some other driver has claimed;
             * it stays queued until that driver finishes. */
            yashy_sleep_ms(1);
            continue;
        }
        if (got == 0) {
            continue;
        }
        void *ready = yax_poll_tracked(ex, task.poll, task.frame, 1, 1);
        if (ready != NULL) {
            yax_lock_acquire(&ex->lock);
            ex->completed++;
            yax_lock_release(&ex->lock);
        } else {
            /* Still pending: requeue at the tail (FIFO, no work stealing). */
            yax_requeue(ex, task.poll, task.frame);
        }
        if (ready != NULL) {
            yax_finish_one(ex);
        }
    }
}

yashy_executor *yashy_executor_new(int threads) {
    yashy_executor *ex = (yashy_executor *)malloc(sizeof *ex);
    if (ex == NULL) {
        return NULL;
    }
    if (!yax_lock_init(&ex->lock)) {
        free(ex);
        return NULL;
    }
    if (!yax_cond_init(&ex->has_work)) {
        yax_lock_destroy(&ex->lock);
        free(ex);
        return NULL;
    }
    if (!yax_cond_init(&ex->idle)) {
        yax_cond_destroy(&ex->has_work);
        yax_lock_destroy(&ex->lock);
        free(ex);
        return NULL;
    }
    ex->queue = NULL;
    ex->retained = NULL;
    ex->retained_count = 0;
    ex->retained_capacity = 0;
    ex->head = 0;
    ex->count = 0;
    ex->capacity = 0;
    ex->outstanding = 0;
    ex->completed = 0;
    ex->shutdown = 0;
    ex->threads = threads;
    ex->workers = NULL;
    ex->active_scope = NULL;
    ex->registry = NULL;
    ex->registry_count = 0;
    ex->registry_capacity = 0;
    ex->next_id = 0;
    if (threads > 0) {
        ex->workers = (yax_thread_handle *)malloc((size_t)threads * sizeof *ex->workers);
        if (ex->workers == NULL) {
            yax_cond_destroy(&ex->idle);
            yax_cond_destroy(&ex->has_work);
            yax_lock_destroy(&ex->lock);
            free(ex);
            return NULL;
        }
        for (int i = 0; i < threads; i++) {
#if defined(_WIN32)
            unsigned thread_id = 0;
            uintptr_t native = _beginthreadex(
                NULL, 0, yax_worker_loop, ex, 0, &thread_id);
            if (native == 0) {
                ex->shutdown = 1;
                yax_cond_broadcast(&ex->has_work);
                for (int j = 0; j < i; j++) {
                    (void)WaitForSingleObject(ex->workers[j], INFINITE);
                    (void)CloseHandle(ex->workers[j]);
                }
                free(ex->workers);
                yax_cond_destroy(&ex->idle);
                yax_cond_destroy(&ex->has_work);
                yax_lock_destroy(&ex->lock);
                free(ex);
                return NULL;
            }
            ex->workers[i] = (HANDLE)native;
#else
            if (pthread_create(&ex->workers[i], NULL, yax_worker_loop, ex) != 0) {
                ex->shutdown = 1;
                yax_cond_broadcast(&ex->has_work);
                for (int j = 0; j < i; j++) {
                    (void)pthread_join(ex->workers[j], NULL);
                }
                free(ex->workers);
                yax_cond_destroy(&ex->idle);
                yax_cond_destroy(&ex->has_work);
                yax_lock_destroy(&ex->lock);
                free(ex);
                return NULL;
            }
#endif
        }
    }
    return ex;
}

void yashy_executor_free(yashy_executor *ex) {
    if (ex == NULL) {
        return;
    }
    yax_lock_acquire(&ex->lock);
    ex->shutdown = 1;
    /* Cancel every task still queued: a dropped executor frees its frames. */
    while (ex->count > 0) {
        yax_task_record task = ex->queue[ex->head];
        ex->head = (ex->head + 1) % ex->capacity;
        ex->count--;
        yashy_free(task.frame);
        ex->outstanding--;
    }
    yax_cond_broadcast(&ex->has_work);
    yax_lock_release(&ex->lock);
    for (int i = 0; i < ex->threads; i++) {
#if defined(_WIN32)
        (void)WaitForSingleObject(ex->workers[i], INFINITE);
        (void)CloseHandle(ex->workers[i]);
#else
        (void)pthread_join(ex->workers[i], NULL);
#endif
    }
    /* Tasks a worker had checked out and requeued after shutdown stay queued
     * behind it; free anything left. */
    yax_lock_acquire(&ex->lock);
    while (ex->count > 0) {
        yax_task_record task = ex->queue[ex->head];
        ex->head = (ex->head + 1) % ex->capacity;
        ex->count--;
        yashy_free(task.frame);
    }
    for (long long i = 0; i < ex->retained_count; i++) {
        /* A retained frame has no owner left: whoever drove the task home
         * either retires the mark and frees the frame (`yashy_task_release`,
         * the scope's join) or never does. Every mark names a live frame, so
         * freeing it here cannot double-free. */
        yashy_free(ex->retained[i].frame);
    }
    yax_lock_release(&ex->lock);
    free(ex->workers);
    free(ex->queue);
    free(ex->retained);
    free(ex->registry);
    yax_cond_destroy(&ex->idle);
    yax_cond_destroy(&ex->has_work);
    yax_lock_destroy(&ex->lock);
    free(ex);
}

/* The process-wide default executor, created lazily. Generated code drives
 * every task through it, so a program needs no explicit executor value. */
static yax_os_lock yax_default_lock =
#if defined(_WIN32)
    SRWLOCK_INIT;
#else
    PTHREAD_MUTEX_INITIALIZER;
#endif
static yashy_executor *yax_default = NULL;

yashy_executor *yashy_executor_default(void) {
    yax_lock_acquire(&yax_default_lock);
    if (yax_default == NULL) {
        /* The mode switch the conformance suite uses: `YASHY_EXECUTOR_THREADS`
         * names the worker count of the default executor, so the same binary
         * runs single-threaded (unset, 0 or non-numeric) or on a pool without
         * a rebuild. */
        int threads = 0;
        const char *requested = getenv("YASHY_EXECUTOR_THREADS");
        if (requested != NULL && requested[0] != '\0') {
            int wanted = atoi(requested);
            if (wanted > 0) {
                threads = wanted > 64 ? 64 : wanted;
            }
        }
        yax_default = yashy_executor_new(threads);
    }
    yashy_executor *ex = yax_default;
    yax_lock_release(&yax_default_lock);
    return ex;
}

/* A snapshot of one registry entry, copied under the lock so the dump can
 * print after it. */
typedef struct yax_task_snapshot {
    long long id;
    const char *name;
    int resume;
    int state;
} yax_task_snapshot;

void yashy_tasks_dump(void) {
    /* Snapshot the default executor without creating it: a program that
     * never used the executor has no live tasks. */
    yax_lock_acquire(&yax_default_lock);
    yashy_executor *ex = yax_default;
    yax_lock_release(&yax_default_lock);
    if (ex == NULL) {
        return;
    }
    yax_task_snapshot *snap = NULL;
    long long count = 0;
    yax_lock_acquire(&ex->lock);
    if (ex->registry_count > 0) {
        snap = (yax_task_snapshot *)malloc(
            (size_t)ex->registry_count * sizeof *snap);
        if (snap == NULL) {
            yax_lock_release(&ex->lock);
            return;
        }
        for (long long i = 0; i < ex->registry_count; i++) {
            const char *name =
                *(const char *const *)((const unsigned char *)ex->registry[i].frame + 32);
            /* Frames the code generator wrote: their name word holds a NUL-
             * terminated string in a global, never a small integer. A
             * hand-made frame (the C tests zero theirs) carries no name, and
             * a first field that happens to look like a small integer must
             * not be read as one either. */
            if ((uintptr_t)name <= 4096) {
                name = NULL;
            }
            snap[count].id = ex->registry[i].id;
            snap[count].name = name;
            snap[count].resume = *(const int *)ex->registry[i].frame;
            snap[count].state = ex->registry[i].state;
            count++;
        }
    }
    yax_lock_release(&ex->lock);
    if (snap == NULL) {
        return;
    }
    /* The registry's order is registration order except that unregistering
     * swaps with the last entry, so sort by id: the dump must be stable. */
    for (long long i = 1; i < count; i++) {
        yax_task_snapshot key = snap[i];
        long long j = i - 1;
        while (j >= 0 && snap[j].id > key.id) {
            snap[j + 1] = snap[j];
            j--;
        }
        snap[j + 1] = key;
    }
    fprintf(stderr, "tasks: %lld\n", count);
    for (long long i = 0; i < count; i++) {
        fprintf(stderr, "task %lld %s resume=%d state=%s\n", snap[i].id,
                snap[i].name != NULL ? snap[i].name : "<unknown>", snap[i].resume,
                snap[i].state == 1 ? "running" : "pending");
    }
    fflush(stderr);
    free(snap);
}

int yashy_task_spawn(yashy_executor *ex, yashy_poll_fn poll, void *frame) {
    if (ex == NULL || poll == NULL || frame == NULL) {
        return -1;
    }
    /* The sandbox's spawn capability is off by default. */
    if (yashy_sandbox_active() != NULL) {
        const char *spawn_allowed = getenv("YASHY_SANDBOX_SPAWN");
        if (spawn_allowed == NULL || spawn_allowed[0] != '1') {
            return -1;
        }
    }
    /* A frame's mark is retired by the scope's join (or by cancel/drop), so
     * an entry still standing for this address would belong to a frame that
     * has been freed without a join ever reaching it. Nothing can tell such
     * an entry from a live one -- the marks are compared by pointer only --
     * so the spawn path leaves them alone: retiring one here would strip the
     * only protection a freed child slot has against being read again. The
     * one case this cannot cover (a frame reallocated at a freed child's
     * address before that child's join retires its mark) is recorded in the
     * phase-13 ledger as an open finding. */
    int rc = yax_enqueue(ex, poll, frame);
    /* A spawn while a scope is active belongs to that scope: register it so
     * leaving the scope joins or cancels it in reverse spawn order. */
    if (rc == 0 && ex->active_scope != NULL) {
        yashy_scope *scope = ex->active_scope;
        if (scope->count == scope->capacity) {
            long long grown = scope->capacity == 0 ? 16 : scope->capacity * 2;
            yax_scope_child *next =
                (yax_scope_child *)realloc(scope->children, (size_t)grown * sizeof *next);
            if (next != NULL) {
                scope->children = next;
                scope->capacity = grown;
            }
        }
        if (scope->count < scope->capacity) {
            scope->children[scope->count].poll = poll;
            scope->children[scope->count].frame = frame;
            scope->children[scope->count].done = 0;
            scope->count++;
        }
    }
    return rc;
}

yashy_scope *yashy_scope_create(yashy_executor *ex) {
    if (ex == NULL) {
        return NULL;
    }
    yashy_scope *scope = (yashy_scope *)malloc(sizeof *scope);
    if (scope == NULL) {
        return NULL;
    }
    scope->ex = ex;
    scope->children = NULL;
    scope->count = 0;
    scope->capacity = 0;
    yax_lock_acquire(&ex->lock);
    scope->parent = ex->active_scope;
    ex->active_scope = scope;
    yax_lock_release(&ex->lock);
    return scope;
}

void yashy_scope_free(yashy_scope *scope) {
    if (scope == NULL) {
        return;
    }
    yax_lock_acquire(&scope->ex->lock);
    if (scope->ex->active_scope == scope) {
        scope->ex->active_scope = scope->parent;
    }
    yax_lock_release(&scope->ex->lock);
    free(scope->children);
    free(scope);
}

/* True when a completion mark is standing for this frame. A retained frame
 * has already reached done, so no driver may poll it again; see
 * `yax_poll_tracked`. */
static int yax_is_retained(yashy_executor *ex, void *frame) {
    if (ex == NULL || frame == NULL) {
        return 0;
    }
    int found = 0;
    yax_lock_acquire(&ex->lock);
    for (long long i = 0; i < ex->retained_count; i++) {
        if (ex->retained[i].frame == frame) {
            found = 1;
            break;
        }
    }
    yax_lock_release(&ex->lock);
    return found;
}

/* Retire every completion mark for a frame: the caller is now its owner and
 * is about to free it. The marks are a set, so every entry for the frame
 * goes. */
static void yax_forget(yashy_executor *ex, void *frame) {
    if (ex == NULL || frame == NULL) {
        return;
    }
    yax_lock_acquire(&ex->lock);
    for (long long i = 0; i < ex->retained_count; i++) {
        if (ex->retained[i].frame == frame) {
            ex->retained[i] = ex->retained[ex->retained_count - 1];
            ex->retained_count--;
            i--;
        }
    }
    yax_lock_release(&ex->lock);
}

/* Remove a task's queued record without freeing its frame: used when a
 * driver takes the task over inline — the scope joining a child home, or
 * `block_on` starting an `await` — so a later executor drain must not poll
 * it a second time. Answers 1 when the record was here and 0 when someone
 * else already held it (the frame stays live either way; the caller that
 * ends up owning it frees it and retires any mark). */
static int yax_unqueue(yashy_executor *ex, void *frame) {
    if (ex == NULL || frame == NULL) {
        return 0;
    }
    int removed = 0;
    yax_lock_acquire(&ex->lock);
    for (long long i = 0; i < ex->count; i++) {
        long long index = (ex->head + i) % ex->capacity;
        if (ex->queue[index].frame == frame) {
            ex->queue[index] =
                ex->queue[(ex->head + ex->count - 1) % ex->capacity];
            ex->count--;
            ex->outstanding--;
            removed = 1;
            if (ex->outstanding == 0) {
                yax_cond_broadcast(&ex->idle);
            }
            break;
        }
    }
    yax_lock_release(&ex->lock);
    return removed;
}

/* Join or cancel every child in reverse spawn order. A child that reached
 * ready through another driver is not polled again. Every other child is
 * polled until it is ready; a child that stays pending is cancelled (its
 * frame is freed, which is also the cancel path a `defer` observes through
 * the one final poll that runs its pending segment).
 *
 * Two answers are possible for a child that another driver finished first:
 * this loop drove it home, or a queue driver / an `await` did. Both leave a
 * live frame (an `await` marks the slot done instead, so its frame is freed
 * there and this loop skips it), so the join frees it either way. That is the
 * fix for the leak this gate found: a discarded `spawn` handle completed by a
 * pool worker stayed in the retained list forever, because the join read
 * "retained" as "awaited" and nothing else could free it.
 */
void yashy_scope_join_or_cancel(yashy_scope *scope) {
    if (scope == NULL) {
        return;
    }
    for (long long i = scope->count - 1; i >= 0; i--) {
        yax_scope_child child = scope->children[i];
        if (child.poll == NULL || child.frame == NULL) {
            continue;
        }
        if (child.done) {
            /* The child's `await` read the result and handed the frame back;
             * it was freed there, so this slot holds a dead pointer. */
            continue;
        }
        /* Header word 0 is the resume index: -1 means done. A child another
         * driver finished is not polled again -- it is only read, which is
         * safe because a scope child's frame is live until this loop frees
         * it. The index is re-read on every attempt: a pool worker may drive
         * this very child to completion while the loop waits. */
        int ready = 0;
        for (int attempt = 0; attempt < 1000 && *(int *)child.frame != -1; attempt++) {
            if (yax_poll_tracked(scope->ex, child.poll, child.frame, 0, 0) != NULL) {
                ready = 1;
                break;
            }
            yashy_sleep_ms(1);
        }
        if (!ready && *(int *)child.frame != -1) {
            /* Still pending after the bounded wait: cancel it. That path runs
             * the task's pendings `defer`s once, takes its mark back, and
             * frees the frame. */
            yashy_task_cancel(scope->ex, child.poll, child.frame);
            continue;
        }
        /* This loop drove the child home, or another driver did. Either way
         * the frame is ours now: wait out any claim, remove the queue record
         * so no driver can dequeue it after the free, retire its mark and
         * free it. */
        if (yax_acquire_frame(scope->ex, child.frame)) {
            yax_unqueue(scope->ex, child.frame);
            yax_unregister(scope->ex, child.frame);
            yax_forget(scope->ex, child.frame);
            yashy_free(child.frame);
        }
    }
    scope->count = 0;
}

int yashy_task_poll(yashy_executor *ex, yashy_poll_fn poll, void *frame) {
    if (poll == NULL || frame == NULL) {
        return -1;
    }
    return yax_poll_tracked(ex, poll, frame, 0, 0) != NULL ? 1 : 0;
}

/* Poll one task; on a pending answer, drive the spawned queue once so
 * `spawn`ed children make progress even while the main task waits. */
static int yax_drive_once(yashy_executor *ex, yashy_poll_fn poll, void *frame) {
    int ready =
        poll != NULL && frame != NULL && yax_poll_tracked(ex, poll, frame, 0, 0) != NULL;
    if (ready) {
        return 1;
    }
    yax_task_record task;
    if (yax_dequeue(ex, &task) == 1) {
        void *child_ready = yax_poll_tracked(ex, task.poll, task.frame, 1, 1);
        if (child_ready != NULL) {
            yax_lock_acquire(&ex->lock);
            ex->completed++;
            yax_lock_release(&ex->lock);
            yax_finish_one(ex);
        } else {
            yax_requeue(ex, task.poll, task.frame);
        }
    }
    return 0;
}

int yashy_block_on(yashy_executor *ex, yashy_poll_fn poll, void *frame) {
    if (ex == NULL || poll == NULL || frame == NULL) {
        return -1;
    }
    /* An awaited `spawn`ed task still carries its queue record. This call
     * now drives the frame inline, so hand the record off first — the same
     * hand-off `yashy_scope_join_or_cancel` does when the scope itself
     * drove a child home. Without it, a drain nested inside this very poll
     * (the `await` in the task being driven) dequeues the record, meets the
     * claim this call holds, and requeues it forever: the two drivers
     * wait on each other and the program never terminates. */
    yax_unqueue(ex, frame);
    /* The task being driven is live too, so the dump can name it while it
     * runs or suspends. */
    yax_lock_acquire(&ex->lock);
    yax_register_locked(ex, poll, frame);
    yax_lock_release(&ex->lock);
    const char *tasks_dumped = getenv("YASHY_TASKS_DUMP");
    long long pending_spins = 0;
    for (;;) {
        if (yax_drive_once(ex, poll, frame)) {
            break;
        }
        /* With a pool, workers may finish the queue; without one, requeue
         * churn is the only progress possible, so poll again. A tiny sleep
         * keeps a genuinely pending task (a channel wait inside a poll)
         * from spinning hot. */
        yashy_sleep_ms(1);
        /* `yashy tasks` on a hung program: while the main task is still
         * pending after about a thousand 1-ms spins, print the live tasks
         * so waiting is visible instead of silent. */
        if (tasks_dumped != NULL && tasks_dumped[0] != '\0' && tasks_dumped[0] != '0') {
            pending_spins++;
            if (pending_spins % 1000 == 0) {
                yashy_tasks_dump();
            }
        }
    }
    /* No completion mark is published for the frame this call drove: the
     * caller receives it and hands it back through `yashy_task_release`,
     * which retires the mark and frees the frame. A mark would be redundant
     * here, and a mark that outlives its frame is the one thing this design
     * cannot represent -- the marks are compared by pointer only. */
    /* The main task completed before `block_on` returns, and so did every
     * task spawned from it: drain what is still queued. Every ready task is
     * completed here (the loop is unbounded for ready tasks). A task that is
     * still pending is requeued once and the drain stops -- only a worker or
     * a later `block_on` can complete it, so spinning cannot help. */
    long long claimed_spins = 0;
    for (;;) {
        yax_task_record task;
        int got = yax_dequeue(ex, &task);
        if (got == 0) {
            break; /* queue empty */
        }
        if (got == 2) {
            /* Another driver has the head record's frame checked out; it
             * will either finish or requeue it. Let it, but do not spin
             * forever on a driver that never lets go. */
            if (++claimed_spins > 1000) {
                break;
            }
            yashy_sleep_ms(1);
            continue;
        }
        void *ready = yax_poll_tracked(ex, task.poll, task.frame, 1, 1);
        if (ready != NULL) {
            yax_lock_acquire(&ex->lock);
            ex->completed++;
            yax_lock_release(&ex->lock);
            yax_finish_one(ex);
            continue;
        }
        /* Still pending: put it back and stop draining. */
        yax_requeue(ex, task.poll, task.frame);
        break;
    }
    yax_lock_acquire(&ex->lock);
    long long outstanding = ex->outstanding;
    yax_lock_release(&ex->lock);
    if (ex->threads > 0 && outstanding > 0) {
        yax_lock_acquire(&ex->lock);
        while (ex->outstanding > 0) {
            yax_cond_wait(&ex->idle, &ex->lock);
        }
        yax_lock_release(&ex->lock);
    }
    yax_unregister(ex, frame);
    if (tasks_dumped != NULL && tasks_dumped[0] != '\0' && tasks_dumped[0] != '0') {
        yashy_tasks_dump();
    }
    return 0;
}

/* Wait, briefly, until no driver holds this frame. A poll holds the claim for
 * one segment, and a queue poller holds it from its dequeue from the queue
 * until it either completes the task or requeues it -- state 2 is that
 * checkout. A frame freed under a worker would be read after the claim is
 * released. The wait is bounded because a scope has to make progress even if
 * a driver wedges. Returns 1 when the frame is unclaimed. */
static int yax_wait_unclaimed(yashy_executor *ex, void *frame) {
    if (ex == NULL || frame == NULL) {
        return 1;
    }
    for (int attempt = 0; attempt < 100; attempt++) {
        yax_lock_acquire(&ex->lock);
        long long index = yax_registry_index(ex, frame);
        int claimed = index >= 0 && ex->registry[index].state != 0;
        yax_lock_release(&ex->lock);
        if (!claimed) {
            return 1;
        }
        yashy_sleep_ms(1);
    }
    return 0;
}

/* Take exclusive ownership of a frame: wait until no driver is inside it,
 * then answer 1 so the caller may unregister, retire the mark and free it. */
static int yax_acquire_frame(yashy_executor *ex, void *frame) {
    return yax_wait_unclaimed(ex, frame);
}

/* Note that a finished child's frame has been handed back. Whether a frame
 * belongs to a scope is answered by walking the active scope and its parents
 * -- an `await` can sit inside a nested scope and wait on a child of an outer
 * one -- and the slot it belongs to is marked done there, so the join skips
 * the slot instead of reading a frame the allocator has already taken back.
 * Returns 1 when a live scope owned the frame. */
static int yax_scope_note_released(yashy_executor *ex, void *frame) {
    if (ex == NULL || frame == NULL) {
        return 0;
    }
    yax_lock_acquire(&ex->lock);
    yashy_scope *scope = ex->active_scope;
    yax_lock_release(&ex->lock);
    for (; scope != NULL; scope = scope->parent) {
        for (long long i = 0; i < scope->count; i++) {
            if (scope->children[i].frame == frame && !scope->children[i].done) {
                scope->children[i].done = 1;
                return 1;
            }
        }
    }
    return 0;
}

/* Hand a finished child frame back after its result has been read. A spawned
 * task's frame belongs to the scope that spawned it: that scope's join is
 * told the frame is gone (so it never reads it) and the frame is freed here,
 * which keeps a loop of awaits from holding every completed frame until the
 * scope ends. A frame of a directly called `async fn` has no slot to mark and
 * is freed the same way.
 *
 * This is what lets the join read its children safely: a slot is either
 * marked done (and never touched) or still holds a live frame. Nothing keys
 * on the frame's address outliving the frame. */
void yashy_task_release(yashy_executor *ex, void *frame) {
    if (frame == NULL) {
        return;
    }
    yax_scope_note_released(ex, frame);
    /* A worker may have requeued the record after this `await` unqueued it
     * (it dequeued first, saw the task pending, and put it back), and it may
     * be inside one segment right now. Drop the record and wait the claim
     * out before the frame goes away, or the queue would hand a freed frame
     * to the next driver. */
    yax_wait_unclaimed(ex, frame);
    yax_unqueue(ex, frame);
    yax_unregister(ex, frame);
    yax_forget(ex, frame);
    yashy_free(frame);
}

void yashy_task_cancel(yashy_executor *ex, yashy_poll_fn poll, void *frame) {
    if (frame == NULL) {
        return;
    }
    /* A frame another thread is inside must not be freed under it. */
    yax_wait_unclaimed(ex, frame);
    yax_unregister(ex, frame);
    if (ex != NULL) {
        yax_lock_acquire(&ex->lock);
        for (long long i = 0; i < ex->count; i++) {
            long long index = (ex->head + i) % ex->capacity;
            if (ex->queue[index].frame == frame) {
                /* Remove by swapping with the last queued record. Frames are
                 * unique pointers, so a second match is a second task. */
                ex->queue[index] = ex->queue[(ex->head + ex->count - 1) % ex->capacity];
                ex->count--;
                ex->outstanding--;
                if (ex->outstanding == 0) {
                    yax_cond_broadcast(&ex->idle);
                }
                yax_lock_release(&ex->lock);
                yax_forget(ex, frame);
                yashy_free(frame);
                return;
            }
        }
        yax_lock_release(&ex->lock);
    }
    (void)poll;
    yax_forget(ex, frame);
    yashy_free(frame);
}

void yashy_task_drop(void *frame) {
    if (frame == NULL) {
        return;
    }
    /* A dropped frame is no longer live; take it out of the default
     * executor's registry and its retained marks without creating the
     * executor just to do so. */
    yax_lock_acquire(&yax_default_lock);
    yashy_executor *ex = yax_default;
    yax_lock_release(&yax_default_lock);
    yax_unregister(ex, frame);
    yax_forget(ex, frame);
    yashy_free(frame);
}

long long yashy_executor_completed(yashy_executor *ex) {
    if (ex == NULL) {
        return 0;
    }
    yax_lock_acquire(&ex->lock);
    long long completed = ex->completed;
    yax_lock_release(&ex->lock);
    return completed;
}
