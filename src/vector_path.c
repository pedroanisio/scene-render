#include "scene_render/vector_path.h"
#include "vector_path_internal.h"

#include <ctype.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>

typedef SrPathPoint Point;
typedef SrPathContour Contour;
typedef SrPreparedPath Path;

void sr_prepared_path_free(SrPreparedPath *path) {
    if (!path) return;
    for (size_t i = 0; i < path->count; ++i) free(path->items[i].points);
    free(path->items);
    *path = (SrPreparedPath){0};
}

static Contour *add_contour(Path *path) {
    if (path->count == path->capacity) {
        size_t capacity=path->capacity?path->capacity*2:4;
        Contour *items=sr_realloc(path->items,capacity*sizeof(*items));
        if(!items)return NULL;
        for(size_t i=path->capacity;i<capacity;++i)items[i]=(Contour){0};
        path->items=items;path->capacity=capacity;
    }
    return &path->items[path->count++];
}

static bool add_point(Contour *contour, Point point) {
    if(contour->count==contour->capacity){size_t capacity=contour->capacity?contour->capacity*2:16;
        Point *points=sr_realloc(contour->points,capacity*sizeof(*points));
        if(!points)return false;
        contour->points=points;contour->capacity=capacity;}
    contour->points[contour->count++]=point;return true;
}

static void separators(const char **cursor) {
    while (isspace((unsigned char)**cursor) || **cursor==',') ++*cursor;
}

static bool number(const char **cursor, double *value) {
    separators(cursor);char *tail=NULL;*value=strtod(*cursor,&tail);
    if(tail==*cursor||!isfinite(*value))return false;
    *cursor=tail;return true;
}

static bool pair(const char **cursor,double *x,double *y){return number(cursor,x)&&number(cursor,y);}

static bool flatten_cubic(Contour *contour,Point start,Point a,Point b,Point end){
    for(int i=1;i<=16;++i){double t=i/16.0,u=1.0-t;
        Point p={u*u*u*start.x+3*u*u*t*a.x+3*u*t*t*b.x+t*t*t*end.x,
                 u*u*u*start.y+3*u*u*t*a.y+3*u*t*t*b.y+t*t*t*end.y};
        if(!add_point(contour,p))return false;}return true;
}

static bool flatten_quadratic(Contour *contour,Point start,Point control,Point end){
    for(int i=1;i<=12;++i){double t=i/12.0,u=1.0-t;
        Point p={u*u*start.x+2*u*t*control.x+t*t*end.x,
                 u*u*start.y+2*u*t*control.y+t*t*end.y};
        if(!add_point(contour,p))return false;}return true;
}

/* SR_OK, SR_ERR_ASSET for malformed path data, SR_ERR_MEMORY when a point
 * or contour cannot be stored. */
