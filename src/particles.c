#include "particles_internal.h"
#include "scene_render/color.h"
#include "scene_render/random.h"
#include "random_internal.h"

#include <math.h>
#include <pthread.h>
#include <stdlib.h>
#include <string.h>

/* Candidate particles examined per frame are bounded so a pathological
 * rate x lifetime cannot stall a render. */
#define SR_MAX_PARTICLE_CANDIDATES UINT64_C(20000000)
/* Animated emission rates are integrated on this fixed grid (seconds from
 * the emitter start), independent of the frame being rendered. */
#define RATE_STEP (1.0 / 240.0)
/* Most grid cells one evaluation may integrate: the whole accepted project
 * duration (renders never ask for later times). */
#define SR_MAX_PARTICLE_RATE_CELLS ((uint64_t)(SR_MAX_DURATION * 240.0) + 16)
/* Grid cells between checkpoints of the newest-first walk. */
#define WALK_BLOCK 1024
/* Grid index standing for "beyond any time this emitter can reach". */
#define FAR_CELL (UINT64_C(1) << 62)

double sr_particles_random(uint64_t seed, uint64_t index, unsigned stream) {
    return sr_random_particle_value_inline(seed, index, stream);
}

uint64_t sr_particles_seed(const SrScene *scene, const SrNode *node) {
    if (node->particle_seed_set) return node->particle_seed;
    return sr_random_particle_seed(scene->project.seed, node->id);
}

/* Upper bound of an animated value over all time (curves may overshoot
 * their keys only through cubic-bezier control values). */
static double anim_upper_bound(const SrAnimValue *value) {
    if (!value->track.count) return value->base;
    const SrTrack *track = &value->track;
    double high = track->keys[0].value;
    for (size_t i = 0; i < track->count; ++i) {
        const SrKeyframe *a = &track->keys[i];
        high = fmax(high, a->value);
        if (i + 1 < track->count && a->curve == SR_CURVE_BEZIER) {
            double b = track->keys[i + 1].value;
            double over = fmax(0.0, fmax(fmax(a->y1 - 1.0, a->y2 - 1.0),
                                         fmax(-a->y1, -a->y2)));
            high = fmax(high, fmax(a->value, b) + fabs(b - a->value) * over);
        }
    }
    return high;
}

typedef struct {
    SrParticle *items;
    size_t count, capacity, limit;
    SrCompositeResources *resources;
    uint64_t candidate_work;
} Collector;

static bool collect(Collector *collector, SrParticle particle) {
    if (collector->count == collector->capacity) {
        size_t capacity = collector->capacity ? collector->capacity * 2 : 64;
        if (capacity > collector->limit) capacity = collector->limit;
        SrParticle *items = sr_composite_realloc(collector->resources,
            collector->items, capacity, sizeof(*items), 0);
        if (!items) return false;
        collector->items = items;
        collector->capacity = capacity;
    }
    collector->items[collector->count++] = particle;
    return true;
}

/* Evaluates particle `index` born at local time `birth`; returns false when
 * it is not alive at local time `now`. */
