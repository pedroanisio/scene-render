#ifndef SCENE_RENDER_COMMON_H
#define SCENE_RENDER_COMMON_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define SR_VERSION "1.0.0"
#define SR_PI 3.14159265358979323846264338327950288

typedef enum {
    SR_OK = 0,
    SR_ERR_ARGUMENT = 2,
    SR_ERR_XML = 3,
    SR_ERR_ASSET = 4,
    SR_ERR_RENDER = 5,
    SR_ERR_ENCODER = 6,
    SR_ERR_IO = 7,
    SR_ERR_MEMORY = 8
} SrStatus;

typedef struct {
    double x;
    double y;
} SrVec2;

typedef struct {
    double r;
    double g;
    double b;
    double a;
} SrColor;

typedef struct {
    double m00, m01, m02;
    double m10, m11, m12;
} SrMat3;

void *sr_alloc(size_t size);
void *sr_realloc(void *ptr, size_t size);
char *sr_strdup(const char *text);
bool sr_parse_double(const char *text, double *value);
bool sr_parse_u32(const char *text, uint32_t *value);
bool sr_parse_u64(const char *text, uint64_t *value);
bool sr_parse_bool(const char *text, bool *value);
bool sr_parse_color(const char *text, SrColor *color);
bool sr_id_valid(const char *text);
char *sr_path_dirname(const char *path);
char *sr_path_join(const char *base, const char *path);
double sr_monotonic_seconds(void);

SrMat3 sr_mat_identity(void);
SrMat3 sr_mat_multiply(SrMat3 a, SrMat3 b);
SrMat3 sr_mat_translate(double x, double y);
SrMat3 sr_mat_scale(double x, double y);
SrMat3 sr_mat_rotate(double radians);
bool sr_mat_inverse(SrMat3 matrix, SrMat3 *inverse);
SrVec2 sr_mat_point(SrMat3 matrix, SrVec2 point);

#endif
