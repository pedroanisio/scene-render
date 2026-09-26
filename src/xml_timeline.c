/* SPDX-License-Identifier: Apache-2.0 */
/* Timeline structure (B1-5): markers, beat grids, node timing attributes,
 * group clocks, sequences and key marker snapping. Parsing records what the
 * document says; sr_xml_resolve_timeline turns it into absolute node
 * intervals and per-track affine clocks before any other resolve step. See
 * docs/design/b1-5-timeline.md. */
#include "xml_internal.h"
#include "scene_render/markers.h"

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static SrTimeline *scene_timeline(ParseContext *ctx) {
    if (!ctx->scene->timeline) ctx->scene->timeline = sr_timeline_create();
    return ctx->scene->timeline;
}

static bool fail_memory(ParseContext *ctx, const char *element, const char *attribute) {
    ctx->out_of_memory = true;
    sr_xml_fail(ctx, element, attribute, "out of memory");
    return false;
}

static bool seconds_attr(ParseContext *ctx, const char *element,
                         const XML_Char **attrs, const char *name, double *value) {
    if (!sr_xml_parse_double_attr(ctx, element, attrs, name, value)) return false;
    if (fabs(*value) > SR_MAX_TIMELINE_SECONDS) {
        sr_xml_fail(ctx, element, name, "expected at most 1e6 seconds in magnitude");
        return false;
    }
    return true;
}

/* ---- markers section ---------------------------------------------------- */

void sr_xml_start_marker(ParseContext *ctx, const XML_Char **attrs) {
    static const char *const allowed[] = {"id", "time", "duration", "kind",
                                          "label", "color"};
    if (!sr_xml_attrs_allowed(ctx, "marker", attrs, allowed,
                              sizeof(allowed) / sizeof(allowed[0]))) return;
    SrTimeline *timeline = scene_timeline(ctx);
    if (!timeline) {
        fail_memory(ctx, "marker", NULL);
        return;
    }
    if (timeline->marker_count >= SR_MAX_MARKERS)
        SR_XML_FAIL_RETURN(ctx, "marker", NULL, "marker limit is 65536");
    SrMarker marker = {.kind = SR_MARKER_CUE, .source_line = sr_xml_line(ctx)};
    const char *time = sr_xml_required(ctx, "marker", attrs, "time");
    if (!time) return;
    if (!seconds_attr(ctx, "marker", attrs, "time", &marker.time) ||
        !seconds_attr(ctx, "marker", attrs, "duration", &marker.duration)) return;
    if (marker.duration < 0.0)
        SR_XML_FAIL_RETURN(ctx, "marker", "duration", "expected a non-negative duration");
    const char *value = sr_xml_attr(attrs, "kind");
    if (value && !sr_marker_kind_parse(value, &marker.kind))
        SR_XML_FAIL_RETURN(ctx, "marker", "kind", "unknown marker kind");
    if ((value = sr_xml_attr(attrs, "color"))) {
        if (!sr_xml_parse_color(ctx, "marker", "color", value, &marker.color)) {
            if (!ctx->failed) sr_xml_fail(ctx, "marker", "color", "invalid color");
            return;
        }
        marker.color_set = true;
    }
    value = sr_xml_attr(attrs, "label");
    if (value && strlen(value) > SR_MAX_MARKER_LABEL)
        SR_XML_FAIL_RETURN(ctx, "marker", "label", "label exceeds 4096 bytes");
    const char *id = sr_xml_attr(attrs, "id");
    if (id && !sr_id_valid(id))
        SR_XML_FAIL_RETURN(ctx, "marker", "id", "expected an XML-compatible identifier");
    if ((id && !(marker.id = sr_strdup(id))) ||
        (value && !(marker.label = sr_strdup(value)))) {
        free(marker.id);
        free(marker.label);
        fail_memory(ctx, "marker", NULL);
        return;
    }
    SrStatus status = sr_timeline_add_marker(timeline, &marker);
    if (status != SR_OK) {
        free(marker.id);
        free(marker.label);
        fail_memory(ctx, "marker", NULL);
    }
}

