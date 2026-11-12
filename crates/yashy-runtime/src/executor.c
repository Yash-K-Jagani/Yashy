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
} yax_task_record;

/* A structured-concurrency scope: children spawned while it is the executor's
 * active scope, in spawn order. Leaving the scope joins or cancels them in
 * reverse spawn order before execution continues past it. */
typedef struct yax_scope_child {
    yashy_poll_fn poll;
    void *frame;
} yax_scope_child;

typedef struct yashy_scope {
    yashy_executor *ex;
    yax_scope_child *children;
    long long count;
    long long capacity;
    struct yashy_scope *parent;
} yashy_scope;

struct yashy_executor {
    yax_os_lock lock;
    yax_os_cond has_work;
    yax_os_cond idle;
    yax_task_record *queue; /* FIFO ring buffer */
    long long head;
    long long count;
    long long capacity;
    void **retained; /* completed frames awaiting their owner */
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
 * done. */
static void *yax_poll_tracked(yashy_executor *ex, yashy_poll_fn poll, void *frame) {
    if (ex != NULL && frame != NULL) {
        yax_lock_acquire(&ex->lock);
        long long index = yax_registry_index(ex, frame);
        if (index >= 0) {
            if (ex->registry[index].state == 1) {
                yax_lock_release(&ex->lock);
                return NULL;
            }
            ex->registry[index].state = 1;
        }
        yax_lock_release(&ex->lock);
    }
    void *ready = poll == NULL ? NULL : poll(frame);
    if (ready != NULL) {
        yax_unregister(ex, frame);
    } else {
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

/* Dequeue one task, or return 0 when the queue is empty. */
static int yax_dequeue(yashy_executor *ex, yax_task_record *task_out) {
    yax_lock_acquire(&ex->lock);
    if (ex->count == 0) {
        yax_lock_release(&ex->lock);
        return 0;
    }
    *task_out = ex->queue[ex->head];
    ex->head = (ex->head + 1) % ex->capacity;
    ex->count--;
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

/* A spawned task reached ready. The executor does not free its frame: the
 * owner (usually the task that awaits it) frees it after reading the
 * result. Until then the frame is retained here and freed by
 * `yashy_executor_free` when the executor itself goes away. */
static void yax_retain(yashy_executor *ex, void *frame) {
    yax_lock_acquire(&ex->lock);
    if (ex->retained_count == ex->retained_capacity) {
        long long grown = ex->retained_capacity == 0 ? 64 : ex->retained_capacity * 2;
        void **next = (void **)realloc(ex->retained, (size_t)grown * sizeof *next);
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
    ex->retained[ex->retained_count++] = frame;
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
        int got = 0;
        yax_lock_acquire(&ex->lock);
        while (ex->count == 0 && !ex->shutdown) {
            yax_cond_wait(&ex->has_work, &ex->lock);
        }
        if (ex->count > 0) {
            task = ex->queue[ex->head];
            ex->head = (ex->head + 1) % ex->capacity;
            ex->count--;
            got = 1;
        }
        int stopping = ex->shutdown && ex->count == 0;
        yax_lock_release(&ex->lock);
        if (!got) {
            if (stopping) {
#if defined(_WIN32)
                return 0;
#else
                return NULL;
#endif
            }
            continue;
        }
        void *ready = yax_poll_tracked(ex, task.poll, task.frame);
        if (ready != NULL) {
            yax_retain(ex, task.frame);
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
        yashy_free(ex->retained[i]);
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

/* True when the executor already retained this frame: it reached ready
 * through a drain or an `await` drive, and must not be polled again. */
static int yax_is_retained(yashy_executor *ex, void *frame) {
    if (ex == NULL || frame == NULL) {
        return 0;
    }
    yax_lock_acquire(&ex->lock);
    int found = 0;
    for (long long i = 0; i < ex->retained_count; i++) {
        if (ex->retained[i] == frame) {
            found = 1;
            break;
        }
    }
    yax_lock_release(&ex->lock);
    return found;
}

/* Remove a task's queued record without freeing its frame: used when a
 * driver takes the task over inline — the scope joining a child home, or
 * `block_on` starting an `await` — so a later executor drain must not poll
 * it a second time. Answers 1 when the record was here (the caller now owns
 * the task's completion: it must retain the frame when it finishes, the mark
 * the queue path leaves behind) and 0 when someone else already held it. */
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

/* Join or cancel every child in reverse spawn order. A child whose resume
 * index already reads done (an `await` drove it home) is skipped. Every other
 * child is polled until it is ready; a child that stays pending is cancelled
 * (its frame is freed, which is also the cancel path a `defer` observes
 * through the one final poll that runs its pending segment). */
void yashy_scope_join_or_cancel(yashy_scope *scope) {
    if (scope == NULL) {
        return;
    }
    for (long long i = scope->count - 1; i >= 0; i--) {
        yax_scope_child child = scope->children[i];
        if (child.poll == NULL || child.frame == NULL) {
            continue;
        }
        /* Header word 0 is the resume index: -1 means done. A frame the
         * executor retained was driven home by an `await` even if the memory
         * was since freed, so it is never polled again here. Both answers are
         * re-read on every attempt rather than once before the loop: a pool
         * worker may drive this very child to completion while the loop waits,
         * and polling such a frame again would run its body a second time. */
        int ready = 0;
        int driven_home = 0;
        for (int attempt = 0; attempt < 1000; attempt++) {
            if (yax_is_retained(scope->ex, child.frame) ||
                *(int *)child.frame == -1) {
                driven_home = 1;
                break;
            }
            if (yax_poll_tracked(scope->ex, child.poll, child.frame) != NULL) {
                ready = 1;
                break;
            }
            yashy_sleep_ms(1);
        }
        if (driven_home) {
            /* The executor drove this child home: dequeue it so a later drain
             * does not touch it, and leave the frame to the driver that owns
             * it -- the same answer as the check above the loop. */
            yax_unqueue(scope->ex, child.frame);
            continue;
        }
        if (!ready) {
            yashy_task_cancel(scope->ex, child.poll, child.frame);
        } else {
            /* The scope drove this child home: dequeue it so the executor's
             * own drain does not run it a second time, and free its frame —
             * a discarded handle has no owner left to free it, and a handle
             * awaited inside the scope took the retained branch above. */
            yax_unqueue(scope->ex, child.frame);
            yashy_free(child.frame);
        }
    }
    scope->count = 0;
}

int yashy_task_poll(yashy_executor *ex, yashy_poll_fn poll, void *frame) {
    if (poll == NULL || frame == NULL) {
        return -1;
    }
    return yax_poll_tracked(ex, poll, frame) != NULL ? 1 : 0;
}

/* Poll one task; on a pending answer, drive the spawned queue once so
 * `spawn`ed children make progress even while the main task waits. */
static int yax_drive_once(yashy_executor *ex, yashy_poll_fn poll, void *frame) {
    int ready = poll != NULL && frame != NULL && yax_poll_tracked(ex, poll, frame) != NULL;
    if (ready) {
        return 1;
    }
    yax_task_record task;
    if (yax_dequeue(ex, &task)) {
        void *child_ready = yax_poll_tracked(ex, task.poll, task.frame);
        if (child_ready != NULL) {
            yax_retain(ex, task.frame);
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
    int took_record = yax_unqueue(ex, frame);
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
    if (took_record) {
        /* The record this call took over would have completed through the
         * queue, and the queue's completion path retains the frame: that
         * retained mark is what `yashy_scope_join_or_cancel` reads to know
         * a child was driven home by an `await` — its owner frees the
         * frame as soon as this returns, and the join must never poll a
         * freed child. Leave the same mark for the frame this call
         * completed itself. */
        yax_retain(ex, frame);
    }
    /* The main task completed before `block_on` returns, and so did every
     * task spawned from it: drain the queue before answering. */
    for (;;) {
        yax_task_record task;
        if (!yax_dequeue(ex, &task)) {
            break;
        }
        void *ready = yax_poll_tracked(ex, task.poll, task.frame);
        if (ready != NULL) {
            yax_retain(ex, task.frame);
            yax_lock_acquire(&ex->lock);
            ex->completed++;
            yax_lock_release(&ex->lock);
        } else {
            yax_requeue(ex, task.poll, task.frame);
            /* A still-pending child after the main task is done can only
             * mean progress stopped; avoid an infinite drain loop. */
            yashy_sleep_ms(1);
            if (!yax_dequeue(ex, &task)) {
                continue;
            }
            ready = yax_poll_tracked(ex, task.poll, task.frame);
            if (ready != NULL) {
                yax_retain(ex, task.frame);
                yax_lock_acquire(&ex->lock);
                ex->completed++;
                yax_lock_release(&ex->lock);
            } else {
                yax_requeue(ex, task.poll, task.frame);
            }
        }
        yax_finish_one(ex);
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

void yashy_task_cancel(yashy_executor *ex, yashy_poll_fn poll, void *frame) {
    if (frame == NULL) {
        return;
    }
    /* A frame another thread is inside must not be freed under it: wait for
     * that claim to clear. The wait is bounded because a scope has to make
     * progress even if a driver wedges, and a claim is held only across one
     * poll segment, which returns promptly. */
    if (ex != NULL) {
        for (int attempt = 0; attempt < 100; attempt++) {
            yax_lock_acquire(&ex->lock);
            long long index = yax_registry_index(ex, frame);
            int claimed = index >= 0 && ex->registry[index].state == 1;
            yax_lock_release(&ex->lock);
            if (!claimed) {
                break;
            }
            yashy_sleep_ms(1);
        }
    }
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
                yashy_free(frame);
                return;
            }
        }
        yax_lock_release(&ex->lock);
    }
    (void)poll;
    yashy_free(frame);
}

void yashy_task_drop(void *frame) {
    if (frame == NULL) {
        return;
    }
    /* A dropped frame is no longer live; take it out of the default
     * executor's registry without creating the executor just to do so. */
    yax_lock_acquire(&yax_default_lock);
    yashy_executor *ex = yax_default;
    yax_lock_release(&yax_default_lock);
    yax_unregister(ex, frame);
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
