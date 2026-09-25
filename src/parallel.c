#define _POSIX_C_SOURCE 200809L
#include "scene_render/parallel.h"

#include <pthread.h>
#include <stdlib.h>
#include <unistd.h>

typedef struct {
    SrParallelFunction function;
    void *context;
    size_t begin;
    size_t end;
} SrParallelJob;

static void *run_job(void *opaque) {
    SrParallelJob *job = opaque;
    job->function(job->context, job->begin, job->end);
    return NULL;
}

unsigned sr_parallel_thread_count(unsigned requested, size_t work_items) {
    if (work_items == 0) return 0;
    unsigned count = requested;
    if (count == 0) {
        long detected = sysconf(_SC_NPROCESSORS_ONLN);
        count = detected > 0 ? (unsigned)detected : 1U;
    }
    if (count > 64U) count = 64U;
    if ((size_t)count > work_items) count = (unsigned)work_items;
    return count ? count : 1U;
}

SrStatus sr_parallel_for(size_t work_items, unsigned requested,
                         SrParallelFunction function, void *context) {
    if (!function) return SR_ERR_ARGUMENT;
    if (work_items == 0) return SR_OK;
    unsigned count = sr_parallel_thread_count(requested, work_items);
    if (count <= 1) {
        function(context, 0, work_items);
        return SR_OK;
    }
    SrParallelJob *jobs = sr_alloc((size_t)count * sizeof(*jobs));
    pthread_t *threads = sr_alloc((size_t)(count - 1) * sizeof(*threads));
    bool *started = sr_alloc((size_t)(count - 1) * sizeof(*started));
    if (!jobs || !threads || !started) {
        free(jobs); free(threads); free(started);
        function(context, 0, work_items);
        return SR_OK;
    }
    for (unsigned i = 0; i < count; ++i) {
        jobs[i] = (SrParallelJob){
            .function = function,
            .context = context,
            .begin = work_items * i / count,
            .end = work_items * (i + 1U) / count
        };
    }
    for (unsigned i = 1; i < count; ++i) {
        started[i - 1] = pthread_create(&threads[i - 1], NULL, run_job,
                                        &jobs[i]) == 0;
        if (!started[i - 1]) run_job(&jobs[i]);
    }
    run_job(&jobs[0]);
    for (unsigned i = 1; i < count; ++i)
        if (started[i - 1]) pthread_join(threads[i - 1], NULL);
    free(started); free(threads); free(jobs);
    return SR_OK;
}