static SrStatus parse_path(const char *text,Path *path) {
    const char *cursor=text;char command=0;Contour *contour=NULL;Point current={0},first={0};
    while(1){separators(&cursor);if(!*cursor)break;
        if(isalpha((unsigned char)*cursor))command=*cursor++;
        else if(!command)return SR_ERR_ASSET;
        bool relative=islower((unsigned char)command)!=0;char op=(char)toupper((unsigned char)command);
        if(op=='Z'){if(contour&&contour->count&&
            (current.x!=first.x||current.y!=first.y)&&!add_point(contour,first))return SR_ERR_MEMORY;
            if(contour)contour->closed=true;
            current=first;command=0;continue;}
        if(op=='M'||op=='L'){double x,y;if(!pair(&cursor,&x,&y))return SR_ERR_ASSET;
            if(relative){x+=current.x;y+=current.y;}current=(Point){x,y};
            if(op=='M'){contour=add_contour(path);if(!contour)return SR_ERR_MEMORY;first=current;
                command=relative?'l':'L';}
            if(!contour)return SR_ERR_ASSET;
            if(!add_point(contour,current))return SR_ERR_MEMORY;
            continue;}
        if(!contour||!contour->count)return SR_ERR_ASSET;
        if(op=='H'){double x;if(!number(&cursor,&x))return SR_ERR_ASSET;if(relative)x+=current.x;
            current.x=x;if(!add_point(contour,current))return SR_ERR_MEMORY;continue;}
        if(op=='V'){double y;if(!number(&cursor,&y))return SR_ERR_ASSET;if(relative)y+=current.y;
            current.y=y;if(!add_point(contour,current))return SR_ERR_MEMORY;continue;}
        if(op=='C'){double ax,ay,bx,by,x,y;if(!pair(&cursor,&ax,&ay)||!pair(&cursor,&bx,&by)||!pair(&cursor,&x,&y))return SR_ERR_ASSET;
            if(relative){ax+=current.x;ay+=current.y;bx+=current.x;by+=current.y;x+=current.x;y+=current.y;}
            Point end={x,y};if(!flatten_cubic(contour,current,(Point){ax,ay},(Point){bx,by},end))return SR_ERR_MEMORY;current=end;continue;}
        if(op=='Q'){double cx,cy,x,y;if(!pair(&cursor,&cx,&cy)||!pair(&cursor,&x,&y))return SR_ERR_ASSET;
            if(relative){cx+=current.x;cy+=current.y;x+=current.x;y+=current.y;}
            Point end={x,y};if(!flatten_quadratic(contour,current,(Point){cx,cy},end))return SR_ERR_MEMORY;current=end;continue;}
        return SR_ERR_ASSET;
    }
    if(!path->count)return SR_ERR_ASSET;
    for(size_t i=0;i<path->count;++i)if(path->items[i].count<2)return SR_ERR_ASSET;
    return SR_OK;
}

SrStatus sr_prepared_path_parse(const char *text, SrPreparedPath *out) {
    if (!out) return SR_ERR_ARGUMENT;
    *out = (SrPreparedPath){0};
    if (!text) return SR_ERR_ASSET;
    SrStatus status = parse_path(text, out);
    if (status != SR_OK) sr_prepared_path_free(out);
    return status;
}

SrStatus sr_vector_path_check(const char *text) {
    Path path = {0};
    SrStatus status = sr_prepared_path_parse(text, &path);
    sr_prepared_path_free(&path);
    return status;
}

bool sr_vector_path_valid(const char *text) {
    return sr_vector_path_check(text)==SR_OK;
}

/* Signed-area accumulation (as in font-rs): each edge adds, to the cells it
 * crosses, the area it sweeps; a running sum along each row then yields the
 * winding-weighted coverage of every pixel exactly. Rows carry two guard
 * columns so edges clamped to x = width stay in bounds. */
typedef struct {
    float *cells;
    int width, height;   /* width includes the two guard columns */
} Accumulator;