static bool particle_at(const SrScene *scene, const SrNode *node, uint64_t seed,
                        uint64_t index, double birth, double now,
                        SrParticle *out) {
    double age = now - birth;
    if (age < 0.0) return false;
    double tb = node->start_time + birth;
    double lifetime = fmax(1e-6, sr_anim_eval(&node->particle_lifetime, tb) +
        node->particle_lifetime_variance *
            (2.0 * sr_particles_random(seed, index, 0) - 1.0));
    if (age > lifetime) return false;
    double f = age / lifetime;
    double jitter = 2.0 * sr_particles_random(seed, index, 1) - 1.0;
    double angle = (sr_anim_eval(&node->particle_direction, tb) +
                    sr_anim_eval(&node->particle_spread, tb) * jitter) * SR_PI / 180.0;
    double base_speed = sr_anim_eval(&node->particle_speed, tb);
    double variance = node->particle_speed_variance_set
        ? node->particle_speed_variance
        : base_speed * node->particle_speed_var_factor;
    double speed = base_speed * node->particle_speed_factor +
        variance * (2.0 * sr_particles_random(seed, index, 2) - 1.0);
    double ex = node->particle_emitter_width *
                (sr_particles_random(seed, index, 3) - 0.5);
    double ey = node->particle_emitter_height *
                (sr_particles_random(seed, index, 4) - 0.5);
    out->index = index;
    out->x = ex + cos(angle) * speed * age +
             0.5 * node->particle_gravity_x * age * age;
    out->y = ey + sin(angle) * speed * age +
             0.5 * node->particle_gravity_y * age * age;
    if (node->particle_wobble != 0.0)
        out->x += sin(age * node->particle_wobble_frequency + jitter) *
                  node->particle_wobble;
    double size = fmax(0.0, sr_anim_eval(&node->particle_size, tb));
    double size_end = node->particle_size_end_set ? node->particle_size_end
                    : node->particle_grow ? size * (1.0 + lifetime) : size;
    out->radius = size + (size_end - size) * f;
    SrColor birth_color = sr_anim_color_eval(&node->particle_color, tb);
    SrColor end_color;
    if (node->particle_color_end_set) {
        end_color = sr_anim_color_eval(&node->particle_color_end, tb);
    } else {
        end_color = birth_color;
        end_color.a = 0.0;
    }
    out->color = sr_color_mix_linear(birth_color, end_color, f,
                                     scene->project.working_color_space);
    return true;
}

/* Animated rates: the cumulative emitted count N is the trapezoid integral
 * of the (clamped non-negative) rate on the fixed grid t_k = k * RATE_STEP
 * from the emitter start, linear inside each cell. Grid cells before the
 * first key and after the last see a constant rate, so N is closed form
 * there; between them the cache holds N at each key's grid boundary and
 * only the segment being walked is integrated per evaluation. */
struct SrParticleRateCache {
    size_t count;               /* middle boundaries: cell[0] .. cell[count-1] */
    size_t ready;               /* total[0 .. ready-1] are computed */
    uint64_t *cell;
    double *total;
    uint64_t first_cell;        /* cells [0, first_cell) run at first_rate */
    uint64_t last_cell;         /* cells [last_cell, inf) run at last_rate */
    double first_rate, last_rate;
    /* Walk checkpoints: N at cell[j] + b * WALK_BLOCK for the middle segment
     * starting at boundary j, stored at marks[mark_offset[j] + b]; the first
     * mark_ready[j] are computed. They are the very running sums the
     * sequential trapezoid walk from cell[j] produces, so a walk may resume
     * from them instead of re-integrating the segment from its start.
     * mark_capacity is 0 when the key span is too long to keep them. */
    uint64_t *mark_offset;
    uint64_t *mark_ready;
    double *marks;
    uint64_t mark_capacity;
};

static pthread_mutex_t rate_cache_lock = PTHREAD_MUTEX_INITIALIZER;

static double rate_at(const SrNode *node, uint64_t k) {
    return fmax(0.0, sr_anim_eval(&node->particle_rate,
                                  node->start_time + (double)k * RATE_STEP));
}

/* Smallest grid index whose time is >= t (0 when t <= the start). */
static uint64_t cell_at_or_after(const SrNode *node, double t) {
    double x = (t - node->start_time) / RATE_STEP;
    if (!(x > 0.0)) return 0;
    if (!(x < (double)FAR_CELL)) return FAR_CELL;
    uint64_t k = (uint64_t)ceil(x);
    while (k > 0 && node->start_time + (double)(k - 1) * RATE_STEP >= t) --k;
    while (node->start_time + (double)k * RATE_STEP < t) ++k;
    return k;
}

/* Largest grid index whose time is <= t (0 when t <= the start). */
static uint64_t cell_at_or_before(const SrNode *node, double t) {
    double x = (t - node->start_time) / RATE_STEP;
    if (!(x > 0.0)) return 0;
    if (!(x < (double)FAR_CELL)) return FAR_CELL;
    uint64_t k = (uint64_t)floor(x);
    while (node->start_time + (double)(k + 1) * RATE_STEP <= t) ++k;
    while (k > 0 && node->start_time + (double)k * RATE_STEP > t) --k;
    return k;
}

