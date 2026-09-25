#include "scene_render/color.h"
#include "scene_render/parallel.h"

#include <math.h>
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

typedef struct {
    SrFrame *frame;
    SrColorSpace source;
    SrColorSpace target;
    Matrix3 matrix;
} ConvertContext;

static void convert_worker(void *opaque, size_t begin, size_t end) {
    ConvertContext *context = opaque;
    for (size_t pixel=begin; pixel<end; ++pixel) {
        double input[3], output[3] = {0};
        for (size_t channel=0; channel<3; ++channel)
            input[channel] = sr_color_decode(
                context->frame->rgba[pixel*4+channel]/255.0, context->source);
        for (size_t row=0; row<3; ++row) for (size_t column=0; column<3; ++column)
            output[row] += context->matrix.value[row][column] * input[column];
        for (size_t channel=0; channel<3; ++channel)
            context->frame->rgba[pixel*4+channel] = (uint8_t)lrint(
                sr_color_encode(output[channel], context->target)*255.0);
    }
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

SrStatus sr_color_convert_frame(SrFrame *frame, SrColorSpace source,
                                SrColorSpace target, unsigned threads) {
    if (!frame || !frame->rgba) return SR_ERR_ARGUMENT;
    if (source == target) return SR_OK;
    ConvertContext context = {
        frame, source, target,
        multiply(inverse(rgb_to_xyz(target)), rgb_to_xyz(source))
    };
    return sr_parallel_for((size_t)frame->width*frame->height, threads,
                           convert_worker, &context);
}