static void accumulate_edge(Accumulator *acc, double x0, double y0,
                            double x1, double y1) {
    if (y0 == y1) return;
    float direction = 1.0f;
    if (y0 > y1) {
        double tx = x0, ty = y0;
        x0 = x1; y0 = y1; x1 = tx; y1 = ty;
        direction = -1.0f;
    }
    if (y1 <= 0.0 || y0 >= acc->height) return;
    double dxdy = (x1 - x0) / (y1 - y0), x = x0;
    /* Clamp in double before converting so huge coordinates stay defined. */
    int ystart = (int)floor(y0 < 0.0 ? 0.0 : y0);
    if (y0 < 0.0) x -= y0 * dxdy;
    int yend = (int)fmin(ceil(y1), (double)acc->height);
    /* Endpoints are clamped to [0, limit] by clipped_edge, but intersections
     * computed from the slope can drift by rounding; keep them in the grid. */
    double limit = acc->width - 2;
    x = fmin(fmax(x, 0.0), limit);
    for (int y = ystart; y < yend; ++y) {
        float *row = acc->cells + (size_t)y * (size_t)acc->width;
        double dy = fmin(y + 1.0, y1) - fmax((double)y, y0);
        double xnext = fmin(fmax(x + dxdy * dy, 0.0), limit);
        float d = (float)dy * direction;
        double lo = x < xnext ? x : xnext, hi = x < xnext ? xnext : x;
        double lof = floor(lo);
        int li = (int)lof, hi_c = (int)ceil(hi);
        if (hi_c <= li + 1) {
            float xm = (float)(0.5 * (x + xnext) - lof);
            row[li] += d - d * xm;
            row[li + 1] += d * xm;
        } else {
            float sl = (float)(1.0 / (hi - lo));
            float f0 = (float)(lo - lof);
            float a0 = 0.5f * sl * (1.0f - f0) * (1.0f - f0);
            float f1 = (float)(hi - hi_c + 1);
            float am = 0.5f * sl * f1 * f1;
            row[li] += d * a0;
            if (hi_c == li + 2) {
                row[li + 1] += d * (1.0f - a0 - am);
            } else {
                float a1 = sl * (1.5f - f0);
                row[li + 1] += d * (a1 - a0);
                for (int xi = li + 2; xi < hi_c - 1; ++xi) row[xi] += d * sl;
                float a2 = a1 + (float)(hi_c - li - 3) * sl;
                row[hi_c - 1] += d * (1.0f - a2 - am);
            }
            row[hi_c] += d * am;
        }
        x = xnext;
    }
}

/* Splits the edge where it crosses x = 0 and x = limit, then clamps each
 * piece into [0, limit]: a piece left of the grid becomes a vertical edge at
 * x = 0 (it covers every visible pixel to its right), one right of it lands
 * in the guard columns. */
static void clipped_edge(Accumulator *acc, Point a, Point b) {
    double limit = acc->width - 2;
    double cuts[4] = {0.0, 1.0, 1.0, 1.0};
    size_t count = 1;
    if (a.x != b.x) {
        double t0 = (0.0 - a.x) / (b.x - a.x), t1 = (limit - a.x) / (b.x - a.x);
        if (t0 > 0.0 && t0 < 1.0) cuts[count++] = t0;
        if (t1 > 0.0 && t1 < 1.0) cuts[count++] = t1;
    }
    cuts[count++] = 1.0;
    if (count == 4 && cuts[1] > cuts[2]) {
        double t = cuts[1]; cuts[1] = cuts[2]; cuts[2] = t;
    }
    for (size_t i = 0; i + 1 < count; ++i) {
        double ta = cuts[i], tb = cuts[i + 1];
        double xa = a.x + (b.x - a.x) * ta, ya = a.y + (b.y - a.y) * ta;
        double xb = a.x + (b.x - a.x) * tb, yb = a.y + (b.y - a.y) * tb;
        if (i == 0) { xa = a.x; ya = a.y; }
        if (i + 2 == count) { xb = b.x; yb = b.y; }
        xa = fmin(fmax(xa, 0.0), limit);
        xb = fmin(fmax(xb, 0.0), limit);
        accumulate_edge(acc, xa, ya, xb, yb);
    }
}

static void accumulate_polygon(Accumulator *acc, const Point *points,
                               size_t count) {
    for (size_t i = 0; i < count; ++i)
        clipped_edge(acc, points[i], points[(i + 1) % count]);
}

static float resolve_winding(float sum, SrFillRule rule) {
    float value = fabsf(sum);
    if (rule == SR_FILL_EVENODD) {
        value = fmodf(value, 2.0f);
        if (value > 1.0f) value = 2.0f - value;
    }
    return value > 1.0f ? 1.0f : value;
}

static void resolve(const Accumulator *acc, SrFillRule rule, float *coverage) {
    int visible = acc->width - 2;
    for (int y = 0; y < acc->height; ++y) {
        float sum = 0.0f;
        const float *row = acc->cells + (size_t)y * (size_t)acc->width;
        for (int x = 0; x < visible; ++x) {
            sum += row[x];
            coverage[(size_t)y * (size_t)visible + (size_t)x] =
                resolve_winding(sum, rule);
        }
    }
}