typedef struct {
    size_t slots, bytes;
    uint64_t marks, first, last;
} RateLayout;

static bool rate_layout(const SrNode *node, RateLayout *layout) {
    const SrTrack *track = &node->particle_rate.track;
    size_t keys = track->count;
    bool extended = sr_track_extended(track);
    if (keys > (SIZE_MAX - sizeof(struct SrParticleRateCache)) /
                   (sizeof(uint64_t) + sizeof(double)) - 2)
        return false;
    size_t slots = extended ? 2 : keys + 2;
    double first = track->keys[0].time;
    uint64_t first_cell = first > node->start_time ? cell_at_or_before(node, first) : 0;
    uint64_t last_cell = cell_at_or_after(node, track->keys[keys - 1].time);
    if (extended) {
        first_cell = 0;
        double end = track->domain_end > node->start_time
            ? track->domain_end : node->start_time + SR_MAX_DURATION;
        last_cell = cell_at_or_after(node, end);
    }
    if (last_cell < first_cell) last_cell = first_cell;
    /* Each middle segment of length L keeps ceil(L / WALK_BLOCK) marks; the
     * segments tile [first_cell, last_cell), so this bounds their sum. */
    uint64_t mark_capacity = 0;
    if (last_cell - first_cell <= SR_MAX_PARTICLE_RATE_CELLS)
        mark_capacity = (last_cell - first_cell) / WALK_BLOCK + slots;
    if (slots > (SIZE_MAX - sizeof(struct SrParticleRateCache) -
                 mark_capacity * sizeof(double)) /
                    (3 * sizeof(uint64_t) + sizeof(double)))
        return false;
    *layout = (RateLayout){slots,
        sizeof(struct SrParticleRateCache) + slots * (3 * sizeof(uint64_t) + sizeof(double)) +
        (size_t)mark_capacity * sizeof(double), mark_capacity, first_cell, last_cell};
    return true;
}

static bool track_storage(const SrTrack *track) {
    return track->count <= SR_MAX_TRACK_KEYS && (!track->count || track->keys);
}

static bool particle_metadata(const SrNode *node, uint64_t *candidate_work) {
    const SrTrack *tracks[] = {&node->particle_rate.track, &node->particle_lifetime.track,
        &node->particle_speed.track, &node->particle_spread.track, &node->particle_size.track,
        &node->particle_direction.track, &node->particle_color.r, &node->particle_color.g,
        &node->particle_color.b, &node->particle_color.a, &node->particle_color_end.r,
        &node->particle_color_end.g, &node->particle_color_end.b, &node->particle_color_end.a};
    *candidate_work = 64 + (sizeof(SrParticle) + 3) / 4;
    for (size_t i = 0; i < sizeof(tracks) / sizeof(tracks[0]); ++i) {
        if (!track_storage(tracks[i])) return false;
        if (i) *candidate_work += tracks[i]->count ? 64 : 1;
    }
    if (node->particle_max > SR_MAX_PARTICLE_OUTPUT || !isfinite(node->start_time) ||
        fabs(node->start_time) > SR_MAX_DURATION) return false;
    const SrTrack *rate = tracks[0];
    if (rate->count &&
        (!isfinite(rate->keys[0].time) || !isfinite(rate->keys[rate->count - 1].time) ||
         fabs(rate->keys[0].time) > SR_MAX_ANIMATION_TIME ||
         fabs(rate->keys[rate->count - 1].time) > SR_MAX_ANIMATION_TIME)) return false;
    if (sr_track_extended(rate) &&
        (!isfinite(rate->domain_end) || fabs(rate->domain_end) > SR_MAX_DURATION)) return false;
    return true;
}

bool sr_particles_cache_bound(const SrNode *node, uint64_t *bytes) {
    *bytes = 0;
    uint64_t candidate_work;
    if (!particle_metadata(node, &candidate_work)) return false;
    if (!node->particle_seed_set && node->id) {
        size_t i = 0;
        while (i < SR_MAX_PARTICLE_ID_BYTES && node->id[i]) ++i;
        if (node->id[i]) return false;
    }
    if (node->particle_rate.track.count) {
        RateLayout layout;
        if (!rate_layout(node, &layout)) return false;
        *bytes = layout.bytes;
    }
    return true;
}

