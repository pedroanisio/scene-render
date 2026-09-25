#define _POSIX_C_SOURCE 200809L
#include "scene_render/spatial.h"

#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <unistd.h>

typedef struct {
    uint64_t offset;
    uint64_t size;
    uint64_t header;
    bool extended;
    char type[5];
} Mp4Box;

static uint32_t read_be32(const uint8_t bytes[4]) {
    return ((uint32_t)bytes[0] << 24) | ((uint32_t)bytes[1] << 16) |
           ((uint32_t)bytes[2] << 8) | bytes[3];
}

static uint64_t read_be64(const uint8_t bytes[8]) {
    uint64_t value = 0;
    for (size_t i = 0; i < 8; ++i) value = (value << 8) | bytes[i];
    return value;
}

static bool seek_to(FILE *file, uint64_t offset) {
    return offset <= INT64_MAX && fseeko(file, (off_t)offset, SEEK_SET) == 0;
}

static bool read_box(FILE *file, uint64_t offset, uint64_t limit, Mp4Box *box) {
    uint8_t header[16];
    if (limit - offset < 8 || !seek_to(file, offset) ||
        fread(header, 1, 8, file) != 8) return false;
    uint32_t compact = read_be32(header);
    *box = (Mp4Box){.offset = offset, .header = 8};
    memcpy(box->type, header + 4, 4);
    if (compact == 1) {
        if (limit - offset < 16 || fread(header + 8, 1, 8, file) != 8)
            return false;
        box->size = read_be64(header + 8);
        box->header = 16;
        box->extended = true;
    } else {
        box->size = compact ? compact : limit - offset;
    }
    return box->size >= box->header && box->size <= limit - offset;
}

static bool find_child(FILE *file, const Mp4Box *parent, const char type[4],
                       Mp4Box *result) {
    uint64_t cursor = parent->offset + parent->header;
    uint64_t end = parent->offset + parent->size;
    while (cursor < end) {
        Mp4Box box;
        if (!read_box(file, cursor, end, &box)) return false;
        if (memcmp(box.type, type, 4) == 0) {
            *result = box;
            return true;
        }
        cursor += box.size;
    }
    return false;
}

static bool is_video_track(FILE *file, const Mp4Box *track) {
    Mp4Box media, handler;
    if (!find_child(file, track, "mdia", &media) ||
        !find_child(file, &media, "hdlr", &handler) || handler.size < handler.header + 12)
        return false;
    uint8_t type[4];
    return seek_to(file, handler.offset + handler.header + 8) &&
           fread(type, 1, sizeof(type), file) == sizeof(type) &&
           memcmp(type, "vide", 4) == 0;
}

static bool locate_boxes(FILE *file, uint64_t file_size, Mp4Box *movie,
                         Mp4Box *track) {
    uint64_t cursor = 0, last_media_end = 0;
    bool found_movie = false;
    while (cursor < file_size) {
        Mp4Box box;
        if (!read_box(file, cursor, file_size, &box)) return false;
        if (memcmp(box.type, "mdat", 4) == 0) last_media_end = cursor + box.size;
        if (memcmp(box.type, "moov", 4) == 0) {
            *movie = box;
            found_movie = true;
        }
        cursor += box.size;
    }
    if (!found_movie || movie->offset < last_media_end) return false;
    cursor = movie->offset + movie->header;
    uint64_t end = movie->offset + movie->size;
    while (cursor < end) {
        Mp4Box box;
        if (!read_box(file, cursor, end, &box)) return false;
        if (memcmp(box.type, "trak", 4) == 0 && is_video_track(file, &box)) {
            *track = box;
            return true;
        }
        cursor += box.size;
    }
    return false;
}

static bool write_be32(FILE *file, uint32_t value) {
    uint8_t out[4] = {(uint8_t)(value >> 24), (uint8_t)(value >> 16),
                      (uint8_t)(value >> 8), (uint8_t)value};
    return fwrite(out, 1, sizeof(out), file) == sizeof(out);
}

static bool write_be64(FILE *file, uint64_t value) {
    uint8_t out[8];
    for (size_t i = 0; i < 8; ++i) out[7 - i] = (uint8_t)(value >> (i * 8));
    return fwrite(out, 1, sizeof(out), file) == sizeof(out);
}

static bool patch_size(FILE *file, const Mp4Box *box, uint64_t extra) {
    uint64_t size = box->size + extra;
    if (!seek_to(file, box->offset)) return false;
    if (box->extended) {
        return fseeko(file, 8, SEEK_CUR) == 0 && write_be64(file, size);
    }
    return size <= UINT32_MAX && write_be32(file, (uint32_t)size);
}

