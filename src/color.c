#include "scene_render/color.h"
#include "scene_render/parallel.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>

typedef struct { double value[3][3]; } Matrix3;

static Matrix3 rgb_to_xyz(SrColorSpace space) {
    if (space == SR_COLOR_DISPLAY_P3) return (Matrix3){{
        {0.48657095,0.26566769,0.19821729},
        {0.22897456,0.69173852,0.07928691},
        {0.00000000,0.04511338,1.04394437}}};
    if (space == SR_COLOR_REC2020) return (Matrix3){{
        {0.63695805,0.14461690,0.16888098},
        {0.26270021,0.67799807,0.05930172},
        {0.00000000,0.02807269,1.06098506}}};
    return (Matrix3){{
        {0.41245640,0.35757610,0.18043750},
        {0.21267290,0.71515220,0.07217500},
        {0.01933390,0.11919200,0.95030410}}};
}

static Matrix3 multiply(Matrix3 left, Matrix3 right) {
    Matrix3 result = {0};
    for (size_t row=0; row<3; ++row) for (size_t column=0; column<3; ++column)
        for (size_t k=0; k<3; ++k)
            result.value[row][column] += left.value[row][k] *
                                         right.value[k][column];
    return result;
}

static Matrix3 inverse(Matrix3 matrix) {
    double (*m)[3] = matrix.value;
    double determinant = m[0][0]*(m[1][1]*m[2][2]-m[1][2]*m[2][1]) -
        m[0][1]*(m[1][0]*m[2][2]-m[1][2]*m[2][0]) +
        m[0][2]*(m[1][0]*m[2][1]-m[1][1]*m[2][0]);
    Matrix3 result = {{
        {(m[1][1]*m[2][2]-m[1][2]*m[2][1])/determinant,
         (m[0][2]*m[2][1]-m[0][1]*m[2][2])/determinant,
         (m[0][1]*m[1][2]-m[0][2]*m[1][1])/determinant},
        {(m[1][2]*m[2][0]-m[1][0]*m[2][2])/determinant,
         (m[0][0]*m[2][2]-m[0][2]*m[2][0])/determinant,
         (m[0][2]*m[1][0]-m[0][0]*m[1][2])/determinant},
        {(m[1][0]*m[2][1]-m[1][1]*m[2][0])/determinant,
         (m[0][1]*m[2][0]-m[0][0]*m[2][1])/determinant,
         (m[0][0]*m[1][1]-m[0][1]*m[1][0])/determinant}}};
    return result;
}

double sr_color_decode(double value, SrColorSpace space) {
    value = fmax(0.0, fmin(1.0, value));
    if (space == SR_COLOR_REC2020 || space == SR_COLOR_REC709) {
        const double alpha = space == SR_COLOR_REC2020
            ? 1.09929682680944 : 1.099;
        const double beta = space == SR_COLOR_REC2020
            ? 0.018053968510807 : 0.018;
        return value < 4.5*beta ? value/4.5 :
            pow((value+alpha-1.0)/alpha, 1.0/0.45);
    }
    return value <= .04045 ? value/12.92 : pow((value+.055)/1.055, 2.4);
}

double sr_color_encode(double value, SrColorSpace space) {
    value = fmax(0.0, fmin(1.0, value));
    if (space == SR_COLOR_REC2020 || space == SR_COLOR_REC709) {
        const double alpha = space == SR_COLOR_REC2020
            ? 1.09929682680944 : 1.099;
        const double beta = space == SR_COLOR_REC2020
            ? 0.018053968510807 : 0.018;
        return value < beta ? value*4.5 : alpha*pow(value,.45)-(alpha-1.0);
    }
    return value <= .0031308 ? value*12.92 : 1.055*pow(value,1.0/2.4)-.055;
}

bool sr_color_space_parse(const char *text, SrColorSpace *space) {
    if (!text || !space) return false;
    if (!strcmp(text,"srgb")) *space=SR_COLOR_SRGB;
    else if (!strcmp(text,"rec709")) *space=SR_COLOR_REC709;
    else if (!strcmp(text,"display-p3")) *space=SR_COLOR_DISPLAY_P3;
    else if (!strcmp(text,"rec2020")) *space=SR_COLOR_REC2020;
    else return false;
    return true;
}

const char *sr_color_space_name(SrColorSpace space) {
    if (space == SR_COLOR_DISPLAY_P3) return "display-p3";
    if (space == SR_COLOR_REC2020) return "rec2020";
    if (space == SR_COLOR_REC709) return "rec709";
    return "srgb";
}

static Matrix3 gamut_matrix(SrColorSpace source, SrColorSpace target) {
    return multiply(inverse(rgb_to_xyz(target)), rgb_to_xyz(source));
}

static bool same_gamut(SrColorSpace a, SrColorSpace b) {
    /* Rec.709 and sRGB share primaries and white point. */
    bool a_srgb = a == SR_COLOR_SRGB || a == SR_COLOR_REC709;
    bool b_srgb = b == SR_COLOR_SRGB || b == SR_COLOR_REC709;
    return a == b || (a_srgb && b_srgb);
}