/* Round-joined, round-capped stroke coverage: the stroke is the set of
 * points within half_width of the polyline, so coverage follows from the
 * distance to the nearest segment. Unlike a union of
 * accumulated quads and discs, overlapping joins are never counted twice. */
static double segment_distance(Point a, Point b, double px, double py) {
    double dx = b.x - a.x, dy = b.y - a.y, len2 = dx * dx + dy * dy;
    double t = len2 > 0.0 ? ((px - a.x) * dx + (py - a.y) * dy) / len2 : 0.0;
    t = fmin(fmax(t, 0.0), 1.0);
    return hypot(px - (a.x + t * dx), py - (a.y + t * dy));
}

/* Overlap of a 1 px box across the stroke, [d-.5, d+.5], with the band
 * [-half_width, half_width]: exact for straight runs, also for strokes
 * thinner than a pixel. */
static float stroke_value(double best, double half_width) {
    double coverage = fmin(best + 0.5, half_width) -
                      fmax(best - 0.5, -half_width);
    return (float)fmin(fmax(coverage, 0.0), 1.0);
}

/* The reference evaluation: every pixel against every segment. */
static void stroke_coverage_all(const Path *path, double half_width,
                                uint32_t width, uint32_t height, float *stroke) {
    for (uint32_t y = 0; y < height; ++y) {
        for (uint32_t x = 0; x < width; ++x) {
            double px = x + 0.5, py = y + 0.5, best = HUGE_VAL;
            for (size_t c = 0; c < path->count; ++c) {
                const Contour *contour = &path->items[c];
                size_t n = contour->count;
                size_t segments = contour->closed ? n : n - 1;
                for (size_t i = 0; i < segments; ++i) {
                    double d = segment_distance(contour->points[i],
                                                contour->points[(i + 1) % n],
                                                px, py);
                    if (d < best) best = d;
                }
            }
            stroke[(size_t)y * width + x] = stroke_value(best, half_width);
        }
    }
}

/* Pixels are evaluated in STROKE_TILE x STROKE_TILE tiles against only the
 * segments whose bounding box lies within `reach` of the tile's pixel
 * centres. A skipped segment is farther than reach > half_width + 0.5 from
 * every centre of the tile, so it cannot be the minimum unless the minimum
 * itself yields zero coverage; and a minimum over any superset of the
 * segments that can be nearest is the same value. Segments provably farther
 * from every centre than some other segment (bounds via the tile centre)
 * are skipped too. The result is therefore bit-identical to
 * stroke_coverage_all. */
#define STROKE_TILE 16u
/* Coordinates beyond this (or non-finite) use the reference path, so the
 * rounding slack below stays far under one pixel. */
#define STROKE_COORD_LIMIT 1e12

typedef struct { Point a, b; double x0, y0, x1, y1; } StrokeSegment;

/* SR_ERR_MEMORY when the tile index cannot be allocated (reported, never
 * silently degraded); out-of-range geometry uses the all-segments loop. */
