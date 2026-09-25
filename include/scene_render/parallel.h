#ifndef SCENE_RENDER_PARALLEL_H
#define SCENE_RENDER_PARALLEL_H

#include "scene_render/common.h"

typedef void (*SrParallelFunction)(void *context, size_t begin, size_t end);

unsigned sr_parallel_thread_count(unsigned requested, size_t work_items);
SrStatus sr_parallel_for(size_t work_items, unsigned requested,
                         SrParallelFunction function, void *context);

#endif