static double clamp_unit(double value) {
    return value > 0.0 ? (value < 1.0 ? value : 1.0) : 0.0;
}

void sr_color_to_blend(const SrProject *project, SrColor color, float out[4]) {
    double alpha = clamp_unit(color.a);
    double rgb[3] = {color.r, color.g, color.b};
    for (size_t c = 0; c < 3; ++c) {
        double value = clamp_unit(rgb[c]);
        if (project->linear_light)
            value = sr_color_decode(value, project->working_color_space);
        out[c] = (float)(value * alpha);
    }
    out[3] = (float)alpha;
}

SrStatus sr_color_image_from_rgba8(const SrProject *project,
                                   SrColorSpace source,
                                   const uint8_t *rgba8, size_t stride,
                                   uint32_t width, uint32_t height,
                                   SrImage *out) {
    if (!project || !rgba8 || !out || !width || !height) return SR_ERR_ARGUMENT;
    *out = (SrImage){0};
    size_t pixels = (size_t)width * height;
    if (pixels / height != width || pixels > SIZE_MAX / (4 * sizeof(float)) ||
        stride < (size_t)width * 4)
        return SR_ERR_MEMORY;
    float *px = sr_alloc(pixels * 4 * sizeof(float));
    if (!px) return SR_ERR_MEMORY;
    SrColorSpace working = project->working_color_space;
    bool linear = project->linear_light;
    bool direct = source == working;
    Matrix3 matrix = gamut_matrix(source, working);
    /* Transfer decode table for the source's 8-bit codes. For a same-space
     * source it already yields blend-space values. */
    float table[256];
    double linear_table[256];
    for (int i = 0; i < 256; ++i) {
        linear_table[i] = sr_color_decode(i / 255.0, source);
        table[i] = direct && !linear ? (float)(i / 255.0)
                                     : (float)linear_table[i];
    }
    for (uint32_t y = 0; y < height; ++y) {
        const uint8_t *row = rgba8 + (size_t)y * stride;
        float *target = px + (size_t)y * width * 4;
        for (uint32_t x = 0; x < width; ++x) {
            const uint8_t *s = row + (size_t)x * 4;
            float *d = target + (size_t)x * 4;
            float alpha = (float)(s[3] / 255.0);
            if (direct) {
                for (size_t c = 0; c < 3; ++c) d[c] = table[s[c]] * alpha;
            } else {
                double in[3] = {linear_table[s[0]], linear_table[s[1]],
                                linear_table[s[2]]};
                for (size_t row_index = 0; row_index < 3; ++row_index) {
                    double value = 0.0;
                    for (size_t k = 0; k < 3; ++k)
                        value += matrix.value[row_index][k] * in[k];
                    value = clamp_unit(value);
                    if (!linear) value = sr_color_encode(value, working);
                    d[row_index] = (float)value * alpha;
                }
            }
            d[3] = alpha;
        }
    }
    out->width = width;
    out->height = height;
    out->px = px;
    return SR_OK;
}

static uint8_t round_code(double value) {
    double scaled = floor(clamp_unit(value) * 255.0 + 0.5);
    return (uint8_t)(scaled > 255.0 ? 255.0 : scaled);
}

SrStatus sr_color_output_init(SrColorOutput *output, const SrProject *project,
                              SrColorSpace target) {
    if (!output || !project) return SR_ERR_ARGUMENT;
    *output = (SrColorOutput){0};
    SrColorSpace working = project->working_color_space;
    output->linear_light = project->linear_light;
    output->passthrough = !project->linear_light && working == target;
    output->identity_gamut = same_gamut(working, target);
    Matrix3 matrix = gamut_matrix(working, target);
    memcpy(output->matrix, matrix.value, sizeof(output->matrix));
    if (output->passthrough) return SR_OK;
    output->encode = sr_alloc(SR_COLOR_LUT_SIZE);
    if (!output->encode) return SR_ERR_MEMORY;
    for (size_t i = 0; i < SR_COLOR_LUT_SIZE; ++i)
        output->encode[i] = round_code(sr_color_encode(
            (double)i / (SR_COLOR_LUT_SIZE - 1), target));
    if (!project->linear_light) {
        output->decode = sr_alloc(SR_COLOR_LUT_SIZE * sizeof(float));
        if (!output->decode) {
            sr_color_output_free(output);
            return SR_ERR_MEMORY;
        }
        for (size_t i = 0; i < SR_COLOR_LUT_SIZE; ++i)
            output->decode[i] = (float)sr_color_decode(
                (double)i / (SR_COLOR_LUT_SIZE - 1), working);
    }
    return SR_OK;
}

void sr_color_output_free(SrColorOutput *output) {
    if (!output) return;
    free(output->decode);
    free(output->encode);
    *output = (SrColorOutput){0};
}

static inline int lut_index(float value) {
    if (!(value > 0.0f)) return 0;
    if (value >= 1.0f) return SR_COLOR_LUT_SIZE - 1;
    return (int)(value * (float)(SR_COLOR_LUT_SIZE - 1) + 0.5f);
}