static SrStatus stroke_coverage(const Path *path, double half_width,
                                uint32_t width, uint32_t height, float *stroke) {
    size_t total = 0;
    bool bounded = half_width < STROKE_COORD_LIMIT;
    for (size_t c = 0; c < path->count && bounded; ++c) {
        const Contour *contour = &path->items[c];
        total += contour->closed ? contour->count : contour->count - 1;
        for (size_t i = 0; i < contour->count; ++i)
            if (!(fabs(contour->points[i].x) <= STROKE_COORD_LIMIT) ||
                !(fabs(contour->points[i].y) <= STROKE_COORD_LIMIT))
                bounded = false;
    }
    if (!bounded || total > UINT32_MAX) {
        stroke_coverage_all(path, half_width, width, height, stroke);
        return SR_OK;
    }
    StrokeSegment *segments = sr_alloc(total * sizeof(*segments));
    uint32_t *band = segments ? sr_alloc(total * sizeof(*band)) : NULL;
    uint32_t *tile = band ? sr_alloc(total * sizeof(*tile)) : NULL;
    double *centre = tile ? sr_alloc(total * sizeof(*centre)) : NULL;
    if (!centre) {
        free(segments); free(band); free(tile); free(centre);
        return SR_ERR_MEMORY;
    }
    /* Segments in the reference order (contour by contour, i -> i+1). */
    size_t k = 0;
    for (size_t c = 0; c < path->count; ++c) {
        const Contour *contour = &path->items[c];
        size_t n = contour->count;
        size_t count = contour->closed ? n : n - 1;
        for (size_t i = 0; i < count; ++i, ++k) {
            StrokeSegment *s = &segments[k];
            s->a = contour->points[i];
            s->b = contour->points[(i + 1) % n];
            s->x0 = fmin(s->a.x, s->b.x); s->x1 = fmax(s->a.x, s->b.x);
            s->y0 = fmin(s->a.y, s->b.y); s->y1 = fmax(s->a.y, s->b.y);
        }
    }
    /* One pixel of slack covers the rounding of segment_distance many
     * times over at these magnitudes. */
    double reach = half_width + 0.5 + 1.0;
    for (uint32_t ty = 0; ty < height; ty += STROKE_TILE) {
        uint32_t tyend = height - ty < STROKE_TILE ? height : ty + STROKE_TILE;
        double cy0 = ty + 0.5 - reach, cy1 = (tyend - 1) + 0.5 + reach;
        size_t bands = 0;
        for (size_t i = 0; i < total; ++i)
            if (segments[i].y1 >= cy0 && segments[i].y0 <= cy1)
                band[bands++] = (uint32_t)i;
        for (uint32_t tx = 0; tx < width; tx += STROKE_TILE) {
            uint32_t txend = width - tx < STROKE_TILE ? width : tx + STROKE_TILE;
            double cx0 = tx + 0.5 - reach, cx1 = (txend - 1) + 0.5 + reach;
            /* Every pixel centre p of the tile is within `radius` of the
             * tile centre c, so dist(p, s) >= dist(c, s) - radius. */
            double hx = 0.5 * (double)(txend - 1 - tx), hy = 0.5 * (double)(tyend - 1 - ty);
            double centre_x = tx + 0.5 + hx, centre_y = ty + 0.5 + hy;
            double radius = hypot(hx, hy), limit = reach + radius;
            size_t near = 0;
            double nearest = HUGE_VAL;
            for (size_t i = 0; i < bands; ++i) {
                const StrokeSegment *s = &segments[band[i]];
                if (!(s->x1 >= cx0 && s->x0 <= cx1)) continue;
                double d = segment_distance(s->a, s->b, centre_x, centre_y);
                if (d <= limit) {
                    tile[near] = band[i];
                    centre[near++] = d;
                    if (d < nearest) nearest = d;
                }
            }
            /* Every centre p also has some segment within nearest + radius,
             * so a segment farther than that from all of them (with a pixel
             * of slack for rounding) is never p's minimum. */
            double keep = fmin(limit, nearest + 2.0 * radius + 1.0);
            size_t tiles = 0;
            for (size_t i = 0; i < near; ++i)
                if (centre[i] <= keep) tile[tiles++] = tile[i];
            for (uint32_t y = ty; y < tyend; ++y) {
                for (uint32_t x = tx; x < txend; ++x) {
                    double px = x + 0.5, py = y + 0.5, best = HUGE_VAL;
                    for (size_t i = 0; i < tiles; ++i) {
                        const StrokeSegment *s = &segments[tile[i]];
                        double d = segment_distance(s->a, s->b, px, py);
                        if (d < best) best = d;
                    }
                    stroke[(size_t)y * width + x] = stroke_value(best, half_width);
                }
            }
        }
    }
    free(segments);
    free(band);
    free(tile);
    free(centre);
    return SR_OK;
}

