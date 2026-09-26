/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SCENE_RENDER_PAINT_H
#define SCENE_RENDER_PAINT_H

#include "scene_render/compositor.h"
#include "scene_render/scene.h"

/* B1-4 gradient paints (docs/design/b1-4-shapes-paints.md, "Paints"). */

#define SR_MAX_PAINTS 4096u
#define SR_MAX_GRADIENT_STOPS 256u
#define SR_MAX_PAINT_COORDINATE 1e9

typedef enum { SR_PAINT_LINEAR, SR_PAINT_RADIAL, SR_PAINT_CONIC } SrPaintType;
typedef enum { SR_SPREAD_PAD, SR_SPREAD_REFLECT, SR_SPREAD_REPEAT } SrSpread;
typedef enum { SR_PAINT_UNITS_OBJECT, SR_PAINT_UNITS_USER } SrPaintUnits;
typedef enum { SR_INTERP_LINEAR, SR_INTERP_SRGB, SR_INTERP_OKLAB,
               SR_INTERP_OKLCH } SrInterpolationSpace;

typedef struct {
    SrAnimValue offset;         /* 0..1 along the gradient */
    SrAnimColor color;          /* straight working-space colour */
    SrAnimValue opacity;        /* multiplies the colour's alpha */
    SrAnimValue midpoint;       /* colour hint of the segment to the next stop */
    size_t source_line;
} SrGradientStop;

/* Owned by the scene; immutable after loading. Unused geometry fields keep
 * their schema defaults. */
typedef struct SrPaint {
    char *id;
    SrPaintType type;
    SrSpread spread;
    SrPaintUnits units;
    SrInterpolationSpace space;
    bool dither;
    SrAnimValue rotation;       /* degrees, clockwise about the box centre */
    SrAnimValue x1, y1, x2, y2; /* linear */
    SrAnimValue cx, cy, r;      /* radial and conic centre, radial radius */
    SrAnimValue fx, fy, fr;     /* radial focal circle */
    bool fx_set, fy_set;        /* attribute or track; otherwise cx/cy */
    SrAnimValue aspect;         /* radial rx / ry */
    SrAnimValue angle;          /* conic start angle, degrees from the top */
    SrGradientStop *stops;
    size_t stop_count, stop_capacity;
    uint64_t dither_key;        /* seeded Bayer offset, set when resolved */
    size_t source_line;
} SrPaint;

/* A paint evaluated at one time for one painted box: immutable, shared by
 * row workers. Stop colours are premultiplied in the interpolation space. */
#define SR_PAINT_EVAL_STOPS SR_MAX_GRADIENT_STOPS
typedef struct {
    const SrPaint *paint;
    SrPaintType type;
    SrSpread spread;
    SrInterpolationSpace space;
    bool dither;
    unsigned dither_x, dither_y;
    bool linear_light;          /* project blend space */
    SrColorSpace working;
    bool srgb_gamut;            /* working primaries are sRGB/Rec.709 */
    double to_srgb[3][3], from_srgb[3][3];  /* linear working <-> sRGB */
    bool degenerate;            /* linear: coincident points, last stop */
    bool unpainted;             /* radial: identical circles paint nothing */
    /* local point -> gradient space: g = A * p + b */
    double a00, a01, a10, a11, b0, b1;
    double x1, y1, dx, dy, inv_length2;     /* linear */
    double cx, cy, r, fx, fy, fr, aspect;   /* radial */
    bool concentric;
    double angle;                           /* conic, degrees */
    size_t stop_count;
    double offsets[SR_PAINT_EVAL_STOPS];
    double hints[SR_PAINT_EVAL_STOPS];      /* colour-hint exponent, or <0 */
    double midpoints[SR_PAINT_EVAL_STOPS];
    double values[SR_PAINT_EVAL_STOPS][4];  /* premultiplied channels */
    double alpha[SR_PAINT_EVAL_STOPS];
    bool powerless[SR_PAINT_EVAL_STOPS];    /* oklch hue missing */
} SrPaintEval;

bool sr_paint_has_animation(const SrPaint *paint);
/* Frees the paint's owned storage (not the struct). */
void sr_paint_free(SrPaint *paint);
/* Default-initialized paint of a type (schema defaults). */
void sr_paint_init(SrPaint *paint, SrPaintType type);
/* Seeded Bayer offset key: splitmix64(FNV-1a64(id, 0, "paint.dither") XOR
 * project seed). */
uint64_t sr_paint_dither_key(const char *id, uint64_t project_seed);

/* Evaluates `paint` at `time` for the painted box [0,w] x [0,h]. Returns
 * SR_ERR_RENDER with a diagnostic naming the paint when an evaluated value
 * is outside its bounds (nonfinite or beyond SR_MAX_PAINT_COORDINATE). */
SrStatus sr_paint_eval(const SrPaint *paint, const SrProject *project,
                       double time, double width, double height,
                       SrPaintEval *out, SrDiagnostics *diag);

/* Premultiplied blend-space colour at local point (x, y); (px, py) are the
 * integer pixel coordinates of the receiving raster (for dithering). */
void sr_paint_sample(const SrPaintEval *eval, double x, double y,
                     int px, int py, float out[4]);

/* Gradient parameter t before spread for local point (x, y); false when the
 * point is not painted (radial outside the cone). Exposed for tests. */
bool sr_paint_parameter(const SrPaintEval *eval, double x, double y, double *t);

/* Oklab conversions of linear sRGB-gamut values (tests and interpolation). */
void sr_oklab_from_linear_srgb(const double rgb[3], double lab[3]);
void sr_linear_srgb_from_oklab(const double lab[3], double rgb[3]);

/* Fills `frame` with the paint evaluated for the frame box, row-parallel. */
SrStatus sr_paint_fill_frame(const SrPaint *paint, const SrProject *project,
                             double time, SrFrame *frame, unsigned threads,
                             SrDiagnostics *diag);

#endif
