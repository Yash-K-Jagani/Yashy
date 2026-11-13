/* The executor, exercised as a real C program: 10 000 tasks complete in
 * both modes, a task spawned from a task completes before `block_on`
 * returns, a scope frees the frame of a discarded `spawn` handle while
 * leaving an awaited child's freed frame alone, and every frame the test
 * allocates comes back (the `yashy_live_allocations_now` counter returns to
 * its baseline).
 *
 * Frames are `yashy_alloc` blocks with the generated code's header layout:
 *   [0] unsigned resume, [1] padding, [2] poll fn pointer, [3] child frame
 * The executor itself never reads the header; the test's poll functions
 * mimic the generated poll functions, which do.
 */

#include "yashy_runtime.h"

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

#define FRAME_SIZE 64

static void frame_init(void *frame, void *(*poll)(void *)) {
    unsigned char *bytes = (unsigned char *)frame;
    memset(bytes, 0, FRAME_SIZE);
    *(unsigned int *)bytes = 0;
    /* Header word 1 (offset 8) is the poll function pointer the code
     * generator writes; the tests keep the same layout. */
    *(void **)(bytes + 8) = (void *)poll;
}

/* A task that is ready on its first poll and records its value. */
typedef struct counter_frame {
    unsigned int resume;
    unsigned int pad;
    void *poll;
    void *child;
    long long value;
} counter_frame;

static void *counter_poll(void *raw) {
    counter_frame *frame = (counter_frame *)raw;
    frame->value += 1;
    return frame;
}

/* A task that pends once, then is ready: the resume index survives. */
typedef struct suspend_frame {
    unsigned int resume;
    unsigned int pad;
    void *poll;
    void *child;
    long long before;
    long long after;
} suspend_frame;

static void *suspend_poll(void *raw) {
    suspend_frame *frame = (suspend_frame *)raw;
    if (frame->resume == 0) {
        frame->before = 41;
        frame->resume = 1;
        return NULL; /* pending */
    }
    frame->after = frame->before + 1;
    return frame;
}

/* A parent task that spawns a child from inside its poll. */
typedef struct parent_frame {
    unsigned int resume;
    unsigned int pad;
    void *poll;
    void *child;
    yashy_executor *executor;
    long long *child_done;
    int phase;
} parent_frame;

typedef struct child_frame {
    unsigned int resume;
    unsigned int pad;
    void *poll;
    void *child;
    long long *child_done;
} child_frame;

static void *child_poll(void *raw) {
    child_frame *frame = (child_frame *)raw;
    *frame->child_done += 1;
    return frame;
}

static void *parent_poll(void *raw) {
    parent_frame *frame = (parent_frame *)raw;
    if (frame->phase == 0) {
        frame->phase = 1;
        void *child = yashy_alloc(FRAME_SIZE, 8);
        CHECK(child != NULL);
        frame_init(child, child_poll);
        ((child_frame *)child)->child_done = frame->child_done;
        CHECK(yashy_task_spawn(frame->executor, child_poll, child) == 0);
        return NULL; /* pending until the child runs */
    }
    return frame;
}