SrStatus sr_vector_path_coverage(const char *text, SrFillRule rule,
                                 double stroke_width, uint32_t width,
                                 uint32_t height, float *fill, float *stroke) {
    if (!text || !fill || !width || !height || width > INT32_MAX - 2 ||
        height > INT32_MAX ||
        (size_t)height > SIZE_MAX / sizeof(float) / ((size_t)width + 2))
        return SR_ERR_ARGUMENT;
    Path path = {0};
    SrStatus status = sr_prepared_path_parse(text, &path);
    if (status == SR_OK)
        status = sr_prepared_path_coverage(&path, rule, stroke_width, width,
                                           height, fill, stroke);
    sr_prepared_path_free(&path);
    return status;
}

SrStatus sr_prepared_path_coverage(const SrPreparedPath *path, SrFillRule rule,
                                   double stroke_width, uint32_t width,
                                   uint32_t height, float *fill, float *stroke) {
    if (!path || !path->count || !path->items || !fill || !width || !height ||
        width > INT32_MAX - 2 || height > INT32_MAX ||
        (size_t)height > SIZE_MAX / sizeof(float) / ((size_t)width + 2))
        return SR_ERR_ARGUMENT;
    size_t cells = ((size_t)width + 2) * height;
    Accumulator acc = {sr_alloc(cells * sizeof(float)), (int)width + 2,
                       (int)height};
    if (!acc.cells) return SR_ERR_MEMORY;
    for (size_t c = 0; c < path->count; ++c)
        accumulate_polygon(&acc, path->items[c].points, path->items[c].count);
    resolve(&acc, rule, fill);
    if (stroke) {
        if (stroke_width > 0.0) {
            SrStatus stroked = stroke_coverage(path, stroke_width * 0.5,
                                               width, height, stroke);
            if (stroked != SR_OK) {
                free(acc.cells);
                return stroked;
            }
        } else {
            memset(stroke, 0, (size_t)width * height * sizeof(float));
        }
    }
    free(acc.cells);
    return SR_OK;
}

void sr_vector_compose(const float *fill, const float *stroke,
                       const float fill_px[4], const float stroke_px[4],
                       size_t pixels, float *px) {
    for (size_t i = 0; i < pixels; ++i) {
        float cf = fill[i], cs = stroke ? stroke[i] : 0.0f;
        float keep = 1.0f - stroke_px[3] * cs;
        float *out = &px[i * 4];
        for (int c = 0; c < 4; ++c)
            out[c] = stroke_px[c] * cs + fill_px[c] * cf * keep;
    }
}

SrStatus sr_vector_path_render(const char *text, const SrVectorStyle *style,
                               const float fill_px[4], const float stroke_px[4],
                               uint32_t width, uint32_t height, float *px,
                               size_t source_line, SrDiagnostics *diag) {
    size_t pixels = (size_t)width * height;
    bool stroked = style->stroke_width > 0.0;
    float *fill = sr_alloc(pixels * sizeof(float));
    float *stroke = stroked ? sr_alloc(pixels * sizeof(float)) : NULL;
    if (!fill || (stroked && !stroke)) {
        free(fill); free(stroke);
        return SR_ERR_MEMORY;
    }
    SrStatus status = sr_vector_path_coverage(text, style->fill_rule,
                                              style->stroke_width, width,
                                              height, fill, stroke);
    if (status == SR_ERR_ASSET)
        sr_diag_error(diag, source_line, "vector", "path",
                      "invalid path; supported commands are M/L/H/V/C/Q/Z");
    if (status == SR_OK)
        sr_vector_compose(fill, stroke, fill_px, stroke_px, pixels, px);
    free(fill);
    free(stroke);
    return status;
}
