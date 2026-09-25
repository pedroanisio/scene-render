#define _POSIX_C_SOURCE 200809L
#include "scene_render/parallel.h"

#include <pthread.h>
#include <stdlib.h>
#include <unistd.h>

/* Persistent worker pool behind sr_parallel_for.
 *
 * Partitions are the same static [n*i/count, n*(i+1)/count) ranges as a
 * spawn-per-call implementation, and each range is still processed by one
 * call of the worker function, so results are identical for any thread
 * count; only thread creation/join is amortised. Workers are created on
 * first use (up to SR_POOL_MAX), sleep on a condition variable between
 * dispatches, and are joined at exit. One dispatch runs at a time; a
 * dispatch issued from inside a worker (or while another is in flight on a
 * different thread) runs inline on the caller, which is equally
 * deterministic. */

#define SR_POOL_MAX 64U

typedef struct {
    pthread_mutex_t lock;
    pthread_cond_t wake;      /* workers wait for a new generation */
    pthread_cond_t done;      /* dispatcher waits for completion */
    pthread_t threads[SR_POOL_MAX];
    unsigned started;         /* workers created */
    unsigned long generation; /* bumped per dispatch */
    unsigned active;          /* participants in the current dispatch */
    unsigned remaining;       /* participants not yet finished */
    bool busy;                /* a dispatch is in flight */
    bool stopping;
    SrParallelFunction function;
    void *context;
    size_t work_items;
    unsigned count;           /* partitions, including the caller's */
} SrPool;

static SrPool pool = {
    .lock = PTHREAD_MUTEX_INITIALIZER,
    .wake = PTHREAD_COND_INITIALIZER,
    .done = PTHREAD_COND_INITIALIZER,
};
static pthread_once_t pool_exit_once = PTHREAD_ONCE_INIT;
static _Thread_local bool in_worker;

static void pool_shutdown(void) {
    pthread_mutex_lock(&pool.lock);
    pool.stopping = true;
    pthread_cond_broadcast(&pool.wake);
    unsigned started = pool.started;
    pthread_mutex_unlock(&pool.lock);
    for (unsigned i = 0; i < started; ++i) pthread_join(pool.threads[i], NULL);
}

static void pool_register_exit(void) { atexit(pool_shutdown); }

/* Worker `index` (0-based) owns partition index + 1; the caller owns 0. */
static void *pool_worker(void *opaque) {
    unsigned index = (unsigned)(size_t)opaque;
    in_worker = true;
    unsigned long seen = 0;
    pthread_mutex_lock(&pool.lock);
    for (;;) {
        while (!pool.stopping && pool.generation == seen)
            pthread_cond_wait(&pool.wake, &pool.lock);
        if (pool.stopping) break;
        seen = pool.generation;
        if (index + 1U >= pool.count) continue; /* not needed this round */
        SrParallelFunction function = pool.function;
        void *context = pool.context;
        size_t n = pool.work_items, count = pool.count, part = index + 1U;
        pthread_mutex_unlock(&pool.lock);
        function(context, n * part / count, n * (part + 1U) / count);
        pthread_mutex_lock(&pool.lock);
        if (--pool.remaining == 0) pthread_cond_signal(&pool.done);
    }
    pthread_mutex_unlock(&pool.lock);
    return NULL;
}

/* Called with the lock held; returns the number of usable workers. */
static unsigned pool_ensure(unsigned wanted) {
    pthread_once(&pool_exit_once, pool_register_exit);
    if (wanted > SR_POOL_MAX) wanted = SR_POOL_MAX;
    while (pool.started < wanted) {
        if (pthread_create(&pool.threads[pool.started], NULL, pool_worker,
                           (void *)(size_t)pool.started) != 0)
            break;
        ++pool.started;
    }
    return pool.started;
}

static void run_inline(size_t work_items, unsigned count,
                       SrParallelFunction function, void *context) {
    for (unsigned i = 0; i < count; ++i)
        function(context, work_items * i / count, work_items * (i + 1U) / count);
}

unsigned sr_parallel_thread_count(unsigned requested, size_t work_items) {
    if (work_items == 0) return 0;
    unsigned count = requested;
    if (count == 0) {
        long detected = sysconf(_SC_NPROCESSORS_ONLN);
        count = detected > 0 ? (unsigned)detected : 1U;
    }
    if (count > SR_POOL_MAX) count = SR_POOL_MAX;
    if ((size_t)count > work_items) count = (unsigned)work_items;
    return count ? count : 1U;
}

SrStatus sr_parallel_for(size_t work_items, unsigned requested,
                         SrParallelFunction function, void *context) {
    if (!function) return SR_ERR_ARGUMENT;
    if (work_items == 0) return SR_OK;
    unsigned count = sr_parallel_thread_count(requested, work_items);
    if (count <= 1 || in_worker) {
        run_inline(work_items, count, function, context);
        return SR_OK;
    }
    pthread_mutex_lock(&pool.lock);
    if (pool.busy || pool.stopping) {
        /* Another thread owns the pool: same partitions, run here. */
        pthread_mutex_unlock(&pool.lock);
        run_inline(work_items, count, function, context);
        return SR_OK;
    }
    unsigned workers = pool_ensure(count - 1U);
    if (workers + 1U < count) {
        pthread_mutex_unlock(&pool.lock);
        run_inline(work_items, count, function, context);
        return SR_OK;
    }
    pool.busy = true;
    pool.function = function;
    pool.context = context;
    pool.work_items = work_items;
    pool.count = count;
    pool.remaining = count - 1U;
    ++pool.generation;
    pthread_cond_broadcast(&pool.wake);
    pthread_mutex_unlock(&pool.lock);

    function(context, 0, work_items / count);

    pthread_mutex_lock(&pool.lock);
    while (pool.remaining > 0) pthread_cond_wait(&pool.done, &pool.lock);
    pool.busy = false;
    pthread_mutex_unlock(&pool.lock);
    return SR_OK;
}
