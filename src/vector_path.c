#include "scene_render/vector_path.h"

#include <ctype.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>

typedef struct { double x, y; } Point;
typedef struct { Point *points; size_t count, capacity; bool closed; } Contour;
typedef struct { Contour *items; size_t count, capacity; } Path;

static void path_free(Path *path) {
    for (size_t i=0;i<path->count;++i) free(path->items[i].points);
    free(path->items);
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

SrStatus sr_vector_path_check(const char *text) {
    if (!text) return SR_ERR_ASSET;
    Path path={0};
    SrStatus status=parse_path(text,&path);
    path_free(&path);
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

static void stroke_coverage(const Path *path, double half_width,
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
            /* Overlap of a 1 px box across the stroke, [d-.5, d+.5], with the
             * band [-half_width, half_width]: exact for straight runs, also
             * for strokes thinner than a pixel. */
            double coverage = fmin(best + 0.5, half_width) -
                              fmax(best - 0.5, -half_width);
            stroke[(size_t)y * width + x] =
                (float)fmin(fmax(coverage, 0.0), 1.0);
        }
    }
}

SrStatus sr_vector_path_coverage(const char *text, SrFillRule rule,
                                 double stroke_width, uint32_t width,
                                 uint32_t height, float *fill, float *stroke) {
    if (!text || !fill || !width || !height || width > INT32_MAX - 2 ||
        height > INT32_MAX ||
        (size_t)height > SIZE_MAX / sizeof(float) / ((size_t)width + 2))
        return SR_ERR_ARGUMENT;
    Path path = {0};
    SrStatus parsed = parse_path(text, &path);
    if (parsed != SR_OK) {
        path_free(&path);
        return parsed;
    }
    size_t cells = ((size_t)width + 2) * height;
    Accumulator acc = {sr_alloc(cells * sizeof(float)), (int)width + 2,
                       (int)height};
    if (!acc.cells) {
        path_free(&path);
        return SR_ERR_MEMORY;
    }
    for (size_t c = 0; c < path.count; ++c)
        accumulate_polygon(&acc, path.items[c].points, path.items[c].count);
    resolve(&acc, rule, fill);
    if (stroke) {
        if (stroke_width > 0.0) {
            stroke_coverage(&path, stroke_width * 0.5, width, height, stroke);
        } else {
            memset(stroke, 0, (size_t)width * height * sizeof(float));
        }
    }
    free(acc.cells);
    path_free(&path);
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