void sr_particles_invalidate(SrNode *node) {
    if (!node) return;
    pthread_mutex_lock(&rate_cache_lock);
    free(node->particle_rate_cache);
    node->particle_rate_cache = NULL;
    pthread_mutex_unlock(&rate_cache_lock);
}

static bool particle_failure(SrCompositeResources *resources, const char *message) {
    return sr_composite_resource_fail(resources, SR_ERR_RENDER, message);
}

static bool particle_admit(const SrNode *node, double time,
                            SrCompositeResources *resources, uint64_t *candidate_work) {
    *candidate_work = 0;
    if (!resources) return true;
    if (!particle_metadata(node, candidate_work) || !isfinite(time) ||
        fabs(time) > SR_MAX_DURATION)
        return particle_failure(resources, "invalid bounded particle metadata or clock");
    double now = time - node->start_time;
    if (now < 0) return true;
    if (!sr_composite_work(resources, node->particle_lifetime.track.count, 64) ||
        !sr_composite_work(resources, 1, 128)) return false;
    if (!node->particle_seed_set && node->id) {
        for (size_t i = 0; node->id[i]; ++i) {
            if (i == SR_MAX_PARTICLE_ID_BYTES)
                return particle_failure(resources, "particle seed id exceeds 1048576 bytes");
            if (!sr_composite_work(resources, 2, 1)) return false;
        }
    }
    if (node->particle_rate.track.count) {
        RateLayout layout;
        if (!rate_layout(node, &layout))
            return particle_failure(resources, "particle cache capacity overflow");
        uint64_t top = (uint64_t)(floor(now / RATE_STEP) + 1.0);
        if (top > layout.last) top = layout.last;
        uint64_t cells = top > layout.first ? top - layout.first : 0;
        if (cells > SR_MAX_PARTICLE_RATE_CELLS)
            return particle_failure(resources, "particle sampling grid limit exceeded");
        /* Cold totals, checkpoints and regeneration; starting samples,
         * reverse cell visits and metadata/copy dispatch remain covered. */
        if (!sr_composite_work(resources, cells + layout.slots + 1, 4 * 80) ||
            !sr_composite_work(resources, (layout.bytes + 3) / 4, 1)) return false;
    }
    return true;
}

static struct SrParticleRateCache *rate_cache_build(const SrNode *node) {
    const SrTrack *track = &node->particle_rate.track;
    size_t keys = track->count;
    bool extended = sr_track_extended(track);
    RateLayout layout;
    if (!rate_layout(node, &layout)) return NULL;
    size_t slots = layout.slots;
    uint64_t first_cell = layout.first, last_cell = layout.last;
    struct SrParticleRateCache *cache = sr_alloc(layout.bytes);
    if (!cache) return NULL;
    cache->cell = (uint64_t *)(void *)(cache + 1);
    cache->total = (double *)(void *)(cache->cell + slots);
    cache->mark_offset = (uint64_t *)(void *)(cache->total + slots);
    cache->mark_ready = cache->mark_offset + slots;
    cache->marks = (double *)(void *)(cache->mark_ready + slots);
    cache->mark_capacity = layout.marks;
    cache->first_rate = fmax(0.0, track->keys[0].value);
    cache->last_rate = fmax(0.0, track->keys[keys - 1].value);
    if (extended) {
        cache->first_rate = rate_at(node, first_cell);
        cache->last_rate = rate_at(node, last_cell);
    }
    cache->first_cell = first_cell;
    cache->last_cell = last_cell;
    cache->count = 0;
    cache->cell[cache->count++] = cache->first_cell;
    for (size_t i = 1; !extended && i + 1 < keys; ++i) {
        uint64_t k = cell_at_or_after(node, track->keys[i].time);
        if (k > cache->cell[cache->count - 1] && k < cache->last_cell)
            cache->cell[cache->count++] = k;
    }
    if (cache->last_cell > cache->cell[cache->count - 1])
        cache->cell[cache->count++] = cache->last_cell;
    uint64_t offset = 0;
    for (size_t j = 0; j < cache->count; ++j) {
        cache->mark_offset[j] = offset;
        cache->mark_ready[j] = 0;
        if (j + 1 < cache->count)
            offset += (cache->cell[j + 1] - cache->cell[j] + WALK_BLOCK - 1) / WALK_BLOCK;
    }
    if (offset > cache->mark_capacity) cache->mark_capacity = 0;
    double step = 0.5 * (cache->first_rate + cache->first_rate) * RATE_STEP;
    cache->total[0] = (double)cache->first_cell * step;
    cache->ready = 1;
    return cache;
}

