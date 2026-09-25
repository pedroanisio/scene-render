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

static bool parse_path(const char *text,Path *path) {
    const char *cursor=text;char command=0;Contour *contour=NULL;Point current={0},first={0};
    while(1){separators(&cursor);if(!*cursor)break;
        if(isalpha((unsigned char)*cursor))command=*cursor++;
        else if(!command)return false;
        bool relative=islower((unsigned char)command)!=0;char op=(char)toupper((unsigned char)command);
        if(op=='Z'){if(contour&&contour->count&&
            (current.x!=first.x||current.y!=first.y)&&!add_point(contour,first))return false;
            if(contour)contour->closed=true;
            current=first;command=0;continue;}
        if(op=='M'||op=='L'){double x,y;if(!pair(&cursor,&x,&y))return false;
            if(relative){x+=current.x;y+=current.y;}current=(Point){x,y};
            if(op=='M'){contour=add_contour(path);if(!contour)return false;first=current;
                command=relative?'l':'L';}
            if(!contour||!add_point(contour,current))return false;
            continue;}
        if(!contour||!contour->count)return false;
        if(op=='H'){double x;if(!number(&cursor,&x))return false;if(relative)x+=current.x;
            current.x=x;if(!add_point(contour,current))return false;continue;}
        if(op=='V'){double y;if(!number(&cursor,&y))return false;if(relative)y+=current.y;
            current.y=y;if(!add_point(contour,current))return false;continue;}
        if(op=='C'){double ax,ay,bx,by,x,y;if(!pair(&cursor,&ax,&ay)||!pair(&cursor,&bx,&by)||!pair(&cursor,&x,&y))return false;
            if(relative){ax+=current.x;ay+=current.y;bx+=current.x;by+=current.y;x+=current.x;y+=current.y;}
            Point end={x,y};if(!flatten_cubic(contour,current,(Point){ax,ay},(Point){bx,by},end))return false;current=end;continue;}
        if(op=='Q'){double cx,cy,x,y;if(!pair(&cursor,&cx,&cy)||!pair(&cursor,&x,&y))return false;
            if(relative){cx+=current.x;cy+=current.y;x+=current.x;y+=current.y;}
            Point end={x,y};if(!flatten_quadratic(contour,current,(Point){cx,cy},end))return false;current=end;continue;}
        return false;
    }
    if(!path->count)return false;
    for(size_t i=0;i<path->count;++i)if(path->items[i].count<2)return false;
    return true;
}

bool sr_vector_path_valid(const char *text) {
    Path path={0};
    bool valid=text&&parse_path(text,&path);
    path_free(&path);
    return valid;
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
    int ystart = (int)floor(y0 < 0.0 ? 0.0 : y0);
    if (y0 < 0.0) x -= y0 * dxdy;
    int yend = (int)ceil(y1) < acc->height ? (int)ceil(y1) : acc->height;
    for (int y = ystart; y < yend; ++y) {
        float *row = acc->cells + (size_t)y * (size_t)acc->width;
        double dy = fmin(y + 1.0, y1) - fmax((double)y, y0);
        double xnext = x + dxdy * dy;
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

enum { JOIN_STEPS = 16 };

/* Stroke as the union of one quad per segment and a disc per vertex (round
 * joins and caps), all wound the same way so overlaps add and clamp at full
 * coverage under the nonzero rule. */
static void accumulate_stroke(Accumulator *acc, const Path *path,
                              double half_width) {
    Point disc[JOIN_STEPS];
    for (size_t c = 0; c < path->count; ++c) {
        const Contour *contour = &path->items[c];
        size_t n = contour->count;
        for (size_t i = 0; i < n; ++i) {
            Point center = contour->points[i];
            for (int j = 0; j < JOIN_STEPS; ++j) {
                double angle = 2.0 * SR_PI * j / JOIN_STEPS;
                disc[j] = (Point){center.x + half_width * cos(angle),
                                  center.y + half_width * sin(angle)};
            }
            accumulate_polygon(acc, disc, JOIN_STEPS);
        }
        size_t segments = contour->closed ? n : n - 1;
        for (size_t i = 0; i < segments; ++i) {
            Point a = contour->points[i], b = contour->points[(i + 1) % n];
            double length = hypot(b.x - a.x, b.y - a.y);
            if (length < 1e-12) continue;
            double nx = -(b.y - a.y) / length * half_width;
            double ny = (b.x - a.x) / length * half_width;
            /* Wound like the discs (clockwise on screen). */
            Point quad[4] = {{a.x - nx, a.y - ny}, {b.x - nx, b.y - ny},
                             {b.x + nx, b.y + ny}, {a.x + nx, a.y + ny}};
            accumulate_polygon(acc, quad, 4);
        }
    }
}

SrStatus sr_vector_path_coverage(const char *text, SrFillRule rule,
                                 double stroke_width, uint32_t width,
                                 uint32_t height, float *fill, float *stroke) {
    if (!text || !fill || !width || !height || width > INT32_MAX - 2 ||
        height > INT32_MAX)
        return SR_ERR_ARGUMENT;
    Path path = {0};
    if (!parse_path(text, &path)) {
        path_free(&path);
        return SR_ERR_ASSET;
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
            memset(acc.cells, 0, cells * sizeof(float));
            accumulate_stroke(&acc, &path, stroke_width * 0.5);
            resolve(&acc, SR_FILL_NONZERO, stroke);
        } else {
            memset(stroke, 0, (size_t)width * height * sizeof(float));
        }
    }
    free(acc.cells);
    path_free(&path);
    return SR_OK;
}

static uint8_t channel(double value) {
    return (uint8_t)floor(fmax(0.0, fmin(1.0, value)) * 255.0 + 0.5);
}

void sr_vector_compose(const float *fill, const float *stroke,
                       const SrVectorStyle *style, size_t pixels,
                       uint8_t *rgba8) {
    const SrColor f = style->fill, s = style->stroke;
    for (size_t i = 0; i < pixels; ++i) {
        double cf = fill[i] * f.a;
        double cs = stroke ? stroke[i] * s.a : 0.0;
        double keep = 1.0 - cs;
        double alpha = cs + cf * keep;
        uint8_t *out = &rgba8[i * 4];
        if (!(alpha > 0.0)) {
            out[0] = out[1] = out[2] = out[3] = 0;
            continue;
        }
        out[0] = channel((s.r * cs + f.r * cf * keep) / alpha);
        out[1] = channel((s.g * cs + f.g * cf * keep) / alpha);
        out[2] = channel((s.b * cs + f.b * cf * keep) / alpha);
        out[3] = channel(alpha);
    }
}

SrStatus sr_vector_path_render(const char *text, const SrVectorStyle *style,
                               uint32_t width, uint32_t height, uint8_t *rgba8,
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
    if (status == SR_OK) sr_vector_compose(fill, stroke, style, pixels, rgba8);
    free(fill);
    free(stroke);
    return status;
}
