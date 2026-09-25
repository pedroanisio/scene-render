#define _POSIX_C_SOURCE 200809L
#include "scene_render/spatial.h"

#include "spatial_internal.h"

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

/* ---- in-memory box walking (the moov box) ------------------------------ */

typedef struct {
    size_t offset;
    size_t size;
    size_t header;
    bool extended;
} MemBox;

static bool mem_box(const uint8_t *data, size_t offset, size_t limit,
                    MemBox *box) {
    if (limit < offset || limit - offset < 8) return false;
    uint32_t compact = read_be32(data + offset);
    *box = (MemBox){.offset = offset, .header = 8};
    if (compact == 1) {
        if (limit - offset < 16) return false;
        uint64_t size = read_be64(data + offset + 8);
        if (size > SIZE_MAX) return false;
        box->size = (size_t)size;
        box->header = 16;
        box->extended = true;
    } else {
        box->size = compact ? compact : limit - offset;
    }
    return box->size >= box->header && box->size <= limit - offset;
}

static bool mem_type(const uint8_t *data, const MemBox *box, const char *type) {
    return memcmp(data + box->offset + 4, type, 4) == 0;
}

static bool mem_child(const uint8_t *data, const MemBox *parent,
                      const char *type, MemBox *result) {
    size_t cursor = parent->offset + parent->header;
    size_t end = parent->offset + parent->size;
    while (cursor < end) {
        MemBox box;
        if (!mem_box(data, cursor, end, &box)) return false;
        if (mem_type(data, &box, type)) {
            *result = box;
            return true;
        }
        cursor += box.size;
    }
    return false;
}

static bool mem_is_video_track(const uint8_t *data, const MemBox *track) {
    MemBox media, handler;
    return mem_child(data, track, "mdia", &media) &&
           mem_child(data, &media, "hdlr", &handler) &&
           handler.size >= handler.header + 12 &&
           memcmp(data + handler.offset + handler.header + 8, "vide", 4) == 0;
}

static void write_be32_mem(uint8_t *out, uint32_t value) {
    out[0] = (uint8_t)(value >> 24); out[1] = (uint8_t)(value >> 16);
    out[2] = (uint8_t)(value >> 8); out[3] = (uint8_t)value;
}

static void write_be64_mem(uint8_t *out, uint64_t value) {
    for (size_t i = 0; i < 8; ++i) out[7 - i] = (uint8_t)(value >> (i * 8));
}

static bool mem_grow(uint8_t *data, const MemBox *box, size_t extra) {
    uint64_t size = (uint64_t)box->size + extra;
    if (box->extended) {
        write_be64_mem(data + box->offset + 8, size);
        return true;
    }
    if (size > UINT32_MAX) return false;
    write_be32_mem(data + box->offset, (uint32_t)size);
    return true;
}

/* The chunk-offset table (stco or co64) of the trak found at or after
 * *cursor in `movie`, with its ancestors; *cursor moves past that trak.
 * 1 = found, 0 = no more traks, -1 = malformed. */
typedef struct {
    MemBox track, media, info, table, offsets;
    bool wide;
    uint32_t count;
} OffsetTable;

static int next_offset_table(const uint8_t *data, const MemBox *movie,
                             size_t *cursor, OffsetTable *out) {
    size_t end = movie->offset + movie->size;
    while (*cursor < end) {
        OffsetTable t = {0};
        if (!mem_box(data, *cursor, end, &t.track)) return -1;
        *cursor += t.track.size;
        if (!mem_type(data, &t.track, "trak")) continue;
        if (!mem_child(data, &t.track, "mdia", &t.media) ||
            !mem_child(data, &t.media, "minf", &t.info) ||
            !mem_child(data, &t.info, "stbl", &t.table)) continue;
        if (!mem_child(data, &t.table, "stco", &t.offsets)) {
            if (!mem_child(data, &t.table, "co64", &t.offsets)) continue;
            t.wide = true;
        }
        size_t body = t.offsets.offset + t.offsets.header;
        if (t.offsets.size < t.offsets.header + 8) return -1;
        t.count = read_be32(data + body + 4);
        if ((t.offsets.size - t.offsets.header - 8) / (t.wide ? 8 : 4) < t.count)
            return -1;
        *out = t;
        return 1;
    }
    return 0;
}

/* True when shifting this 32-bit table by `shift` pushes an entry at or
 * past `threshold` beyond UINT32_MAX. */