/* Sequential trapezoid sum of N from cell `from` (where N = `value`) to
 * cell `to`, optionally storing N every WALK_BLOCK cells in `marks`. */
static double integrate(const SrNode *node, uint64_t from, uint64_t to,
                        double value, double *marks) {
    double previous = rate_at(node, from);
    for (uint64_t k = from; k < to; ++k) {
        if (marks && (k - from) % WALK_BLOCK == 0)
            marks[(k - from) / WALK_BLOCK] = value;
        double next = rate_at(node, k + 1);
        value += 0.5 * (previous + next) * RATE_STEP;
        previous = next;
    }
    return value;
}

/* Copies N at the middle boundaries up to and including index `upto`,
 * extending the node's cache first when needed. */
static SrStatus rate_totals(const SrNode *node, size_t upto,
                            struct SrParticleRateCache *out, double *totals) {
    SrStatus status = SR_OK;
    pthread_mutex_lock(&rate_cache_lock);
    SrNode *owner = (SrNode *)node;     /* the cache is logically const */
    if (!owner->particle_rate_cache) owner->particle_rate_cache = rate_cache_build(node);
    struct SrParticleRateCache *cache = owner->particle_rate_cache;
    if (!cache) {
        status = SR_ERR_MEMORY;
    } else {
        if (upto >= cache->count) upto = cache->count - 1;
        while (cache->ready <= upto) {
            size_t j = cache->ready;
            cache->total[j] = integrate(node, cache->cell[j - 1], cache->cell[j],
                                        cache->total[j - 1], NULL);
            cache->ready = j + 1;
        }
        *out = *cache;
        if (totals) memcpy(totals, cache->total, (upto + 1) * sizeof(*totals));
    }
    pthread_mutex_unlock(&rate_cache_lock);
    return status;
}

/* Copies into `out` the walk checkpoints of middle segment `j` (whose N at
 * cell[j] is total[j], already computed) for cells below `to`: the
 * (to - cell[j] - 1) / WALK_BLOCK + 1 values integrate(cell[j], to,
 * total[j], out) would store, extending the node's cache first when
 * needed. Returns false when the cache keeps no checkpoints. */
static bool segment_marks(const SrNode *node, size_t j, uint64_t to, double *out) {
    bool ok = false;
    pthread_mutex_lock(&rate_cache_lock);
    struct SrParticleRateCache *cache = node->particle_rate_cache;
    if (cache && cache->mark_capacity && j + 1 < cache->count && j < cache->ready &&
        to > cache->cell[j] && to <= cache->cell[j + 1]) {
        uint64_t from = cache->cell[j];
        uint64_t needed = (to - from - 1) / WALK_BLOCK + 1;
        double *marks = cache->marks + cache->mark_offset[j];
        uint64_t ready = cache->mark_ready[j];
        if (ready == 0) marks[ready++] = cache->total[j];
        /* The same sequential sum as integrate(), resumed at the newest
         * stored checkpoint: identical operations in identical order. */
        while (ready < needed) {
            uint64_t begin = from + (ready - 1) * WALK_BLOCK;
            double value = marks[ready - 1];
            double previous = rate_at(node, begin);
            for (uint64_t k = begin; k < begin + WALK_BLOCK; ++k) {
                double next = rate_at(node, k + 1);
                value += 0.5 * (previous + next) * RATE_STEP;
                previous = next;
            }
            marks[ready++] = value;
        }
        cache->mark_ready[j] = ready;
        memcpy(out, marks, (size_t)needed * sizeof(*out));
        ok = true;
    }
    pthread_mutex_unlock(&rate_cache_lock);
    return ok;
}

