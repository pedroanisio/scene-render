/* SPDX-License-Identifier: Apache-2.0 */
#include "scene_render/markers.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>

SrTimeline *sr_timeline_create(void) {
    return sr_alloc(sizeof(SrTimeline));
}

void sr_timeline_free(SrTimeline *timeline) {
    if (!timeline) return;
    for (size_t i = 0; i < timeline->marker_count; ++i) {
        free(timeline->markers[i].id);
        free(timeline->markers[i].label);
    }
    for (size_t i = 0; i < timeline->reference_count; ++i)
        free(timeline->references[i]);
    free(timeline->markers);
    free(timeline->index);
    free(timeline->references);
    free(timeline);
}

SrStatus sr_timeline_add_marker(SrTimeline *timeline, const SrMarker *marker) {
    if (!timeline || !marker) return SR_ERR_ARGUMENT;
    if (timeline->marker_count >= SR_MAX_MARKERS) return SR_ERR_XML;
    if (timeline->marker_count == timeline->marker_capacity) {
        size_t capacity = timeline->marker_capacity ? timeline->marker_capacity * 2 : 8;
        if (capacity > SR_MAX_MARKERS) capacity = SR_MAX_MARKERS;
        SrMarker *items = sr_realloc(timeline->markers, capacity * sizeof(*items));
        if (!items) return SR_ERR_MEMORY;
        timeline->markers = items;
        timeline->marker_capacity = capacity;
    }
    timeline->markers[timeline->marker_count++] = *marker;
    return SR_OK;
}

SrStatus sr_timeline_add_reference(SrTimeline *timeline, const char *id,
                                   uint32_t *reference) {
    if (!timeline || !id || !reference) return SR_ERR_ARGUMENT;
    /* Bounded by the scene key limit; a uint32 index always suffices. */
    if (timeline->reference_count >= UINT32_MAX - 1) return SR_ERR_XML;
    if (timeline->reference_count == timeline->reference_capacity) {
        size_t capacity = timeline->reference_capacity
            ? timeline->reference_capacity * 2 : 8;
        char **items = sr_realloc(timeline->references, capacity * sizeof(*items));
        if (!items) return SR_ERR_MEMORY;
        timeline->references = items;
        timeline->reference_capacity = capacity;
    }
    char *copy = sr_strdup(id);
    if (!copy) return SR_ERR_MEMORY;
    timeline->references[timeline->reference_count++] = copy;
    *reference = (uint32_t)timeline->reference_count;
    return SR_OK;
}

const char *sr_timeline_reference(const SrTimeline *timeline, uint32_t reference) {
    if (!timeline || !reference || reference > timeline->reference_count) return NULL;
    return timeline->references[reference - 1];
}

bool sr_beat_id_parse(const char *id, bool *bar, uint64_t *number) {
    if (!id) return false;
    const char *digits;
    if (!strncmp(id, "beat.", 5)) {
        *bar = false;
        digits = id + 5;
    } else if (!strncmp(id, "bar.", 4)) {
        *bar = true;
        digits = id + 4;
    } else {
        return false;
    }
    if (*digits < '1' || *digits > '9') return false;
    uint64_t value = 0;
    size_t count = 0;
    for (const char *p = digits; *p; ++p) {
        if (*p < '0' || *p > '9' || ++count > 19) return false;
        value = value * 10u + (uint64_t)(*p - '0');
    }
    *number = value;
    return true;
}

/* (N - 1) * 60 is an exact integer product for every bounded beat, so bar
 * and beat instants are computed by identical operations. */
double sr_beat_grid_beat_time(const SrBeatGrid *grid, uint64_t beat) {
    return grid->offset + (double)((beat - 1u) * 60u) / grid->bpm;
}

double sr_beat_grid_bar_time(const SrBeatGrid *grid, uint64_t bar) {
    return sr_beat_grid_beat_time(grid, (bar - 1u) * grid->beats_per_bar + 1u);
}

bool sr_timeline_generates(const SrTimeline *timeline, const char *id) {
    bool bar;
    uint64_t number;
    if (!timeline || !timeline->grid.present || !sr_beat_id_parse(id, &bar, &number))
        return false;
    return number <= (bar ? timeline->grid.bar_count : timeline->grid.beat_count);
}

static int compare_ids(const void *left, const void *right) {
    return strcmp(((const SrMarkerIndex *)left)->id,
                  ((const SrMarkerIndex *)right)->id);
}