static bool needs_co64(const uint8_t *data, const OffsetTable *t,
                       uint64_t threshold, uint64_t shift) {
    if (t->wide) return false;
    const uint8_t *at = data + t->offsets.offset + t->offsets.header + 8;
    for (uint32_t i = 0; i < t->count; ++i, at += 4) {
        uint64_t value = read_be32(at);
        if (value >= threshold && value + shift > UINT32_MAX) return true;
    }
    return false;
}

/* Rewrites stco `t` of the moov buffer (*data, *size) as co64: the table
 * and every ancestor grow by 4 bytes per entry. */
static bool promote_to_co64(uint8_t **data, size_t *size, const OffsetTable *t) {
    size_t growth = (size_t)t->count * 4;
    MemBox root;
    if (!mem_box(*data, 0, *size, &root) || *size > SIZE_MAX - growth) return false;
    uint8_t *grown = realloc(*data, *size + growth);
    if (!grown) return false;
    *data = grown;
    size_t entries = t->offsets.offset + t->offsets.header + 8;
    size_t tail = entries + (size_t)t->count * 4;
    memmove(grown + tail + growth, grown + tail, *size - tail);
    for (uint32_t i = t->count; i-- > 0;)
        write_be64_mem(grown + entries + (size_t)i * 8,
                       read_be32(grown + entries + (size_t)i * 4));
    memcpy(grown + t->offsets.offset + 4, "co64", 4);
    *size += growth;
    return mem_grow(grown, &t->offsets, growth) && mem_grow(grown, &t->table, growth) &&
           mem_grow(grown, &t->info, growth) && mem_grow(grown, &t->media, growth) &&
           mem_grow(grown, &t->track, growth) && mem_grow(grown, &root, growth);
}

/* Adds `extra` to every stco/co64 chunk offset at or past `threshold` (the
 * original end of moov) in every track: needed when the moov box precedes
 * media data it grows in front of. */
static bool shift_chunk_offsets(uint8_t *data, const MemBox *movie,
                                uint64_t threshold, uint64_t extra) {
    size_t cursor = movie->offset + movie->header;
    OffsetTable t;
    int found;
    while ((found = next_offset_table(data, movie, &cursor, &t)) > 0) {
        uint8_t *at = data + t.offsets.offset + t.offsets.header + 8;
        for (uint32_t i = 0; i < t.count; ++i, at += t.wide ? 8 : 4) {
            if (t.wide) {
                uint64_t value = read_be64(at);
                if (value >= threshold) {
                    if (value > UINT64_MAX - extra) return false;
                    write_be64_mem(at, value + extra);
                }
            } else {
                uint64_t value = read_be32(at);
                if (value < threshold) continue;
                value += extra;
                if (value > UINT32_MAX) return false;
                write_be32_mem(at, (uint32_t)value);
            }
        }
    }
    return found == 0;
}

bool sr_spatial_shift_offsets(uint8_t **moov, size_t *size, uint64_t threshold,
                              uint64_t extra) {
    /* Each promotion grows moov, which moves the media further and can push
     * another table over the limit: repeat until no table needs it. */
    uint64_t growth = 0;
    for (;;) {
        MemBox root;
        if (!mem_box(*moov, 0, *size, &root) || root.size != *size) return false;
        size_t cursor = root.header;
        OffsetTable t;
        int found;
        while ((found = next_offset_table(*moov, &root, &cursor, &t)) > 0 &&
               !needs_co64(*moov, &t, threshold, extra + growth)) {}
        if (found < 0) return false;
        if (found == 0) break;
        if (!promote_to_co64(moov, size, &t)) return false;
        growth += (uint64_t)t.count * 4;
    }
    MemBox root;
    return mem_box(*moov, 0, *size, &root) &&
           shift_chunk_offsets(*moov, &root, threshold, extra + growth);
}

