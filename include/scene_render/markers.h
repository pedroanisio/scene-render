/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SCENE_RENDER_MARKERS_H
#define SCENE_RENDER_MARKERS_H

#include "scene_render/common.h"

/* Timeline structure (schema 1.1, B1-5; docs/design/b1-5-timeline.md).
 * Everything here is resolved at load time and read-only while rendering. */

#define SR_MAX_MARKERS 65536u
#define SR_MAX_MARKER_LABEL 4096u
#define SR_MAX_GENERATED_MARKERS 1048576u
#define SR_MAX_BEATS_PER_BAR 1024u
#define SR_MAX_BPM 1e6
#define SR_MAX_TIMELINE_SECONDS 1e6   /* marker times, offsets and gaps */
#define SR_MIN_TIME_SCALE 1e-6
#define SR_MAX_TIME_SCALE 1e6
#define SR_MAX_CLOCK_OFFSET 1e12
#define SR_MAX_NODE_NAME 1024u
#define SR_MAX_NODE_TAGS 64u
#define SR_MAX_TAG_BYTES 128u

typedef enum {
    SR_MARKER_CUE, SR_MARKER_CHAPTER, SR_MARKER_SECTION, SR_MARKER_BEAT,
    SR_MARKER_COMMENT, SR_MARKER_TODO, SR_MARKER_CTA
} SrMarkerKind;

typedef struct {
    char *id;                  /* owned; NULL when the marker has no id */
    double time;               /* composition seconds */
    double duration;           /* seconds, >= 0 */
    SrMarkerKind kind;
    char *label;               /* owned; NULL when absent */
    SrColor color;             /* straight working-space color */
    bool color_set;
    size_t source_line;
} SrMarker;

/* A musical grid generating the ids beat.N and bar.M (both 1-based):
 * beat.N is at offset + (N - 1) * 60 / bpm, bar.M at beat.((M-1)*k + 1). */
typedef struct {
    bool present;
    double bpm;
    double offset;             /* composition seconds of beat.1 */
    uint32_t beats_per_bar;
    uint64_t beat_count;       /* generated beats; set by sr_timeline_prepare */
    uint64_t bar_count;
    size_t source_line;
} SrBeatGrid;

typedef struct {
    const char *id;            /* borrowed from the marker */
    uint32_t marker;           /* index into SrTimeline.markers */
} SrMarkerIndex;

typedef struct SrTimeline {
    SrMarker *markers;         /* document order */
    size_t marker_count, marker_capacity;
    SrMarkerIndex *index;      /* markers with ids, sorted by id */
    size_t index_count;
    SrBeatGrid grid;
    char **references;         /* key marker ids; SrKeyframe.marker - 1 */
    size_t reference_count, reference_capacity;
    size_t pending_tracks;     /* tracks with snapped keys not yet resolved */
    size_t pending_line;       /* first such key, for diagnostics */
} SrTimeline;

/* Per-node B1-5 attributes; allocated only for nodes that use one. */
typedef struct SrNodeTimeline {
    char *name;                /* owned; NULL when absent */
    char **tags;               /* owned NMTOKENs in document order */
    size_t tag_count;
    char *start_marker, *end_marker; /* owned ids; NULL when absent */
    bool sequence;             /* a <sequence>: children are placed in order */
    double time_offset;        /* children clock offset, seconds */
    double time_scale;         /* children clock rate, > 0 */
    double time_gap;           /* sequence: seconds between items */
} SrNodeTimeline;

typedef enum {
    SR_MARKER_LOOKUP_FOUND,
    SR_MARKER_LOOKUP_UNKNOWN,       /* no marker and no generated id */
    SR_MARKER_LOOKUP_OUT_OF_GRID    /* beat.N / bar.M outside the grid */
} SrMarkerLookup;

SrTimeline *sr_timeline_create(void);
void sr_timeline_free(SrTimeline *timeline);
/* Takes ownership of marker->id and marker->label on success only. */
SrStatus sr_timeline_add_marker(SrTimeline *timeline, const SrMarker *marker);
/* Copies `id`; *reference receives the 1-based SrKeyframe.marker value. */
SrStatus sr_timeline_add_reference(SrTimeline *timeline, const char *id,
                                   uint32_t *reference);
const char *sr_timeline_reference(const SrTimeline *timeline, uint32_t reference);

/* Counts the generated beats within [.., duration] and builds the sorted id
 * index. On SR_ERR_XML, *problem names the failure and *line its source:
 * a duplicate marker id, a grid over SR_MAX_GENERATED_MARKERS, or an
 * explicit marker whose id equals a generated one. */
SrStatus sr_timeline_prepare(SrTimeline *timeline, double duration,
                             const char **problem, size_t *line);
SrMarkerLookup sr_timeline_lookup(const SrTimeline *timeline, const char *id,
                                  double *time);

/* Generated ids: "beat." or "bar." then a canonical positive decimal (no
 * sign, no leading zero, at most 19 digits). */
bool sr_beat_id_parse(const char *id, bool *bar, uint64_t *number);
bool sr_timeline_generates(const SrTimeline *timeline, const char *id);
double sr_beat_grid_beat_time(const SrBeatGrid *grid, uint64_t beat);
double sr_beat_grid_bar_time(const SrBeatGrid *grid, uint64_t bar);

bool sr_marker_kind_parse(const char *text, SrMarkerKind *kind);
void sr_node_timeline_free(SrNodeTimeline *timeline);

#endif