void sr_xml_start_beat_grid(ParseContext *ctx, const XML_Char **attrs) {
    static const char *const allowed[] = {"bpm", "offset", "beatsPerBar", "source"};
    if (!sr_xml_attrs_allowed(ctx, "beatGrid", attrs, allowed,
                              sizeof(allowed) / sizeof(allowed[0]))) return;
    if (sr_xml_attr(attrs, "source"))
        SR_XML_FAIL_RETURN(ctx, "beatGrid", "source", "unsupported in this build");
    SrTimeline *timeline = scene_timeline(ctx);
    if (!timeline) {
        fail_memory(ctx, "beatGrid", NULL);
        return;
    }
    if (timeline->grid.present)
        SR_XML_FAIL_RETURN(ctx, "beatGrid", NULL,
                           "only one beatGrid is allowed: a second grid would "
                           "generate the same beat.N and bar.N ids");
    SrBeatGrid grid = {.present = true, .beats_per_bar = 4,
                       .source_line = sr_xml_line(ctx)};
    const char *bpm = sr_xml_required(ctx, "beatGrid", attrs, "bpm");
    if (!bpm) return;
    if (!sr_parse_double(bpm, &grid.bpm) || !(grid.bpm > 0.0) || grid.bpm > SR_MAX_BPM)
        SR_XML_FAIL_RETURN(ctx, "beatGrid", "bpm", "expected a tempo in (0,1e6] bpm");
    if (!seconds_attr(ctx, "beatGrid", attrs, "offset", &grid.offset)) return;
    const char *value = sr_xml_attr(attrs, "beatsPerBar");
    if (value && (!sr_parse_u32(value, &grid.beats_per_bar) || !grid.beats_per_bar ||
                  grid.beats_per_bar > SR_MAX_BEATS_PER_BAR))
        SR_XML_FAIL_RETURN(ctx, "beatGrid", "beatsPerBar", "expected an integer in [1,1024]");
    timeline->grid = grid;
}

/* ---- node attributes ---------------------------------------------------- */