static bool copy_range(FILE *source, FILE *target, uint64_t count) {
    uint8_t buffer[64 * 1024];
    while (count) {
        size_t amount = count < sizeof(buffer) ? (size_t)count : sizeof(buffer);
        if (fread(buffer, 1, amount, source) != amount ||
            fwrite(buffer, 1, amount, target) != amount) return false;
        count -= amount;
    }
    return true;
}

static bool write_uuid_box(FILE *file, const char *xml) {
    static const uint8_t uuid[16] = {
        0xff, 0xcc, 0x82, 0x63, 0xf8, 0x55, 0x4a, 0x93,
        0x88, 0x14, 0x58, 0x7a, 0x02, 0x52, 0x1f, 0xdd};
    size_t xml_size = strlen(xml);
    uint64_t size = 8 + sizeof(uuid) + xml_size;
    return size <= UINT32_MAX && write_be32(file, (uint32_t)size) &&
           fwrite("uuid", 1, 4, file) == 4 &&
           fwrite(uuid, 1, sizeof(uuid), file) == sizeof(uuid) &&
           fwrite(xml, 1, xml_size, file) == xml_size;
}

bool sr_spatial_is_mp4(const char *path) {
    const char *dot = path ? strrchr(path, '.') : NULL;
    return dot && (!strcasecmp(dot, ".mp4") || !strcasecmp(dot, ".mov") ||
                   !strcasecmp(dot, ".m4v"));
}

SrStatus sr_spatial_inject_mp4(const char *path, uint32_t width,
                               uint32_t height, SrDiagnostics *diag) {
    FILE *source = fopen(path, "rb");
    if (!source || fseeko(source, 0, SEEK_END) != 0) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot open MP4 metadata target '%s'", path);
        if (source) fclose(source);
        return SR_ERR_IO;
    }
    off_t end = ftello(source);
    Mp4Box movie, track;
    if (end < 0 || !locate_boxes(source, (uint64_t)end, &movie, &track)) {
        sr_diag_error(diag, 0, NULL, NULL,
                      "MP4 has no patchable post-media video track for 360 metadata");
        fclose(source);
        return SR_ERR_ENCODER;
    }
    char xml[2048];
    int xml_size = snprintf(xml, sizeof(xml),
        "<?xml version=\"1.0\"?><rdf:SphericalVideo xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\" xmlns:GSpherical=\"http://ns.google.com/videos/1.0/spherical/\"><rdf:Description GSpherical:Spherical=\"true\" GSpherical:Stitched=\"true\" GSpherical:StitchingSoftware=\"scene-render\" GSpherical:ProjectionType=\"equirectangular\" GSpherical:StereoMode=\"mono\" GSpherical:SourceCount=\"1\" GSpherical:FullPanoWidthPixels=\"%u\" GSpherical:FullPanoHeightPixels=\"%u\" GSpherical:CroppedAreaImageWidthPixels=\"%u\" GSpherical:CroppedAreaImageHeightPixels=\"%u\" GSpherical:CroppedAreaLeftPixels=\"0\" GSpherical:CroppedAreaTopPixels=\"0\"/></rdf:SphericalVideo>",
        width, height, width, height);
    if (xml_size < 0 || (size_t)xml_size >= sizeof(xml)) {
        fclose(source);
        return SR_ERR_ENCODER;
    }
    uint64_t extra = 24 + (uint64_t)xml_size;
    size_t path_size = strlen(path) + 32;
    char *temporary = sr_alloc(path_size);
    if (!temporary) {
        fclose(source);
        return SR_ERR_MEMORY;
    }
    snprintf(temporary, path_size, "%s.spatial-XXXXXX", path);
    int descriptor = mkstemp(temporary);
    FILE *target = descriptor >= 0 ? fdopen(descriptor, "w+b") : NULL;
    bool ok = target && seek_to(source, 0) &&
              copy_range(source, target, track.offset + track.size) &&
              write_uuid_box(target, xml) &&
              copy_range(source, target,
                         (uint64_t)end - track.offset - track.size) &&
              patch_size(target, &track, extra) &&
              patch_size(target, &movie, extra) && fflush(target) == 0;
    if (fclose(source) != 0) ok = false;
    if (target && fclose(target) != 0) ok = false;
    if (!target && descriptor >= 0) close(descriptor);
    if (ok && rename(temporary, path) != 0) ok = false;
    if (!ok) {
        unlink(temporary);
        sr_diag_error(diag, 0, NULL, NULL,
                      "failed to inject spherical metadata into '%s': %s",
                      path, strerror(errno));
        free(temporary);
        return SR_ERR_IO;
    }
    free(temporary);
    sr_diag_info(diag, "injected Google Spatial Media v1 metadata into '%s'", path);
    return SR_OK;
}
