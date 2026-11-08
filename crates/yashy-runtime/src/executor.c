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
 */

#include "yashy_runtime.h"

#include <stdint.h>
#include <stdlib.h>
#include <string.h>

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

typedef struct yax_task_record {
    yashy_poll_fn poll;
    void *frame;
} yax_task_record;

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
};

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
        void *ready = task.poll == NULL ? NULL : task.poll(task.frame);
        if (ready != NULL) {
            yax_retain(ex, task.frame);
            yax_lock_acquire(&ex->lock);
            ex->completed++;
            yax_lock_release(&ex->lock);
        } else {
            /* Still pending: requeue at the tail (FIFO, no work stealing). */
            yax_enqueue(ex, task.poll, task.frame);
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
        yax_default = yashy_executor_new(0);
    }
    yashy_executor *ex = yax_default;
    yax_lock_release(&yax_default_lock);
    return ex;
}

int yashy_task_spawn(yashy_executor *ex, yashy_poll_fn poll, void *frame) {
    if (ex == NULL || poll == NULL || frame == NULL) {
        return -1;
    }
    return yax_enqueue(ex, poll, frame);
}

int yashy_task_poll(yashy_executor *ex, yashy_poll_fn poll, void *frame) {
    (void)ex;
    if (poll == NULL || frame == NULL) {
        return -1;
    }
    return poll(frame) != NULL ? 1 : 0;
}

/* Poll one task; on a pending answer, drive the spawned queue once so
 * `spawn`ed children make progress even while the main task waits. */
static int yax_drive_once(yashy_executor *ex, yashy_poll_fn poll, void *frame) {
    int ready = poll != NULL && frame != NULL && poll(frame) != NULL;
    if (ready) {
        return 1;
    }
    yax_task_record task;
    if (yax_dequeue(ex, &task)) {
        void *child_ready = task.poll == NULL ? NULL : task.poll(task.frame);
        if (child_ready != NULL) {
            yax_retain(ex, task.frame);
            yax_lock_acquire(&ex->lock);
            ex->completed++;
            yax_lock_release(&ex->lock);
            yax_finish_one(ex);
        } else {
            yax_enqueue(ex, task.poll, task.frame);
        }
    }
    return 0;
}

int yashy_block_on(yashy_executor *ex, yashy_poll_fn poll, void *frame) {
    if (ex == NULL || poll == NULL || frame == NULL) {
        return -1;
    }
    for (;;) {
        if (yax_drive_once(ex, poll, frame)) {
            break;
        }
        /* With a pool, workers may finish the queue; without one, requeue
         * churn is the only progress possible, so poll again. A tiny sleep
         * keeps a genuinely pending task (a channel wait inside a poll)
         * from spinning hot. */
        yashy_sleep_ms(1);
    }
    /* The main task completed before `block_on` returns, and so did every
     * task spawned from it: drain the queue before answering. */
    for (;;) {
        yax_task_record task;
        if (!yax_dequeue(ex, &task)) {
            break;
        }
        void *ready = task.poll == NULL ? NULL : task.poll(task.frame);
        if (ready != NULL) {
            yax_retain(ex, task.frame);
            yax_lock_acquire(&ex->lock);
            ex->completed++;
            yax_lock_release(&ex->lock);
        } else {
            yax_enqueue(ex, task.poll, task.frame);
            /* A still-pending child after the main task is done can only
             * mean progress stopped; avoid an infinite drain loop. */
            yashy_sleep_ms(1);
            if (!yax_dequeue(ex, &task)) {
                continue;
            }
            ready = task.poll == NULL ? NULL : task.poll(task.frame);
            if (ready != NULL) {
                yax_retain(ex, task.frame);
                yax_lock_acquire(&ex->lock);
                ex->completed++;
                yax_lock_release(&ex->lock);
            } else {
                yax_enqueue(ex, task.poll, task.frame);
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
    return 0;
}

void yashy_task_cancel(yashy_executor *ex, yashy_poll_fn poll, void *frame) {
    if (frame == NULL) {
        return;
    }
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
