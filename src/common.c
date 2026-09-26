#define _POSIX_C_SOURCE 200809L
#include "scene_render/common.h"

#include <ctype.h>
#include <errno.h>
#include <limits.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

void *sr_alloc(size_t size) {
    if (size == 0) {
        size = 1;
    }
    return calloc(1, size);
}

void *sr_realloc(void *ptr, size_t size) {
    if (size == 0) {
        size = 1;
    }
    return realloc(ptr, size);
}

char *sr_strdup(const char *text) {
    if (!text) {
        return NULL;
    }
    size_t length = strlen(text) + 1;
    char *copy = sr_alloc(length);
    if (copy) {
        memcpy(copy, text, length);
    }
    return copy;
}

static bool sr_number_tail_ok(const char *tail) {
    while (*tail && isspace((unsigned char)*tail)) {
        ++tail;
    }
    return *tail == '\0';
}

bool sr_parse_double(const char *text, double *value) {
    if (!text || !*text || !value) {
        return false;
    }
    errno = 0;
    char *tail = NULL;
    double parsed = strtod(text, &tail);
    if (errno || tail == text || !sr_number_tail_ok(tail) || !isfinite(parsed)) {
        return false;
    }
    *value = parsed;
    return true;
}

bool sr_parse_u32(const char *text, uint32_t *value) {
    uint64_t parsed = 0;
    if (!sr_parse_u64(text, &parsed) || parsed > UINT32_MAX || !value) {
        return false;
    }
    *value = (uint32_t)parsed;
    return true;
}

/* Unsigned decimal: digits only. Signs, whitespace anywhere and trailing
 * characters are rejected (strtoull alone would accept " 5", "+5" and wrap
 * "-1" to UINT64_MAX). */
bool sr_parse_u64(const char *text, uint64_t *value) {
    if (!text || !*text || !value) {
        return false;
    }
    for (const char *cursor = text; *cursor; ++cursor) {
        if (*cursor < '0' || *cursor > '9') {
            return false;
        }
    }
    errno = 0;
    char *tail = NULL;
    unsigned long long parsed = strtoull(text, &tail, 10);
    if (errno || tail == text || *tail != '\0') {
        return false;
    }
    *value = (uint64_t)parsed;
    return true;
}

bool sr_parse_bool(const char *text, bool *value) {
    if (!text || !value) {
        return false;
    }
    if (strcmp(text, "true") == 0 || strcmp(text, "1") == 0) {
        *value = true;
        return true;
    }
    if (strcmp(text, "false") == 0 || strcmp(text, "0") == 0) {
        *value = false;
        return true;
    }
    return false;
}

bool sr_id_valid(const char *text) {
    if (!text || !(isalpha((unsigned char)*text) || *text == '_')) return false;
    for (++text; *text; ++text)
        if (!(isalnum((unsigned char)*text) || *text == '_' || *text == '-' ||
              *text == '.')) return false;
    return true;
}

static bool sr_hex_byte(const char *text, double *value) {
    int high = isdigit((unsigned char)text[0]) ? text[0] - '0'
               : isxdigit((unsigned char)text[0])
                   ? tolower((unsigned char)text[0]) - 'a' + 10
                   : -1;
    int low = isdigit((unsigned char)text[1]) ? text[1] - '0'
              : isxdigit((unsigned char)text[1])
                  ? tolower((unsigned char)text[1]) - 'a' + 10
                  : -1;
    if (high < 0 || low < 0) {
        return false;
    }
    *value = (double)(high * 16 + low) / 255.0;
    return true;
}

SrStatus sr_parse_color_status(const char *text, SrColor *color) {
    if (!text || !color) {
        return SR_ERR_ARGUMENT;
    }
    size_t length = strlen(text);
    if (text[0] == '#' && (length == 7 || length == 9)) {
        SrColor parsed = {0.0, 0.0, 0.0, 1.0};
        if (!sr_hex_byte(text + 1, &parsed.r) ||
            !sr_hex_byte(text + 3, &parsed.g) ||
            !sr_hex_byte(text + 5, &parsed.b) ||
            (length == 9 && !sr_hex_byte(text + 7, &parsed.a))) {
            return SR_ERR_ARGUMENT;
        }
        *color = parsed;
        return SR_OK;
    }
    double values[4] = {0.0, 0.0, 0.0, 1.0};
    char *copy = sr_strdup(text);
    if (!copy) {
        return SR_ERR_MEMORY;
    }
    size_t count = 0;
    char *cursor = copy;
    while (cursor && count < 4) {
        char *next = strchr(cursor, ',');
        if (next) {
            *next++ = '\0';
        }
        if (!sr_parse_double(cursor, &values[count])) {
            free(copy);
            return SR_ERR_ARGUMENT;
        }
        ++count;
        cursor = next;
    }
    bool valid = (count == 3 || count == 4) && cursor == NULL;
    for (size_t i = 0; valid && i < 4; ++i) {
        valid = values[i] >= 0.0 && values[i] <= 1.0;
    }
    free(copy);
    if (!valid) {
        return SR_ERR_ARGUMENT;
    }
    *color = (SrColor){values[0], values[1], values[2], values[3]};
    return SR_OK;
}