static bool nmtoken_byte(unsigned char c) {
    return c >= 0x80 || (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') ||
           (c >= '0' && c <= '9') || c == '.' || c == '-' || c == '_' || c == ':';
}

static bool xml_space(char c) {
    return c == ' ' || c == '\t' || c == '\r' || c == '\n';
}

static bool parse_tags(ParseContext *ctx, const char *element, const char *text,
                       SrNodeTimeline *timeline) {
    const char *cursor = text;
    while (*cursor) {
        while (xml_space(*cursor)) ++cursor;
        if (!*cursor) break;
        const char *begin = cursor;
        while (*cursor && !xml_space(*cursor)) {
            if (!nmtoken_byte((unsigned char)*cursor)) {
                sr_xml_fail(ctx, element, "tags", "tags must be XML name tokens");
                return false;
            }
            ++cursor;
        }
        size_t length = (size_t)(cursor - begin);
        if (length > SR_MAX_TAG_BYTES) {
            sr_xml_fail(ctx, element, "tags", "a tag exceeds 128 bytes");
            return false;
        }
        if (timeline->tag_count == SR_MAX_NODE_TAGS) {
            sr_xml_fail(ctx, element, "tags", "a node has at most 64 tags");
            return false;
        }
        for (size_t i = 0; i < timeline->tag_count; ++i) {
            if (strlen(timeline->tags[i]) == length &&
                !memcmp(timeline->tags[i], begin, length)) {
                sr_xml_fail(ctx, element, "tags", "duplicate tag");
                return false;
            }
        }
        if (!timeline->tags) {
            timeline->tags = sr_alloc(SR_MAX_NODE_TAGS * sizeof(*timeline->tags));
            if (!timeline->tags) return fail_memory(ctx, element, "tags");
        }
        char *tag = sr_alloc(length + 1);
        if (!tag) return fail_memory(ctx, element, "tags");
        memcpy(tag, begin, length);
        timeline->tags[timeline->tag_count++] = tag;
    }
    return true;
}

static bool marker_id_attr(ParseContext *ctx, const char *element,
                           const XML_Char **attrs, const char *name,
                           const char *literal, char **target) {
    const char *value = sr_xml_attr(attrs, name);
    if (!value) return true;
    if (sr_xml_attr(attrs, literal)) {
        char message[96];
        snprintf(message, sizeof(message), "%s and %s are mutually exclusive",
                 literal, name);
        sr_xml_fail(ctx, element, name, message);
        return false;
    }
    if (!sr_id_valid(value)) {
        sr_xml_fail(ctx, element, name, "expected a marker id");
        return false;
    }
    if (!(*target = sr_strdup(value))) return fail_memory(ctx, element, name);
    return true;
}

bool sr_xml_parse_node_timeline(ParseContext *ctx, const char *element,
                                const XML_Char **attrs, SrNode *node,
                                bool sequence) {
    static const char *const names[] = {"name", "tags", "startMarker",
        "endMarker", "timeOffset", "timeScale", "timeGap"};
    bool used = sequence;
    for (size_t i = 0; i < sizeof(names) / sizeof(names[0]); ++i)
        used = used || sr_xml_attr(attrs, names[i]);
    if (!used) return true;
    if (sequence && (sr_xml_attr(attrs, "transition") ||
                     sr_xml_attr(attrs, "transitionDuration"))) {
        sr_xml_fail(ctx, element, sr_xml_attr(attrs, "transition")
                    ? "transition" : "transitionDuration", "unsupported in this build");
        return false;
    }
    SrNodeTimeline *timeline = sr_alloc(sizeof(*timeline));
    if (!timeline) return fail_memory(ctx, element, NULL);
    node->timeline = timeline;
    timeline->sequence = sequence;
    timeline->time_scale = 1.0;
    const char *value = sr_xml_attr(attrs, "name");
    if (value) {
        if (strlen(value) > SR_MAX_NODE_NAME) {
            sr_xml_fail(ctx, element, "name", "name exceeds 1024 bytes");
            return false;
        }
        if (!(timeline->name = sr_strdup(value))) return fail_memory(ctx, element, "name");
    }
    if ((value = sr_xml_attr(attrs, "tags")) && !parse_tags(ctx, element, value, timeline))
        return false;
    if (!marker_id_attr(ctx, element, attrs, "startMarker", "start",
                        &timeline->start_marker) ||
        !marker_id_attr(ctx, element, attrs, "endMarker", "end",
                        &timeline->end_marker) ||
        !seconds_attr(ctx, element, attrs, "timeOffset", &timeline->time_offset) ||
        !seconds_attr(ctx, element, attrs, "timeGap", &timeline->time_gap) ||
        !sr_xml_parse_double_attr(ctx, element, attrs, "timeScale",
                                  &timeline->time_scale)) return false;
    if (!(timeline->time_scale >= SR_MIN_TIME_SCALE) ||
        timeline->time_scale > SR_MAX_TIME_SCALE) {
        sr_xml_fail(ctx, element, "timeScale", "expected a time scale in [1e-6,1e6]");
        return false;
    }
    return true;
}

bool sr_xml_key_marker(ParseContext *ctx, const XML_Char **attrs, SrKeyframe *key) {
    const char *id = sr_xml_attr(attrs, "marker");
    if (!id) return true;
    if (!sr_id_valid(id)) {
        sr_xml_fail(ctx, "key", "marker", "expected a marker id");
        return false;
    }
    if (fabs(key->time) > SR_MAX_TIMELINE_SECONDS) {
        sr_xml_fail(ctx, "key", "time", "a marker offset is at most 1e6 seconds");
        return false;
    }
    SrTimeline *timeline = scene_timeline(ctx);
    SrStatus status = timeline ? sr_timeline_add_reference(timeline, id, &key->marker)
                               : SR_ERR_MEMORY;
    if (status != SR_OK) return fail_memory(ctx, "key", "marker");
    return true;
}

bool sr_xml_track_deferred(ParseContext *ctx, const SrTrack *track) {
    for (size_t i = 0; i < track->count; ++i) {
        if (!track->keys[i].marker) continue;
        SrTimeline *timeline = ctx->scene->timeline;
        if (!timeline->pending_tracks++) timeline->pending_line = track->keys[i].source_line;
        return true;
    }
    return false;
}

/* ---- resolution --------------------------------------------------------- */

typedef struct {
    double a, b;               /* parent = a * composition + b */
} Clock;

static bool clock_identity(Clock clock) {
    return clock.a == 1.0 && clock.b == 0.0;
}

static double clock_map(Clock clock, double t) {
    return clock_identity(clock) ? t : clock.a * t + clock.b;
}

static double clock_unmap(Clock clock, double parent) {
    return clock_identity(clock) ? parent : (parent - clock.b) / clock.a;
}

static const char *node_element(const SrNode *node) {
    if (node->timeline && node->timeline->sequence) return "sequence";
    switch (node->type) {
    case SR_NODE_GROUP: return "group";
    case SR_NODE_MEDIA: return "layer";
    case SR_NODE_PARTICLES: return "particleEmitter";
    default: return "shape";
    }
}

/* "group 'id'" or "group 'id' ("name")" for diagnostics. */
static const char *node_label(const SrNode *node, char *buffer, size_t size) {
    const char *name = node->timeline ? node->timeline->name : NULL;
    if (name)
        snprintf(buffer, size, "%s '%s' (\"%.120s\")", node_element(node),
                 node->id ? node->id : "", name);
    else
        snprintf(buffer, size, "%s '%s'", node_element(node), node->id ? node->id : "");
    return buffer;
}

static bool resolve_error(ParseContext *ctx, size_t line, const char *element,
                          const char *attribute, const char *message) {
    sr_diag_error(ctx->diag, line, element, attribute, "%s", message);
    return false;
}

static bool memory_error(ParseContext *ctx, size_t line, const char *element) {
    ctx->out_of_memory = true;
    return resolve_error(ctx, line, element, NULL, "out of memory");
}

static bool marker_time(ParseContext *ctx, const char *id, size_t line,
                        const char *element, const char *attribute, double *time) {
    SrTimeline *timeline = ctx->scene->timeline;
    SrMarkerLookup found = sr_timeline_lookup(timeline, id, time);
    if (found == SR_MARKER_LOOKUP_FOUND) return true;
    char message[512];
    if (found == SR_MARKER_LOOKUP_OUT_OF_GRID)
        snprintf(message, sizeof(message),
                 "unknown marker id '%.160s': the beat grid generates beat.1 to "
                 "beat.%llu and bar.1 to bar.%llu within the project duration",
                 id, (unsigned long long)timeline->grid.beat_count,
                 (unsigned long long)timeline->grid.bar_count);
    else if (sr_scene_id_exists(ctx->scene, id))
        snprintf(message, sizeof(message),
                 "'%.160s' is not a marker: the id names another element", id);
    else
        snprintf(message, sizeof(message), "unknown marker id '%.160s'", id);
    return resolve_error(ctx, line, element, attribute, message);
}

typedef struct {
    Clock clock;               /* composition -> parent clock */
    double start, end;         /* host interval on the parent clock */
    double absolute_start, absolute_end; /* the same, composition seconds */
    size_t line;
} Host;

/* The resolved clock of a node-hosted track (design note, "Clocks"). */
static bool configure(ParseContext *ctx, SrTrack *track, const Host *host) {
    Clock c = host->clock;
    double duration = ctx->scene->project.duration;
    size_t line = track->count ? track->keys[0].source_line : host->line;
    track->seconds_per_unit = 1.0;
    track->clock_set = false;
    track->clock_scale = 0.0;
    track->clock_offset = 0.0;
    track->domain_start = host->absolute_start;
    track->domain_end = isfinite(host->end) ? host->absolute_end : duration;
    if (track->time_base == SR_TIME_COMPOSITION) {
        if (!clock_identity(c)) {
            track->clock_set = true;
            track->clock_scale = c.a;
            track->clock_offset = c.b;
        }
    } else {
        track->clock_set = true;
        track->clock_scale = c.a;
        track->clock_offset = c.b - host->start;
        if (track->time_base == SR_TIME_NORMALIZED) {
            double end = isfinite(host->end) ? host->end : clock_map(c, duration);
            double span = end - host->start;
            if (!(span >= SR_MIN_KEY_SPACING) || !isfinite(span))
                return resolve_error(ctx, line, "animate", "timeBase",
                    "normalized animation requires a finite positive host span");
            track->clock_scale = c.a / span;
            track->clock_offset = (c.b - host->start) / span;
            track->seconds_per_unit = span;
        }
    }
    if (!isfinite(track->domain_start) || !isfinite(track->domain_end) ||
        fabs(track->clock_scale) > SR_MAX_ANIMATION_VALUE ||
        fabs(track->clock_offset) > SR_MAX_ANIMATION_VALUE)
        return resolve_error(ctx, line, "animate", "timeBase",
                             "resolved clock exceeds 1e12 limit");
    return true;
}

static int key_order(const void *left, const void *right) {
    const SrKeyframe *a = left, *b = right;
    return (a->time > b->time) - (a->time < b->time);
}

/* Moves marker keys onto their marker; returns false with a diagnostic. */
static bool snap(ParseContext *ctx, SrTrack *track, bool *snapped) {
    *snapped = false;
    for (size_t i = 0; i < track->count; ++i) {
        SrKeyframe *key = &track->keys[i];
        if (!key->marker) continue;
        const char *id = sr_timeline_reference(ctx->scene->timeline, key->marker);
        double time;
        if (!id || !marker_time(ctx, id, key->source_line, "key", "marker", &time))
            return false;
        double tau = track->clock_set
            ? time * track->clock_scale + track->clock_offset : time;
        double resolved = tau + key->time;
        if (!isfinite(resolved) || resolved < 0.0)
            return resolve_error(ctx, key->source_line, "key", "marker",
                "the snapped key time (marker plus time) must be non-negative");
        key->time = resolved;
        *snapped = true;
    }
    if (*snapped && track->count > 1)
        qsort(track->keys, track->count, sizeof(*track->keys), key_order);
    return true;
}

/* Re-validates a track after its clock or key times changed. */
static bool finish(ParseContext *ctx, SrAnimValue *value, SrAnimColor *color,
                   bool snapped, size_t fallback_line) {
    size_t line = 0;
    const char *attribute = NULL;
    const char *message = sr_xml_track_problem(value, color, &line, &attribute);
    if (snapped && ctx->scene->timeline && ctx->scene->timeline->pending_tracks)
        --ctx->scene->timeline->pending_tracks;
    if (!message) return true;
    const SrTrack *track = value ? &value->track : &color->r;
    if (!line) line = track->count ? track->keys[0].source_line : fallback_line;
    const char *element = attribute && strcmp(attribute, "property") &&
        strcmp(attribute, "interpolation") ? "key" : "animate";
    return resolve_error(ctx, line, element, attribute, message);
}

static bool resolve_value(ParseContext *ctx, SrAnimValue *value, const Host *host) {
    if (!value->track.count) return true;
    if (host && !configure(ctx, &value->track, host)) return false;
    bool snapped;
    if (!snap(ctx, &value->track, &snapped)) return false;
    if (!snapped && !host) return true;
    if (!snapped && !sr_track_extended(&value->track)) return true;
    return finish(ctx, value, NULL, snapped, host ? host->line : 0);
}

static bool resolve_color(ParseContext *ctx, SrAnimColor *color, const Host *host) {
    SrTrack *channels[] = {&color->r, &color->g, &color->b, &color->a};
    if (!color->r.count) return true;
    bool snapped = false;
    for (size_t i = 0; i < 4; ++i) {
        if (host && !configure(ctx, channels[i], host)) return false;
        bool moved;
        if (!snap(ctx, channels[i], &moved)) return false;
        snapped = snapped || moved;
    }
    if (!snapped && !host) return true;
    if (!snapped && !sr_track_extended(&color->r)) return true;
    return finish(ctx, NULL, color, snapped, host ? host->line : 0);
}

/* Every registry row of `kind` on `object`. */
static bool resolve_host(ParseContext *ctx, SrPropertyHost kind, void *object,
                         const Host *host) {
    uint32_t mask = UINT32_C(1) << kind;
    for (size_t i = 0; i < sr_property_count(); ++i) {
        const SrProperty *row = sr_property_at(i);
        if (!(row->hosts & mask)) continue;
        void *target = (unsigned char *)object + row->offset;
        bool ok = true;
        if (row->type == SR_PROPERTY_NUMBER) ok = resolve_value(ctx, target, host);
        else if (row->type == SR_PROPERTY_COLOR) ok = resolve_color(ctx, target, host);
        if (!ok) return false;
    }
    return true;
}

static bool resolve_node_tracks(ParseContext *ctx, SrNode *node, const Host *host) {
    if (!resolve_host(ctx, sr_property_node_host(node), node, host)) return false;
    for (size_t i = 0; i < node->mask_count; ++i)
        if (!resolve_host(ctx, SR_PROPERTY_MASK, &node->masks[i], host)) return false;
    for (size_t i = 0; i < node->modifier_count; ++i) {
        SrModifier *modifier = &node->modifiers[i];
        if (!resolve_host(ctx, SR_PROPERTY_MODIFIER, modifier, host)) return false;
        size_t points = modifier->points ? (size_t)modifier->rows * modifier->cols * 2 : 0;
        for (size_t j = 0; j < points; ++j)
            if (!resolve_value(ctx, &modifier->points[j], host)) return false;
    }
    return true;
}

typedef struct {
    Clock base;                /* clock of the enclosing group's children */
    double shift;              /* sequence item: its slot start on `base` */
    bool item;                 /* placed by a sequence */
} Placement;

static bool resolve_node(ParseContext *ctx, SrNode *node, Placement placement,
                         double *authored_end);

/* Composition seconds of a parent-clock instant. Marker instants are kept
 * exactly; other instants are unmapped once, so a sequence junction with no
 * gap gives bitwise-equal end and start. */
static double absolute(Placement placement, double parent) {
    double slot = placement.shift != 0.0 ? placement.shift + parent : parent;
    return clock_unmap(placement.base, slot);
}

static bool node_interval(ParseContext *ctx, SrNode *node, Placement placement,
                          Clock parent, Host *host) {
    const SrNodeTimeline *timeline = node->timeline;
    host->clock = parent;
    host->line = node->source_line;
    host->start = node->start_time;
    host->end = node->end_time;
    bool start_marker = timeline && timeline->start_marker;
    bool end_marker = timeline && timeline->end_marker;
    char label[256], message[512];
    node_label(node, label, sizeof(label));
    const char *element = node_element(node);
    if (placement.item && start_marker) {
        snprintf(message, sizeof(message), "%s is placed by its sequence; "
                 "startMarker is not allowed on a sequence item", label);
        return resolve_error(ctx, node->source_line, element, "startMarker", message);
    }
    double time = 0.0;
    if (start_marker) {
        if (!marker_time(ctx, timeline->start_marker, node->source_line, element,
                         "startMarker", &time)) return false;
        host->start = clock_map(parent, time);
        host->absolute_start = time;
    } else {
        host->absolute_start = absolute(placement, host->start);
    }
    if (end_marker) {
        if (!marker_time(ctx, timeline->end_marker, node->source_line, element,
                         "endMarker", &time)) return false;
        host->end = clock_map(parent, time);
        host->absolute_end = time;
    } else {
        host->absolute_end = isfinite(host->end) ? absolute(placement, host->end)
                                                 : host->end;
    }
    const char *attribute = start_marker ? "startMarker" : end_marker ? "endMarker"
                                                                      : "start";
    if ((start_marker || end_marker) &&
        (!isfinite(host->start) || host->start < 0.0 || !(host->end > host->start))) {
        snprintf(message, sizeof(message), "%s: the resolved interval "
                 "[%.9g, %.9g) must satisfy 0 <= start < end", label, host->start,
                 host->end);
        return resolve_error(ctx, node->source_line, element, attribute, message);
    }
    if (!isfinite(host->absolute_start) ||
        fabs(host->absolute_start) > SR_MAX_CLOCK_OFFSET ||
        isnan(host->absolute_end) || !(host->absolute_end > host->absolute_start)) {
        snprintf(message, sizeof(message), "%s: the interval in composition "
                 "seconds is empty or exceeds 1e12 seconds", label);
        return resolve_error(ctx, node->source_line, element, attribute, message);
    }
    return true;
}

static bool children_clock(ParseContext *ctx, const SrNode *node, Clock parent,
                           double start, Clock *children) {
    *children = parent;
    const SrNodeTimeline *timeline = node->timeline;
    if (!timeline || (timeline->time_scale == 1.0 && timeline->time_offset == 0.0))
        return true;
    double q = timeline->time_scale, o = timeline->time_offset;
    if (q == 1.0) {
        children->b = parent.b + o;
    } else {
        children->a = parent.a * q;
        children->b = (parent.b - start) * q + start + o;
    }
    char label[256], message[512];
    if (!(children->a >= SR_MIN_TIME_SCALE) || children->a > SR_MAX_TIME_SCALE) {
        snprintf(message, sizeof(message), "%s: the combined time scale of nested "
                 "groups must stay within [1e-6,1e6]", node_label(node, label, sizeof(label)));
        return resolve_error(ctx, node->source_line, node_element(node), "timeScale", message);
    }
    if (!isfinite(children->b) || fabs(children->b) > SR_MAX_CLOCK_OFFSET) {
        snprintf(message, sizeof(message), "%s: the resolved clock offset exceeds 1e12 "
                 "seconds", node_label(node, label, sizeof(label)));
        return resolve_error(ctx, node->source_line, node_element(node), "timeOffset", message);
    }
    return true;
}

static bool clock_consumers(ParseContext *ctx, SrNode *node, Clock clock) {
    if (clock.a == 1.0) return true;
    char label[256], message[512];
    node_label(node, label, sizeof(label));
    if (node->body.type == SR_BODY_DYNAMIC || node->body.type == SR_BODY_KINEMATIC ||
        node->soft_body.enabled) {
        snprintf(message, sizeof(message), "unsupported in this build: %s simulates "
                 "physics on the project clock and cannot be under a group "
                 "timeScale other than 1", label);
        return resolve_error(ctx, node->source_line, node_element(node),
                             node->soft_body.enabled ? "softBody" : "rigidBody", message);
    }
    if (node->type == SR_NODE_MEDIA) {
        node->speed *= clock.a;
        if (!isfinite(node->speed) || !(node->speed > 0.0)) {
            snprintf(message, sizeof(message), "%s: speed times the group time "
                     "scale must be finite and positive", label);
            return resolve_error(ctx, node->source_line, "layer", "speed", message);
        }
    }
    return true;
}

static bool resolve_children(ParseContext *ctx, SrNode *node, Clock clock,
                             double start) {
    const SrNodeTimeline *timeline = node->timeline;
    if (!timeline || !timeline->sequence) {
        for (size_t i = 0; i < node->child_count; ++i) {
            double end;
            if (!resolve_node(ctx, node->children[i], (Placement){clock, 0.0, false},
                              &end)) return false;
        }
        return true;
    }
    /* Children are still in document order: z sorting happens afterwards. */
    double cursor = start;
    for (size_t i = 0; i < node->child_count; ++i) {
        SrNode *child = node->children[i];
        double end;
        if (!resolve_node(ctx, child, (Placement){clock, cursor, true}, &end))
            return false;
        if (i + 1 == node->child_count) break;
        char label[256], message[512];
        if (!isfinite(end)) {
            snprintf(message, sizeof(message), "%s needs an end so the next "
                     "sequence item can start", node_label(child, label, sizeof(label)));
            return resolve_error(ctx, child->source_line, node_element(child), "end",
                                 message);
        }
        cursor = cursor + end + timeline->time_gap;
        if (!isfinite(cursor) || fabs(cursor) > SR_MAX_CLOCK_OFFSET) {
            snprintf(message, sizeof(message), "%s: the sequence position exceeds "
                     "1e12 seconds", node_label(node, label, sizeof(label)));
            return resolve_error(ctx, node->source_line, "sequence", "timeGap", message);
        }
    }
    return true;
}

static bool resolve_node(ParseContext *ctx, SrNode *node, Placement placement,
                         double *authored_end) {
    Clock parent = placement.base;
    if (placement.shift != 0.0) parent.b = placement.base.b - placement.shift;
    Host host;
    if (!node_interval(ctx, node, placement, parent, &host)) return false;
    *authored_end = host.end;
    if (!resolve_node_tracks(ctx, node, &host)) return false;
    node->start_time = host.absolute_start;
    node->end_time = host.absolute_end;
    node->clock_scale = parent.a;
    node->clock_offset = parent.b;
    if (!clock_consumers(ctx, node, parent)) return false;
    if (node->type != SR_NODE_GROUP) return true;
    Clock children;
    if (!children_clock(ctx, node, parent, host.start, &children)) return false;
    return resolve_children(ctx, node, children, host.start);
}

static bool collision(ParseContext *ctx, const char *id, size_t line,
                      const char *element) {
    if (!id || !sr_timeline_generates(ctx->scene->timeline, id)) return true;
    char message[256];
    snprintf(message, sizeof(message), "id '%.64s' equals an id generated by "
             "beatGrid", id);
    return resolve_error(ctx, line, element, "id", message);
}

static bool node_collisions(ParseContext *ctx, const SrNode *node) {
    if (node != ctx->scene->root &&
        !collision(ctx, node->id, node->source_line, node_element(node))) return false;
    for (size_t i = 0; i < node->child_count; ++i)
        if (!node_collisions(ctx, node->children[i])) return false;
    return true;
}

/* S4: generated ids join the id table before any reference resolves. */
static bool generated_collisions(ParseContext *ctx) {
    SrScene *scene = ctx->scene;
    size_t grid = scene->timeline->grid.source_line;
    if (!node_collisions(ctx, scene->root)) return false;
    for (size_t i = 0; i < scene->asset_count; ++i)
        if (!collision(ctx, scene->assets[i].id, scene->assets[i].source_line, "asset"))
            return false;
    for (size_t i = 0; i < scene->audio.track_count; ++i)
        if (!collision(ctx, scene->audio.tracks[i].id, scene->audio.tracks[i].source_line,
                       "audioTrack")) return false;
    for (size_t i = 0; i < scene->camera_count; ++i)
        if (!collision(ctx, scene->cameras[i].id, scene->cameras[i].source_line, "camera"))
            return false;
    for (size_t i = 0; i < scene->light_count; ++i)
        if (!collision(ctx, scene->lights[i].id, scene->lights[i].source_line, "light"))
            return false;
    for (size_t i = 0; i < scene->object3d_count; ++i)
        if (!collision(ctx, scene->objects3d[i].id, scene->objects3d[i].source_line,
                       "object3D")) return false;
    for (size_t i = 0; i < scene->effect_count; ++i)
        if (!collision(ctx, scene->effects[i].id, scene->effects[i].source_line, "effect"))
            return false;
    for (size_t i = 0; i < scene->material_count; ++i)
        if (!collision(ctx, scene->materials[i].id, grid, "material")) return false;
    for (size_t i = 0; i < scene->physics.constraint_count; ++i)
        if (!collision(ctx, scene->physics.constraints[i].id,
                       scene->physics.constraints[i].source_line, "constraint"))
            return false;
    for (size_t i = 0; i < scene->physics.field_count; ++i)
        if (!collision(ctx, scene->physics.fields[i].id, grid, "forceField"))
            return false;
    return true;
}

static bool resolve_shared(ParseContext *ctx) {
    SrScene *scene = ctx->scene;
    for (size_t i = 0; i < scene->material_count; ++i)
        if (!resolve_host(ctx, SR_PROPERTY_MATERIAL, &scene->materials[i], NULL))
            return false;
    for (size_t i = 0; i < scene->audio.track_count; ++i)
        if (!resolve_host(ctx, SR_PROPERTY_AUDIO_TRACK, &scene->audio.tracks[i], NULL))
            return false;
    for (size_t i = 0; i < scene->camera_count; ++i)
        if (!resolve_host(ctx, SR_PROPERTY_CAMERA, &scene->cameras[i], NULL))
            return false;
    for (size_t i = 0; i < scene->light_count; ++i)
        if (!resolve_host(ctx, SR_PROPERTY_LIGHT, &scene->lights[i], NULL))
            return false;
    for (size_t i = 0; i < scene->effect_count; ++i)
        if (!resolve_host(ctx, SR_PROPERTY_EFFECT, &scene->effects[i], NULL))
            return false;
    for (size_t i = 0; i < scene->object3d_count; ++i)
        if (!resolve_host(ctx, SR_PROPERTY_OBJECT3D, &scene->objects3d[i], NULL))
            return false;
    for (size_t i = 0; i < scene->physics.field_count; ++i)
        if (!resolve_host(ctx, SR_PROPERTY_FIELD, &scene->physics.fields[i], NULL))
            return false;
    return true;
}

bool sr_xml_resolve_timeline(ParseContext *ctx) {
    SrScene *scene = ctx->scene;
    SrTimeline *timeline = scene->timeline;
    if (timeline) {
        const char *problem = NULL;
        size_t line = 0;
        SrStatus status = sr_timeline_prepare(timeline, scene->project.duration,
                                              &problem, &line);
        if (status == SR_ERR_MEMORY) return memory_error(ctx, 1, "markers");
        if (status != SR_OK)
            return resolve_error(ctx, line, line == timeline->grid.source_line
                                 ? "beatGrid" : "marker", NULL, problem);
        if (timeline->grid.present && !generated_collisions(ctx)) return false;
    }
    Placement top = {{1.0, 0.0}, 0.0, false};
    for (size_t i = 0; i < scene->root->child_count; ++i) {
        double end;
        if (!resolve_node(ctx, scene->root->children[i], top, &end)) return false;
    }
    if (timeline && timeline->reference_count && !resolve_shared(ctx)) return false;
    if (timeline && timeline->pending_tracks)
        return resolve_error(ctx, timeline->pending_line, "key", "marker",
                             "unsupported in this build: key marker on this "
                             "animation host");
    return true;
}