static SrStatus count_beats(SrBeatGrid *grid, double duration) {
    grid->beat_count = grid->bar_count = 0;
    if (!grid->present || grid->offset > duration) return SR_OK;
    /* Estimate, then settle on the exact closed form at the boundary. */
    double estimate = floor((duration - grid->offset) * grid->bpm / 60.0) + 1.0;
    if (!(estimate <= (double)SR_MAX_GENERATED_MARKERS + 2.0)) return SR_ERR_XML;
    uint64_t count = estimate < 1.0 ? 1u : (uint64_t)estimate;
    while (count > 0 && sr_beat_grid_beat_time(grid, count) > duration) --count;
    while (count <= SR_MAX_GENERATED_MARKERS &&
           sr_beat_grid_beat_time(grid, count + 1u) <= duration) ++count;
    if (count > SR_MAX_GENERATED_MARKERS) return SR_ERR_XML;
    grid->beat_count = count;
    grid->bar_count = count ? (count - 1u) / grid->beats_per_bar + 1u : 0u;
    return SR_OK;
}

SrStatus sr_timeline_prepare(SrTimeline *timeline, double duration,
                             const char **problem, size_t *line) {
    *problem = NULL;
    *line = 0;
    if (!timeline) return SR_OK;
    free(timeline->index);
    timeline->index = NULL;
    timeline->index_count = 0;
    SrBeatGrid *grid = &timeline->grid;
    if (grid->present &&
        (!(grid->bpm > 0.0) || grid->bpm > SR_MAX_BPM || !isfinite(grid->offset) ||
         fabs(grid->offset) > SR_MAX_TIMELINE_SECONDS || !grid->beats_per_bar ||
         grid->beats_per_bar > SR_MAX_BEATS_PER_BAR)) {
        *problem = "invalid beat grid";
        *line = grid->source_line;
        return SR_ERR_XML;
    }
    if (count_beats(grid, duration) != SR_OK) {
        *problem = "beatGrid generates more than 1048576 beats within the "
                   "project duration";
        *line = grid->source_line;
        return SR_ERR_XML;
    }
    size_t count = 0;
    for (size_t i = 0; i < timeline->marker_count; ++i)
        if (timeline->markers[i].id) ++count;
    if (!count) return SR_OK;
    timeline->index = sr_alloc(count * sizeof(*timeline->index));
    if (!timeline->index) return SR_ERR_MEMORY;
    for (size_t i = 0; i < timeline->marker_count; ++i) {
        const SrMarker *marker = &timeline->markers[i];
        if (!marker->id) continue;
        if (sr_timeline_generates(timeline, marker->id)) {
            *problem = "marker id equals an id generated by beatGrid";
            *line = marker->source_line;
            return SR_ERR_XML;
        }
        timeline->index[timeline->index_count++] =
            (SrMarkerIndex){marker->id, (uint32_t)i};
    }
    qsort(timeline->index, timeline->index_count, sizeof(*timeline->index),
          compare_ids);
    for (size_t i = 1; i < timeline->index_count; ++i) {
        const SrMarker *a = &timeline->markers[timeline->index[i - 1].marker];
        const SrMarker *b = &timeline->markers[timeline->index[i].marker];
        if (!strcmp(a->id, b->id)) {
            *problem = "marker id must be unique";
            *line = a->source_line > b->source_line ? a->source_line : b->source_line;
            return SR_ERR_XML;
        }
    }
    return SR_OK;
}

SrMarkerLookup sr_timeline_lookup(const SrTimeline *timeline, const char *id,
                                  double *time) {
    if (!timeline || !id) return SR_MARKER_LOOKUP_UNKNOWN;
    size_t lo = 0, hi = timeline->index_count;
    while (lo < hi) {
        size_t mid = lo + (hi - lo) / 2;
        const SrMarker *marker = &timeline->markers[timeline->index[mid].marker];
        int order = strcmp(id, marker->id);
        if (!order) {
            *time = marker->time;
            return SR_MARKER_LOOKUP_FOUND;
        }
        if (order < 0) hi = mid;
        else lo = mid + 1;
    }
    bool bar;
    uint64_t number;
    if (!timeline->grid.present || !sr_beat_id_parse(id, &bar, &number))
        return SR_MARKER_LOOKUP_UNKNOWN;
    if (number > (bar ? timeline->grid.bar_count : timeline->grid.beat_count))
        return SR_MARKER_LOOKUP_OUT_OF_GRID;
    *time = bar ? sr_beat_grid_bar_time(&timeline->grid, number)
                : sr_beat_grid_beat_time(&timeline->grid, number);
    return SR_MARKER_LOOKUP_FOUND;
}

bool sr_marker_kind_parse(const char *text, SrMarkerKind *kind) {
    static const char *const names[] = {"cue", "chapter", "section", "beat",
                                        "comment", "todo", "cta"};
    if (!text || !kind) return false;
    for (size_t i = 0; i < sizeof(names) / sizeof(names[0]); ++i) {
        if (!strcmp(text, names[i])) {
            *kind = (SrMarkerKind)i;
            return true;
        }
    }
    return false;
}

void sr_node_timeline_free(SrNodeTimeline *timeline) {
    if (!timeline) return;
    free(timeline->name);
    for (size_t i = 0; i < timeline->tag_count; ++i) free(timeline->tags[i]);
    free(timeline->tags);
    free(timeline->start_marker);
    free(timeline->end_marker);
    free(timeline);
}
