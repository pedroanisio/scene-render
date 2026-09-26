/* SPDX-License-Identifier: Apache-2.0 */
#ifndef SR_COMPOSITOR_MATTE_INTERNAL_H
#define SR_COMPOSITOR_MATTE_INTERNAL_H

#include "compositor_internal.h"
#include "compositor_resources_internal.h"

/* One track-matte source image of the current frame, reduced to the two
 * coverage channels consumers sample: alpha and alpha * Y(straight color)
 * (sr_luma_px). Composition-sized; `dirty` bounds the nonzero samples.
 * Immutable once published, until the frame's final queue flush. */
typedef struct SrMatteCapture {
    float *coverage;            /* 2 floats per pixel, or NULL when empty */
    uint32_t width, height;
    SrClip dirty;
    bool rendered;              /* false: source not needed at this time */
} SrMatteCapture;

typedef struct SrMatteFrame {
    SrMatteCapture *captures;   /* indexed by the plan's source index */
    size_t count;
} SrMatteFrame;

SrStatus sr_matte_frame_create(SrCompositeResources *resources, size_t count,
                               SrMatteFrame **out);
void sr_matte_frame_free(SrCompositeResources *resources, SrMatteFrame *frame);

/* Reduces a rendered premultiplied RGBA capture (rows of `width` pixels,
 * nonzero only inside `dirty`) into `capture`. */
SrStatus sr_matte_capture_store(SrCompositeResources *resources,
                                const SrLumaConfig *luma, const float *rgba,
                                uint32_t width, uint32_t height, SrClip dirty,
                                SrMatteCapture *capture);

/* Matte coverage in [0,1] at receiving pixel centre (cx, cy): the capture
 * is sampled bilinearly at the composition point of that centre (through
 * link->plane inside a projective card), with zero coverage outside it,
 * then selected/inverted by link->matte_mode. */
float sr_matte_sample(const SrMaskLink *link, double cx, double cy);

/* Coverage everywhere when the source image is empty. */
float sr_matte_empty_value(SrMatteMode mode);

#endif
