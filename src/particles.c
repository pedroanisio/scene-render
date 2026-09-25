#include "scene_render/particles.h"
#include "scene_render/color.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>

/* Candidate particles examined per frame are bounded so a pathological
 * rate x lifetime cannot stall a render. */
#define MAX_CANDIDATES UINT64_C(20000000)
/* Animated emission rates are integrated on this fixed grid (seconds from
 * the emitter start), independent of the frame being rendered. */
#define RATE_STEP (1.0 / 240.0)

static uint64_t splitmix64(uint64_t x) {
    x += UINT64_C(0x9E3779B97F4A7C15);
    x = (x ^ (x >> 30)) * UINT64_C(0xBF58476D1CE4E5B9);
    x = (x ^ (x >> 27)) * UINT64_C(0x94D049BB133111EB);
    return x ^ (x >> 31);
}

double sr_particles_random(uint64_t seed, uint64_t index, unsigned stream) {
    uint64_t h = splitmix64(seed ^ splitmix64(index * 8 + stream));
    return (double)(h >> 11) * (1.0 / 9007199254740992.0);
}

uint64_t sr_particles_seed(const SrScene *scene, const SrNode *node) {
    if (node->particle_seed_set) return node->particle_seed;
    uint64_t hash = UINT64_C(1469598103934665603);
    for (const unsigned char *p = (const unsigned char *)(node->id ? node->id : "");
         *p; ++p) {
        hash ^= *p;
        hash *= UINT64_C(1099511628211);
    }
    return scene->project.seed ^ hash;
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
} Collector;

static bool collect(Collector *collector, SrParticle particle) {
    if (collector->count == collector->capacity) {
        size_t capacity = collector->capacity ? collector->capacity * 2 : 64;
        if (capacity > collector->limit) capacity = collector->limit;
        SrParticle *items = sr_realloc(collector->items, capacity * sizeof(*items));
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

SrStatus sr_particles_eval(const SrScene *scene, const SrNode *node,
                           double time, SrParticle **particles, size_t *count) {
    *particles = NULL;
    *count = 0;
    double now = time - node->start_time;
    if (!(now >= 0.0) || !isfinite(now)) return SR_OK;
    uint64_t seed = sr_particles_seed(scene, node);
    double longest = fmax(1e-6, anim_upper_bound(&node->particle_lifetime) +
                                node->particle_lifetime_variance);
    Collector collector = {NULL, 0, 0, node->particle_max ? node->particle_max : 1};
    uint64_t examined = 0;
    bool ok = true;
    if (!node->particle_rate.track.count) {
        double rate = node->particle_rate.base;
        if (!(rate > 0.0)) return SR_OK;
        double last = floor(now * rate);
        double first = fmax(0.0, floor((now - longest) * rate) - 1.0);
        if (!isfinite(last) || last > 9e15) return SR_OK;
        for (double k = last; k >= first && ok; k -= 1.0) {
            if (collector.count >= collector.limit || ++examined > MAX_CANDIDATES) break;
            uint64_t index = (uint64_t)k;
            SrParticle particle;
            if (particle_at(scene, node, seed, index, (double)index / rate, now,
                            &particle))
                ok = collect(&collector, particle);
        }
    } else {
        /* Cumulative emitted count N on a fixed grid; particle i is born
         * where N reaches i, linearly within its grid cell. */
        size_t cells = (size_t)ceil(now / RATE_STEP) + 1;
        double *total = sr_alloc((cells + 1) * sizeof(*total));
        if (!total) return SR_ERR_MEMORY;
        total[0] = 0.0;
        double previous = fmax(0.0, sr_anim_eval(&node->particle_rate, node->start_time));
        for (size_t k = 0; k < cells; ++k) {
            double next = fmax(0.0, sr_anim_eval(&node->particle_rate,
                node->start_time + (double)(k + 1) * RATE_STEP));
            total[k + 1] = total[k] + 0.5 * (previous + next) * RATE_STEP;
            previous = next;
        }
        for (size_t k = cells; k-- > 0 && ok;) {
            double t0 = (double)k * RATE_STEP;
            if (t0 + RATE_STEP < now - longest) break;
            double n0 = total[k], n1 = total[k + 1];
            if (!(n1 > n0)) continue;
            double high = ceil(n1) - 1.0, low = ceil(n0);
            for (double i = high; i >= low && ok; i -= 1.0) {
                if (collector.count >= collector.limit || ++examined > MAX_CANDIDATES)
                    break;
                double birth = t0 + (i - n0) / (n1 - n0) * RATE_STEP;
                SrParticle particle;
                if (particle_at(scene, node, seed, (uint64_t)i, birth, now,
                                &particle))
                    ok = collect(&collector, particle);
            }
            if (collector.count >= collector.limit || examined > MAX_CANDIDATES) break;
        }
        free(total);
    }
    if (!ok) {
        free(collector.items);
        return SR_ERR_MEMORY;
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