bool sr_parse_color(const char *text, SrColor *color) {
    return sr_parse_color_status(text, color) == SR_OK;
}

char *sr_path_dirname(const char *path) {
    if (!path || !*path) {
        return sr_strdup(".");
    }
    const char *slash = strrchr(path, '/');
    if (!slash) {
        return sr_strdup(".");
    }
    if (slash == path) {
        return sr_strdup("/");
    }
    size_t length = (size_t)(slash - path);
    char *dir = sr_alloc(length + 1);
    if (dir) {
        memcpy(dir, path, length);
    }
    return dir;
}

char *sr_path_join(const char *base, const char *path) {
    if (!path) {
        return NULL;
    }
    if (path[0] == '/') {
        return sr_strdup(path);
    }
    if (!base || !*base || strcmp(base, ".") == 0) {
        return sr_strdup(path);
    }
    size_t a = strlen(base);
    size_t b = strlen(path);
    bool slash = base[a - 1] != '/';
    char *joined = sr_alloc(a + b + (slash ? 2 : 1));
    if (!joined) {
        return NULL;
    }
    memcpy(joined, base, a);
    if (slash) {
        joined[a++] = '/';
    }
    memcpy(joined + a, path, b + 1);
    return joined;
}

double sr_monotonic_seconds(void) {
    struct timespec value;
    if (clock_gettime(CLOCK_MONOTONIC, &value) != 0) {
        return 0.0;
    }
    return (double)value.tv_sec + (double)value.tv_nsec / 1000000000.0;
}

uint64_t sr_fnv1a64(uint64_t hash, const void *data, size_t size) {
    const uint8_t *bytes = data;
    for (size_t i = 0; i < size; ++i) {
        hash ^= bytes[i];
        hash *= UINT64_C(1099511628211);
    }
    return hash;
}

SrMat3 sr_mat_identity(void) {
    return (SrMat3){1.0, 0.0, 0.0, 0.0, 1.0, 0.0};
}

SrMat3 sr_mat_multiply(SrMat3 a, SrMat3 b) {
    return (SrMat3){
        a.m00 * b.m00 + a.m01 * b.m10,
        a.m00 * b.m01 + a.m01 * b.m11,
        a.m00 * b.m02 + a.m01 * b.m12 + a.m02,
        a.m10 * b.m00 + a.m11 * b.m10,
        a.m10 * b.m01 + a.m11 * b.m11,
        a.m10 * b.m02 + a.m11 * b.m12 + a.m12};
}

SrMat3 sr_mat_translate(double x, double y) {
    return (SrMat3){1.0, 0.0, x, 0.0, 1.0, y};
}

SrMat3 sr_mat_scale(double x, double y) {
    return (SrMat3){x, 0.0, 0.0, 0.0, y, 0.0};
}

SrMat3 sr_mat_rotate(double radians) {
    double c = cos(radians);
    double s = sin(radians);
    return (SrMat3){c, -s, 0.0, s, c, 0.0};
}

SrMat3 sr_mat_apply_skew(SrMat3 matrix, double x, double y) {
    if (x != 0.0)
        matrix = sr_mat_multiply(matrix,
            (SrMat3){1, tan(x * SR_PI / 180.0), 0, 0, 1, 0});
    if (y != 0.0)
        matrix = sr_mat_multiply(matrix,
            (SrMat3){1, 0, 0, tan(y * SR_PI / 180.0), 1, 0});
    return matrix;
}

bool sr_mat_finite(SrMat3 matrix) {
    return isfinite(matrix.m00) && isfinite(matrix.m01) &&
           isfinite(matrix.m02) && isfinite(matrix.m10) &&
           isfinite(matrix.m11) && isfinite(matrix.m12);
}

bool sr_mat_inverse(SrMat3 m, SrMat3 *inverse) {
    double det = m.m00 * m.m11 - m.m01 * m.m10;
    if (!inverse || fabs(det) < 1e-15) {
        return false;
    }
    double inv = 1.0 / det;
    *inverse = (SrMat3){m.m11 * inv, -m.m01 * inv,
                        (m.m01 * m.m12 - m.m11 * m.m02) * inv,
                        -m.m10 * inv, m.m00 * inv,
                        (m.m10 * m.m02 - m.m00 * m.m12) * inv};
    return true;
}

bool sr_mat_checked_inverse(SrMat3 matrix, SrMat3 *inverse) {
    return sr_mat_finite(matrix) &&
           isfinite(matrix.m00 * matrix.m11 - matrix.m01 * matrix.m10) &&
           sr_mat_inverse(matrix, inverse) && sr_mat_finite(*inverse);
}

SrVec2 sr_mat_point(SrMat3 m, SrVec2 p) {
    return (SrVec2){m.m00 * p.x + m.m01 * p.y + m.m02,
                    m.m10 * p.x + m.m11 * p.y + m.m12};
}