typedef struct {
    const SrScene *scene;
    const SrNode *node;
    uint64_t seed;
    double now, oldest;         /* local time; births before oldest are dead */
    Collector collector;
    uint64_t examined;
    bool ok, done;
} Walk;

/* Offers particle `index` born at local `birth`; false once the walk must
 * stop (cap reached, candidate budget spent or allocation failure). */
static bool offer(Walk *walk, double index, double birth) {
    if (walk->collector.count >= walk->collector.limit ||
        ++walk->examined > SR_MAX_PARTICLE_CANDIDATES) {
        if (walk->examined > SR_MAX_PARTICLE_CANDIDATES && walk->collector.resources) {
            particle_failure(walk->collector.resources, "particle candidate limit exceeded");
            walk->ok = false;
        }
        walk->done = true;
        return false;
    }
    if (!sr_composite_work(walk->collector.resources, 1, walk->collector.candidate_work)) {
        walk->ok = false;
        walk->done = true;
        return false;
    }
    SrParticle particle;
    if (particle_at(walk->scene, walk->node, walk->seed, (uint64_t)index, birth,
                    walk->now, &particle) &&
        !collect(&walk->collector, particle)) {
        walk->ok = false;
        walk->done = true;
        return false;
    }
    return true;
}

/* Newest first over indices [low, high] of a constant-rate stretch where
 * N = n0 + rate * (u - u0). */
static void walk_constant(Walk *walk, double low, double high, double n0,
                          double u0, double rate) {
    for (double i = high; i >= low && !walk->done; i -= 1.0)
        if (!offer(walk, i, u0 + (i - n0) / rate)) return;
}

/* Newest first over the grid cells [from, to) of one middle segment whose
 * N at cell `from` is `value`. */
static SrStatus walk_segment(Walk *walk, size_t segment, uint64_t from,
                             uint64_t to, double value) {
    uint64_t cells = to - from;
    size_t blocks = (size_t)(cells / WALK_BLOCK) + 1;
    SrCompositeResources *resources = walk->collector.resources;
    double *marks = sr_composite_alloc(resources, blocks, sizeof(*marks), 0);
    double *buffer = sr_composite_alloc(resources, WALK_BLOCK + 1, sizeof(*buffer), 0);
    if (!marks || !buffer) {
        sr_composite_free(resources, marks);
        sr_composite_free(resources, buffer);
        return sr_composite_resource_status(resources);
    }
    if (!segment_marks(walk->node, segment, to, marks))
        integrate(walk->node, from, to, value, marks);
    SrStatus status = SR_OK;
    for (size_t b = blocks; b-- > 0 && !walk->done;) {
        uint64_t begin = from + (uint64_t)b * WALK_BLOCK;
        if (begin >= to) continue;
        uint64_t end = begin + WALK_BLOCK < to ? begin + WALK_BLOCK : to;
        buffer[0] = marks[b];
        double previous = rate_at(walk->node, begin);
        for (uint64_t k = begin; k < end; ++k) {
            double next = rate_at(walk->node, k + 1);
            buffer[k - begin + 1] = buffer[k - begin] + 0.5 * (previous + next) * RATE_STEP;
            previous = next;
        }
        for (uint64_t k = end; k-- > begin && !walk->done;) {
            double t0 = (double)k * RATE_STEP;
            if (t0 + RATE_STEP < walk->oldest) { walk->done = true; break; }
            double n0 = buffer[k - begin], n1 = buffer[k - begin + 1];
            if (!isfinite(n0) || !isfinite(n1) || n0 < 0.0 ||
                n1 > SR_MAX_PARTICLE_INDEX) {
                status = SR_ERR_RENDER;
                goto done;
            }
            if (!(n1 > n0)) continue;
            for (double i = ceil(n1) - 1.0, low = ceil(n0); i >= low; i -= 1.0)
                if (!offer(walk, i, t0 + (i - n0) / (n1 - n0) * RATE_STEP)) break;
        }
    }
done:
    sr_composite_free(resources, marks);
    sr_composite_free(resources, buffer);
    return status;
}