static void run_mode(int threads) {
    long long baseline = yashy_live_allocations_now();
    yashy_executor *executor = yashy_executor_new(threads);
    CHECK(executor != NULL);

    /* 10 000 tasks complete. */
    enum { TASKS = 10000 };
    for (int i = 0; i < TASKS; i++) {
        void *frame = yashy_alloc((long long)FRAME_SIZE, 8);
        CHECK(frame != NULL);
        frame_init(frame, counter_poll);
        CHECK(yashy_task_spawn(executor, counter_poll, frame) == 0);
    }
    /* One main task drives the queue to completion. */
    void *main_frame = yashy_alloc((long long)FRAME_SIZE, 8);
    CHECK(main_frame != NULL);
    frame_init(main_frame, counter_poll);
    CHECK(yashy_block_on(executor, counter_poll, main_frame) == 0);
    CHECK(((counter_frame *)main_frame)->value == 1);
    yashy_task_drop(main_frame);
    CHECK(yashy_executor_completed(executor) == TASKS);

    /* A suspended task keeps its local across the suspension. */
    void *suspended = yashy_alloc((long long)FRAME_SIZE, 8);
    CHECK(suspended != NULL);
    frame_init(suspended, suspend_poll);
    CHECK(yashy_task_poll(executor, suspend_poll, suspended) == 0);
    CHECK(yashy_task_poll(executor, suspend_poll, suspended) == 1);
    CHECK(((suspend_frame *)suspended)->after == 42);
    yashy_task_drop(suspended);

    /* A task spawned from a task completes before `block_on` returns. */
    long long child_done = 0;
    void *parent = yashy_alloc((long long)FRAME_SIZE, 8);
    CHECK(parent != NULL);
    frame_init(parent, parent_poll);
    ((parent_frame *)parent)->executor = executor;
    ((parent_frame *)parent)->child_done = &child_done;
    ((parent_frame *)parent)->phase = 0;
    CHECK(yashy_block_on(executor, parent_poll, parent) == 0);

    CHECK(child_done == 1);
    yashy_task_drop(parent);

    /* The queue's completed frames are held until the executor goes away, so
     * the reference here is not the baseline: it is what the sections below
     * must leave unchanged. */
    long long held = yashy_live_allocations_now();

    /* An awaited child of a scope: the `await` reads the frame and hands it
     * back, which marks the scope's slot done and frees the frame. Leaving
     * the scope must therefore skip that slot without reading anything --
     * the frame is gone and the allocator may already have reused it. */
    yashy_scope *awaited_scope = yashy_scope_create(executor);
    CHECK(awaited_scope != NULL);
    void *awaited = yashy_alloc((long long)FRAME_SIZE, 8);
    CHECK(awaited != NULL);
    frame_init(awaited, counter_poll);
    CHECK(yashy_task_spawn(executor, counter_poll, awaited) == 0);
    CHECK(yashy_block_on(executor, counter_poll, awaited) == 0);
    CHECK(((counter_frame *)awaited)->value == 1);
    /* The same block is usually handed straight back; a fresh frame there
     * must not be mistaken for the awaited one. */
    yashy_task_release(executor, awaited); /* what the generated `await` emits */
    void *reused = yashy_alloc((long long)FRAME_SIZE, 8);
    CHECK(reused != NULL);
    frame_init(reused, counter_poll);
    yashy_scope_join_or_cancel(awaited_scope);
    yashy_scope_free(awaited_scope);
    CHECK(((counter_frame *)reused)->value == 0);
    yashy_free(reused);
    CHECK(yashy_live_allocations_now() == held); /* the awaited slot leaked */

    /* A discarded `spawn` handle: nobody awaits the task, so leaving the
     * scope is the only thing that can free its frame. A pool worker
     * completing it is the path that leaked one allocation per handle. */
    yashy_scope *discard_scope = yashy_scope_create(executor);
    CHECK(discard_scope != NULL);
    for (int i = 0; i < 8; i++) {
        void *frame = yashy_alloc((long long)FRAME_SIZE, 8);
        CHECK(frame != NULL);
        frame_init(frame, counter_poll);
        CHECK(yashy_task_spawn(executor, counter_poll, frame) == 0);
    }
    yashy_scope_join_or_cancel(discard_scope);
    yashy_scope_free(discard_scope);
    CHECK(yashy_live_allocations_now() == held); /* a discarded handle leaked */

    /* Cancel frees the frame: the cancelled task was never spawned, so no
     * worker can race the free. */
    void *cancelled = yashy_alloc((long long)FRAME_SIZE, 8);
    CHECK(cancelled != NULL);
    frame_init(cancelled, counter_poll);
    yashy_task_cancel(executor, counter_poll, cancelled);
    CHECK(yashy_live_allocations_now() == held); /* cancel leaked */

    yashy_executor_free(executor);
    CHECK(yashy_live_allocations_now() == baseline);
}

int main(void) {
    run_mode(0); /* single-threaded */
    run_mode(4); /* thread pool */
    return 0;
}
