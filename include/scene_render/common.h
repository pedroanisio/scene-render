#ifndef SCENE_RENDER_COMMON_H
#define SCENE_RENDER_COMMON_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define SR_VERSION "1.1.0"
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
/* Leaves color unchanged on failure; decimal parsing may return SR_ERR_MEMORY. */
SrStatus sr_parse_color_status(const char *text, SrColor *color);
bool sr_parse_color(const char *text, SrColor *color);
bool sr_id_valid(const char *text);
char *sr_path_dirname(const char *path);
char *sr_path_join(const char *base, const char *path);
double sr_monotonic_seconds(void);

/* 64-bit FNV-1a: start from SR_FNV_OFFSET and feed bytes in order. */
#define SR_FNV_OFFSET UINT64_C(14695981039346656037)
uint64_t sr_fnv1a64(uint64_t hash, const void *data, size_t size);

SrMat3 sr_mat_identity(void);
SrMat3 sr_mat_multiply(SrMat3 a, SrMat3 b);
SrMat3 sr_mat_translate(double x, double y);
SrMat3 sr_mat_scale(double x, double y);
SrMat3 sr_mat_rotate(double radians);
/* Append Kx then Ky; angles are degrees, validated by the caller. Zero
 * angles perform no multiplication, preserving the legacy transform. */
SrMat3 sr_mat_apply_skew(SrMat3 matrix, double x, double y);
bool sr_mat_finite(SrMat3 matrix);
bool sr_mat_inverse(SrMat3 matrix, SrMat3 *inverse);
/* Strict inverse for new transforms; leaves legacy inverse policy intact. */
bool sr_mat_checked_inverse(SrMat3 matrix, SrMat3 *inverse);
SrVec2 sr_mat_point(SrMat3 matrix, SrVec2 point);

#endif