static SrStatus walk_keyed(Walk *walk) {
    const SrNode *node = walk->node;
    double now = walk->now;
    SrCompositeResources *resources = walk->collector.resources;
    struct SrParticleRateCache info;
    SrStatus status = rate_totals(node, 0, &info, NULL);
    if (status != SR_OK) return status;
    /* Newest grid cell that can hold a birth at or before `now`. */
    double top_exact = floor(now / RATE_STEP) + 1.0;
    uint64_t top = top_exact < (double)FAR_CELL ? (uint64_t)top_exact : FAR_CELL;
    /* After the last key: constant rate, closed form. */
    if (top >= info.last_cell) {
        size_t last = info.count - 1;
        if (info.last_cell - info.first_cell > SR_MAX_PARTICLE_RATE_CELLS) return SR_ERR_RENDER;
        double *totals = sr_composite_alloc(resources, info.count, sizeof(*totals), 0);
        if (!totals) return sr_composite_resource_status(resources);
        status = rate_totals(node, last, &info, totals);
        double base = totals[last];
        sr_composite_free(resources, totals);
        if (status != SR_OK) return status;
        double u0 = (double)info.last_cell * RATE_STEP, rate = info.last_rate;
        if (rate > 0.0) {
            double high = floor(base + rate * (now - u0)) + 1.0;
            if (!isfinite(high) || high > SR_MAX_PARTICLE_INDEX) {
                if (resources) {
                    particle_failure(resources, "particle emission index exceeds 9e15");
                    return SR_ERR_RENDER;
                }
                return sr_track_extended(&node->particle_rate.track)
                    ? SR_ERR_RENDER : SR_OK;
            }
            double low = fmax(ceil(base),
                              floor(base + rate * fmax(0.0, walk->oldest - u0)) - 1.0);
            walk_constant(walk, low, high, base, u0, rate);
            if (low > ceil(base)) walk->done = true;
        }
        top = info.last_cell;
    }
    /* Between the first and last keys: the grid, one segment at a time. */
    if (!walk->done && top > info.first_cell) {
        if (top - info.first_cell > SR_MAX_PARTICLE_RATE_CELLS) return SR_ERR_RENDER;
        size_t segment = 0;
        while (segment + 1 < info.count && info.cell[segment + 1] < top) ++segment;
        double *totals = sr_composite_alloc(resources, segment + 1, sizeof(*totals), 0);
        if (!totals) return sr_composite_resource_status(resources);
        status = rate_totals(node, segment, &info, totals);
        for (size_t j = segment + 1; status == SR_OK && j-- > 0 && !walk->done;) {
            uint64_t end = j + 1 < info.count && info.cell[j + 1] < top
                ? info.cell[j + 1] : top;
            if (end > info.cell[j])
                status = walk_segment(walk, j, info.cell[j], end, totals[j]);
        }
        sr_composite_free(resources, totals);
        if (status != SR_OK) return status;
    }
    /* Before the first key: constant rate from the start, N = rate * u. */
    if (!walk->done && info.first_cell > 0 && info.first_rate > 0.0) {
        double u0 = 0.0, rate = info.first_rate;
        double limit = (double)(top < info.first_cell ? top : info.first_cell) * RATE_STEP;
        double high = fmin(ceil(info.total[0]) - 1.0,
                           floor(rate * fmin(now, limit)) + 1.0);
        double low = fmax(0.0, floor(rate * fmax(0.0, walk->oldest)) - 1.0);
        if (isfinite(high) && high <= SR_MAX_PARTICLE_INDEX)
            walk_constant(walk, low, high, 0.0, u0, rate);
        else if (resources) {
            particle_failure(resources, "particle emission index exceeds 9e15");
            return SR_ERR_RENDER;
        }
    }
    return SR_OK;
}