static inline uint8_t direct_code(float value) {
    if (!(value > 0.0f)) return 0;
    if (value >= 1.0f) return 255;
    return (uint8_t)(value * 255.0f + 0.5f);
}

typedef struct {
    const SrColorOutput *output;
    const SrFrame *frame;
    uint8_t *rgba8;
} ConvertContext;

static void convert_rows(void *opaque, size_t begin, size_t end) {
    const ConvertContext *context = opaque;
    const SrColorOutput *output = context->output;
    uint32_t width = context->frame->width;
    for (size_t y = begin; y < end; ++y) {
        const float *s = context->frame->px + y * width * 4;
        uint8_t *d = context->rgba8 + y * width * 4;
        for (uint32_t x = 0; x < width; ++x, s += 4, d += 4) {
            float alpha = s[3];
            d[3] = direct_code(alpha);
            if (!(alpha > 0.0f)) {
                d[0] = d[1] = d[2] = 0;
                continue;
            }
            float inv = 1.0f / alpha;
            float c[3] = {s[0] * inv, s[1] * inv, s[2] * inv};
            if (output->passthrough) {
                for (size_t i = 0; i < 3; ++i) d[i] = direct_code(c[i]);
                continue;
            }
            if (!output->linear_light)
                for (size_t i = 0; i < 3; ++i)
                    c[i] = output->decode[lut_index(c[i])];
            if (!output->identity_gamut) {
                float m[3];
                for (size_t row = 0; row < 3; ++row)
                    m[row] = (float)(output->matrix[row][0] * c[0] +
                                     output->matrix[row][1] * c[1] +
                                     output->matrix[row][2] * c[2]);
                memcpy(c, m, sizeof(c));
            }
            for (size_t i = 0; i < 3; ++i)
                d[i] = output->encode[lut_index(c[i])];
        }
    }
}

SrStatus sr_color_convert_frame(const SrColorOutput *output,
                                const SrFrame *frame, uint8_t *rgba8,
                                unsigned threads) {
    if (!output || !frame || !frame->px || !rgba8) return SR_ERR_ARGUMENT;
    if (!output->passthrough && !output->encode) return SR_ERR_ARGUMENT;
    ConvertContext context = {output, frame, rgba8};
    return sr_parallel_for(frame->height, threads, convert_rows, &context);
}

/* ---- animated colors ----------------------------------------------------- */

SrAnimColor sr_anim_color_static(SrColor base) {
    return (SrAnimColor){.base = base, .space = SR_COLOR_SRGB};
}

void sr_anim_color_free(SrAnimColor *color) {
    if (!color) return;
    sr_track_free(&color->r);
    sr_track_free(&color->g);
    sr_track_free(&color->b);
    sr_track_free(&color->a);
}

SrStatus sr_anim_color_add_key(SrAnimColor *color, SrKeyframe key, SrColor value) {
    SrKeyframe channel = key;
    channel.value = sr_color_decode(value.r, color->space);
    SrStatus status = sr_track_add(&color->r, channel);
    channel.value = sr_color_decode(value.g, color->space);
    if (status == SR_OK) status = sr_track_add(&color->g, channel);
    channel.value = sr_color_decode(value.b, color->space);
    if (status == SR_OK) status = sr_track_add(&color->b, channel);
    channel.value = fmax(0.0, fmin(1.0, value.a));
    if (status == SR_OK) status = sr_track_add(&color->a, channel);
    return status;
}

SrStatus sr_anim_color_finalize(SrAnimColor *color) {
    SrStatus status = sr_track_finalize(&color->r);
    if (status == SR_OK) status = sr_track_finalize(&color->g);
    if (status == SR_OK) status = sr_track_finalize(&color->b);
    if (status == SR_OK) status = sr_track_finalize(&color->a);
    return status;
}

SrColor sr_anim_color_eval(const SrAnimColor *color, double time) {
    if (!color) return (SrColor){0, 0, 0, 0};
    if (color->r.count == 0) return color->base;
    return (SrColor){
        sr_color_encode(sr_track_eval(&color->r, 0.0, time), color->space),
        sr_color_encode(sr_track_eval(&color->g, 0.0, time), color->space),
        sr_color_encode(sr_track_eval(&color->b, 0.0, time), color->space),
        fmax(0.0, fmin(1.0, sr_track_eval(&color->a, 0.0, time)))};
}

static double mix_channel(double a, double b, double t, SrColorSpace space) {
    if (a == b || t == 0.0) return a;
    double la = sr_color_decode(a, space), lb = sr_color_decode(b, space);
    return sr_color_encode(la + (lb - la) * t, space);
}

SrColor sr_color_mix_linear(SrColor a, SrColor b, double t, SrColorSpace space) {
    return (SrColor){mix_channel(a.r, b.r, t, space),
                     mix_channel(a.g, b.g, t, space),
                     mix_channel(a.b, b.b, t, space),
                     a.a == b.a || t == 0.0 ? a.a : a.a + (b.a - a.a) * t};
}
