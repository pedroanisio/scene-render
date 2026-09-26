/* SPDX-License-Identifier: Apache-2.0 */
#include "metadata.h"

#include <string.h>

static unsigned char upper_ascii(unsigned char c) {
    return c >= 'a' && c <= 'z' ? (unsigned char)(c - 'a' + 'A') : c;
}

bool sr_metadata_name_equal(const char *left, const char *right) {
    while (*left && upper_ascii((unsigned char)*left) ==
                    upper_ascii((unsigned char)*right)) {
        ++left;
        ++right;
    }
    return *left == *right;
}

/* Keys have already passed the loader's byte limit. Matroska's native aliases
 * and space folding must be applied before collision detection or insertion. */
static void canonical_key(const char *name, bool matroska,
                            char key[SR_MAX_METADATA_NAME + 1]) {
    size_t i = 0;
    for (; name[i]; ++i) {
        unsigned char c = (unsigned char)name[i];
        key[i] = (char)(matroska ? (c == ' ' ? '_' : upper_ascii(c)) : c);
    }
    key[i] = '\0';
    if (matroska && !strcmp(key, "PERFORMER")) strcpy(key, "LEAD_PERFORMER");
    if (matroska && !strcmp(key, "TRACK")) strcpy(key, "PART_NUMBER");
}

static bool reserved_key(const char *key, bool matroska) {
    char upper[SR_MAX_METADATA_NAME + 1];
    size_t size = strlen(key);
    for (size_t i = 0; i < size; ++i)
        upper[i] = (char)upper_ascii((unsigned char)key[i]);
    upper[size] = '\0';
    if (!strcmp(upper, "ENCODER") || !strncmp(upper, "ENCODER-", 8) ||
        !strcmp(upper, "CREATION_TIME")) return true;
    if (matroska)
        return !strcmp(upper, "DURATION") || !strcmp(upper, "ENCODING_TOOL") ||
               !strcmp(upper, "STEREO_MODE") || !strcmp(upper, "ALPHA_MODE");
    return !strcmp(upper, "LOCATION") ||
           !strcmp(upper, "COM.APPLE.QUICKTIME.ARTWORK");
}

SrStatus sr_metadata_validate(const SrScene *scene, const char *container,
                               SrDiagnostics *diag) {
    if (!scene->output.embed_metadata || !scene->metadata_count) return SR_OK;
    if (scene->metadata_count > SR_MAX_METADATA_ENTRIES || !scene->metadata)
        return SR_ERR_ARGUMENT;
    bool matroska = !strcmp(container, "matroska");
    char keys[SR_MAX_METADATA_ENTRIES][SR_MAX_METADATA_NAME + 1];
    for (size_t i = 0; i < scene->metadata_count; ++i) {
        const SrMetadataEntry *entry = &scene->metadata[i];
        if (!entry->name || !entry->value || !entry->name[0] ||
            strlen(entry->name) > SR_MAX_METADATA_NAME ||
            strlen(entry->value) > SR_MAX_METADATA_VALUE) {
            sr_diag_error(diag, entry->source_line, "meta", "name/value",
                          "invalid metadata entry or byte limit exceeded");
            return SR_ERR_ARGUMENT;
        }
        canonical_key(entry->name, matroska, keys[i]);
        if (reserved_key(keys[i], matroska)) {
            sr_diag_error(diag, entry->source_line, "meta", "name",
                          "metadata key '%s' is reserved by %s and cannot "
                          "preserve its authored value", entry->name, container);
            return SR_ERR_ARGUMENT;
        }
        for (size_t j = 0; j < i; ++j) {
            if (sr_metadata_name_equal(keys[j], keys[i])) {
                sr_diag_error(diag, entry->source_line, "meta", "name",
                              "metadata key '%s' collides with '%s' in %s",
                              entry->name, scene->metadata[j].name, container);
                return SR_ERR_ARGUMENT;
            }
        }
    }
    return SR_OK;
}

SrStatus sr_metadata_apply(const SrScene *scene, AVFormatContext *format,
                            const char *container, SrDiagnostics *diag) {
    bool matroska = !strcmp(container, "matroska");
    for (size_t i = 0; i < scene->metadata_count; ++i) {
        const SrMetadataEntry *entry = &scene->metadata[i];
        char key[SR_MAX_METADATA_NAME + 1];
        canonical_key(entry->name, matroska, key);
        int rc = av_dict_set(&format->metadata, key, entry->value, 0);
        if (rc < 0) {
            sr_diag_error(diag, entry->source_line, "meta", "value",
                          "cannot allocate %s metadata key '%s'", container, key);
            return rc == AVERROR(ENOMEM) ? SR_ERR_MEMORY : SR_ERR_ENCODER;
        }
    }
    return SR_OK;
}

SrStatus sr_metadata_check(const SrScene *scene, const AVFormatContext *format,
                            const char *container, SrDiagnostics *diag) {
    bool matroska = !strcmp(container, "matroska");
    for (size_t i = 0; i < scene->metadata_count; ++i) {
        const SrMetadataEntry *entry = &scene->metadata[i];
        char key[SR_MAX_METADATA_NAME + 1];
        canonical_key(entry->name, matroska, key);
        const AVDictionaryEntry *actual = av_dict_get(format->metadata, key,
                                                     NULL, AV_DICT_MATCH_CASE);
        if (!actual || strcmp(entry->value, actual->value)) {
            sr_diag_error(diag, entry->source_line, "meta", "value",
                          "metadata key '%s' was lost or changed while "
                          "writing the %s header", entry->name, container);
            return SR_ERR_ENCODER;
        }
    }
    return SR_OK;
}
