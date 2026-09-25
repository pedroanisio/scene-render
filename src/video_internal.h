/* Internal to the core (and its unit tests); not installed. */
#ifndef SR_VIDEO_INTERNAL_H
#define SR_VIDEO_INTERNAL_H

#include <stdbool.h>

/* swscale matrix (SWS_CS_*) for a libav AVColorSpace tag and frame height.
 * *approximated (may be NULL) is set when swscale has no exact matrix and
 * the closest one is used (BT.2020 constant luminance). */
int sr_video_sws_matrix(int colorspace, int height, bool *approximated);

#endif