static SrStatus particles_eval(const SrScene *scene, const SrNode *node,
                                 double time, SrCompositeResources *resources,
                                 SrParticle **particles, size_t *count) {
    *particles = NULL;
    *count = 0;
    uint64_t candidate_work;
    if (!particle_admit(node, time, resources, &candidate_work)) return SR_ERR_RENDER;
    double now = time - node->start_time;
    if (!(now >= 0.0) || !isfinite(now)) return SR_OK;
    uint64_t seed = sr_particles_seed(scene, node);
    double lifetime = sr_track_extended(&node->particle_lifetime.track)
        ? sr_anim_upper_bound(&node->particle_lifetime, node->start_time, time)
        : anim_upper_bound(&node->particle_lifetime);
    if (!isfinite(lifetime)) return SR_ERR_RENDER;
    double longest = fmax(1e-6, lifetime +
                                node->particle_lifetime_variance);
    Collector collector = {.limit = node->particle_max ? node->particle_max : 1,
        .resources = resources, .candidate_work = candidate_work};
    uint64_t examined = 0;
    bool ok = true;
    if (!node->particle_rate.track.count) {
        double rate = node->particle_rate.base;
        if (!(rate > 0.0)) return SR_OK;
        double last = floor(now * rate);
        double first = fmax(0.0, floor((now - longest) * rate) - 1.0);
        if (!isfinite(last) || last > SR_MAX_PARTICLE_INDEX) {
            if (!resources) return SR_OK;
            particle_failure(resources, "particle emission index exceeds 9e15");
            return SR_ERR_RENDER;
        }
        for (double k = last; k >= first && ok; k -= 1.0) {
            if (collector.count >= collector.limit || ++examined > SR_MAX_PARTICLE_CANDIDATES) {
                if (examined > SR_MAX_PARTICLE_CANDIDATES && resources) {
                    particle_failure(resources, "particle candidate limit exceeded");
                    ok = false;
                }
                break;
            }
            if (!sr_composite_work(resources, 1, candidate_work)) { ok = false; break; }
            uint64_t index = (uint64_t)k;
            SrParticle particle;
            if (particle_at(scene, node, seed, index, (double)index / rate, now,
                            &particle))
                ok = collect(&collector, particle);
        }
    } else {
        Walk walk = {scene, node, seed, now, now - longest, collector, 0, true, false};
        SrStatus status = walk_keyed(&walk);
        collector = walk.collector;
        if (status != SR_OK) {
            sr_composite_free(resources, collector.items);
            return status;
        }
        ok = walk.ok;
    }
    if (!ok) {
        sr_composite_free(resources, collector.items);
        return sr_composite_resource_status(resources);
    }
    /* Reversal copies three records per pair; reserve a whole-record bound
     * plus the compositor's subsequent per-particle draw preparation. */
    if (!sr_composite_work(resources, collector.count,
                            3 * sizeof(SrParticle) / 4 + 64)) {
        sr_composite_free(resources, collector.items);
        return SR_ERR_RENDER;
    }
    /* Collected newest first; draw oldest first. */
    for (size_t a = 0, b = collector.count; a + 1 < b; ++a, --b) {
        SrParticle swap = collector.items[a];
        collector.items[a] = collector.items[b - 1];
        collector.items[b - 1] = swap;
    }
    *particles = collector.items;
    *count = collector.count;
    return SR_OK;
}

SrStatus sr_particles_eval(const SrScene *scene, const SrNode *node,
                           double time, SrParticle **particles, size_t *count) {
    return particles_eval(scene, node, time, NULL, particles, count);
}

SrStatus sr_particles_eval_composite(const SrScene *scene, const SrNode *node,
                                      double time, SrCompositeResources *resources,
                                      SrParticle **particles, size_t *count) {
    SrCompositeOwner previous = sr_composite_owner(resources,
        (SrCompositeOwner){node->source_line, "particleEmitter", "rate/lifetime"});
    SrStatus status = particles_eval(scene, node, time, resources, particles, count);
    sr_composite_owner(resources, previous);
    return status;
}