/* Finds the top-level moov box and whether any mdat follows it. */
static bool locate_movie(FILE *file, uint64_t file_size, Mp4Box *movie,
                         bool *media_after) {
    uint64_t cursor = 0;
    bool found = false;
    *media_after = false;
    while (cursor < file_size) {
        Mp4Box box;
        if (!read_box(file, cursor, file_size, &box)) return false;
        if (memcmp(box.type, "moov", 4) == 0) {
            *movie = box;
            found = true;
        } else if (found && memcmp(box.type, "mdat", 4) == 0) {
            *media_after = true;
        }
        cursor += box.size;
    }
    return found;
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

bool sr_spatial_is_mp4(const char *path) {
    const char *dot = path ? strrchr(path, '.') : NULL;
    return dot && (!strcasecmp(dot, ".mp4") || !strcasecmp(dot, ".mov") ||
                   !strcasecmp(dot, ".m4v"));
}

static size_t uuid_box(uint8_t *out, const char *xml) {
    static const uint8_t uuid[16] = {
        0xff, 0xcc, 0x82, 0x63, 0xf8, 0x55, 0x4a, 0x93,
        0x88, 0x14, 0x58, 0x7a, 0x02, 0x52, 0x1f, 0xdd};
    size_t xml_size = strlen(xml);
    write_be32_mem(out, (uint32_t)(8 + sizeof(uuid) + xml_size));
    memcpy(out + 4, "uuid", 4);
    memcpy(out + 8, uuid, sizeof(uuid));
    memcpy(out + 24, xml, xml_size);
    return 24 + xml_size;
}

/* Inserts a Spherical Video V1 uuid box at the end of the first video trak.
 * Works for both layouts: moov after the media data (nothing else moves) and
 * moov first ("faststart"; every chunk offset then moves by the inserted
 * size). The moov box is rebuilt in memory and the file rewritten. */
SrStatus sr_spatial_inject_mp4(const char *path, uint32_t width,
                               uint32_t height, SrDiagnostics *diag) {
    FILE *source = fopen(path, "rb");
    if (!source || fseeko(source, 0, SEEK_END) != 0) {
        sr_diag_error(diag, 0, NULL, NULL, "cannot open MP4 metadata target '%s'", path);
        if (source) fclose(source);
        return SR_ERR_IO;
    }
    off_t end = ftello(source);
    Mp4Box movie = {0};
    bool media_after = false;
    if (end < 0 || !locate_movie(source, (uint64_t)end, &movie, &media_after) ||
        movie.size > (64u << 20)) {
        sr_diag_error(diag, 0, NULL, NULL,
                      "MP4 has no patchable moov box for 360 metadata");
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
    size_t extra = 24 + (size_t)xml_size;
    size_t moov_size = (size_t)movie.size;
    uint8_t *moov = sr_alloc(moov_size + extra);
    if (!moov) {
        fclose(source);
        return SR_ERR_MEMORY;
    }
    bool ok = seek_to(source, movie.offset) &&
              fread(moov, 1, moov_size, source) == moov_size;
    MemBox root = {0}, track = {0};
    bool found = false;
    if (ok && mem_box(moov, 0, moov_size, &root)) {
        size_t cursor = root.header;
        while (cursor < moov_size) {
            MemBox box;
            if (!mem_box(moov, cursor, moov_size, &box)) break;
            if (mem_type(moov, &box, "trak") && mem_is_video_track(moov, &box)) {
                track = box;
                found = true;
                break;
            }
            cursor += box.size;
        }
    }
    if (!ok || !found) {
        free(moov);
        fclose(source);
        sr_diag_error(diag, 0, NULL, NULL,
                      "MP4 has no patchable video track for 360 metadata");
        return SR_ERR_ENCODER;
    }
    size_t insert = track.offset + track.size;
    memmove(moov + insert + extra, moov + insert, moov_size - insert);
    uuid_box(moov + insert, xml);
    ok = mem_grow(moov, &track, extra) && mem_grow(moov, &root, extra);
    size_t out_size = moov_size + extra;
    if (ok && media_after)
        ok = sr_spatial_shift_offsets(&moov, &out_size, movie.offset + movie.size,
                                      extra);
    size_t path_size = strlen(path) + 32;
    char *temporary = ok ? sr_alloc(path_size) : NULL;
    int descriptor = -1;
    FILE *target = NULL;
    if (temporary) {
        snprintf(temporary, path_size, "%s.spatial-XXXXXX", path);
        descriptor = mkstemp(temporary);
        target = descriptor >= 0 ? fdopen(descriptor, "w+b") : NULL;
    }
    uint64_t after = movie.offset + movie.size;
    ok = ok && target && seek_to(source, 0) &&
         copy_range(source, target, movie.offset) &&
         fwrite(moov, 1, out_size, target) == out_size &&
         seek_to(source, after) &&
         copy_range(source, target, (uint64_t)end - after) &&
         fflush(target) == 0;
    free(moov);
    if (fclose(source) != 0) ok = false;
    if (target && fclose(target) != 0) ok = false;
    if (!target && descriptor >= 0) close(descriptor);
    if (ok && rename(temporary, path) != 0) ok = false;
    if (!ok) {
        if (temporary) unlink(temporary);
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
