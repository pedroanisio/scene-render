/* Re-apply the engine's Spherical Video V1 metadata to an MP4 that was
 * spliced with FFmpeg stream copy (the remux drops the engine's uuid box).
 * Built by render.py against the engine objects in build/. */
#include "scene_render/diagnostics.h"
#include "scene_render/spatial.h"

#include <stdio.h>
#include <stdlib.h>

int main(int argc, char **argv) {
    if (argc != 4) {
        fprintf(stderr, "usage: %s FILE.mp4 WIDTH HEIGHT\n", argv[0]);
        return 2;
    }
    SrDiagnostics diag;
    sr_diag_init(&diag, argv[1], stderr);
    uint32_t width = (uint32_t)strtoul(argv[2], NULL, 10);
    uint32_t height = (uint32_t)strtoul(argv[3], NULL, 10);
    return sr_spatial_inject_mp4(argv[1], width, height, &diag) == SR_OK ? 0 : 1;
}
