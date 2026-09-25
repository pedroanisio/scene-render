#define _POSIX_C_SOURCE 200809L
#include "scene_render/assets.h"
#include "scene_render/color.h"
#include "scene_render/mesh.h"
#include "scene_render/procedural.h"
#include "scene_render/video.h"

#include <ctype.h>
#include <errno.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

/* Wraps decoded 8-bit straight RGBA (consumed) as a blend-space image. */
static SrImage *sr_image_from_rgba8(const SrScene *scene, SrColorSpace space,
                                    uint8_t *rgba, uint32_t width,
                                    uint32_t height) {
    SrImage *image = sr_alloc(sizeof(*image));
    if (image && sr_color_image_from_rgba8(&scene->project, space, rgba,
                                           (size_t)width * 4, width, height,
                                           image) != SR_OK) {
        free(image);
        image = NULL;
    }
    free(rgba);
    return image;
}

/* Decodes a still image in-process (libavformat/libavcodec) to 8-bit RGBA
 * at the declared size, then converts it to blend space. PPM/PNM files must
 * match the declared size; other formats are resampled to it (Lanczos). */
static SrStatus sr_load_image(const SrScene *scene, const char *path,
                              SrAsset *asset, SrDiagnostics *diag) {
    const char *extension = strrchr(path, '.');
    bool exact = extension && (strcmp(extension, ".ppm") == 0 ||
                               strcmp(extension, ".pnm") == 0);
    uint8_t *rgba = NULL;
    char err[256] = "";
    SrStatus status = sr_image_decode_rgba8(path, asset->width, asset->height,
                                            !exact, &rgba, NULL, NULL, err,
                                            sizeof(err));
    if (status != SR_OK) {
        sr_diag_error(diag, asset->source_line, "image", "src",
                      "cannot decode '%s': %s", path, err);
        return status;
    }
    asset->decoded = sr_image_from_rgba8(scene, asset->source_color_space,
                                         rgba, asset->width, asset->height);
    return asset->decoded ? SR_OK : SR_ERR_MEMORY;
}

/* Opens the persistent decoder of a video asset and checks the stream
 * against the declared contract: dimensions must match; a differing frame
 * rate or duration is reported but the declared values still apply. */
static SrStatus sr_open_video(SrScene *scene, const char *path, SrAsset *asset,
                              SrDiagnostics *diag) {
    if (!asset->fps_num || !asset->fps_den || asset->duration <= 0.0) {
        sr_diag_error(diag, asset->source_line, "video", "fps/duration",
                      "video metadata must be positive");
        return SR_ERR_ASSET;
    }
    char err[256] = "";
    SrStatus status = sr_video_open(path, &scene->project,
                                    asset->source_color_space, asset->fps_num,
                                    asset->fps_den, SR_VIDEO_CACHE_DEFAULT_BYTES,
                                    &asset->video, err, sizeof(err));
    if (status != SR_OK) {
        sr_diag_error(diag, asset->source_line, "video", "src",
                      "cannot open '%s': %s", path, err);
        return status;
    }
    const SrVideoInfo *info = sr_video_info(asset->video);
    if (info->width != asset->width || info->height != asset->height) {
        sr_diag_error(diag, asset->source_line, "video", "width/height",
                      "declared dimensions %ux%u do not match the stream's %ux%u",
                      asset->width, asset->height, info->width, info->height);
        sr_video_close(asset->video);
        asset->video = NULL;
        return SR_ERR_ASSET;
    }
    if (info->rate_num > 0 && info->rate_den > 0 &&
        (uint64_t)info->rate_num * asset->fps_den !=
            (uint64_t)asset->fps_num * (uint64_t)info->rate_den)
        sr_diag_warning(diag, asset->source_line, "video", "fps",
                        "declared %u/%u fps but the stream reports %d/%d; "
                        "frames are indexed at the declared rate",
                        asset->fps_num, asset->fps_den, info->rate_num,
                        info->rate_den);
    double fps = (double)asset->fps_num / asset->fps_den;
    int64_t declared = (int64_t)ceil(asset->duration * fps - 1e-12);
    if (declared != info->frame_count)
        sr_diag_warning(diag, asset->source_line, "video", "duration",
                        "declared %lld frames but the stream has %lld at the "
                        "declared rate", (long long)declared,
                        (long long)info->frame_count);
    return SR_OK;
}

static bool sr_font_name_valid(const char *name) {
    if (!name || !*name) return false;
    for (const unsigned char *p=(const unsigned char *)name;*p;++p)
        if (!isalnum(*p) && *p!=' ' && *p!='_' && *p!='-' && *p!='.')
            return false;
    return true;
}

static char *sr_filter_escape(const char *text) {
    size_t length=0;
    for(const char *p=text;*p;++p)length+=(*p==':'||*p=='\\'||*p=='\'')?2:1;
    char *result=sr_alloc(length+1);if(!result)return NULL;char *out=result;
    for(const char *p=text;*p;++p){if(*p==':'||*p=='\\'||*p=='\'')*out++='\\';*out++=*p;}
    *out='\0';return result;
}

static SrImage *sr_render_text_ffmpeg(const SrScene *scene,SrAsset *asset,
                                      SrDiagnostics *diag){
    char temporary[]="/tmp/scene-render-text-XXXXXX";int text_fd=mkstemp(temporary);
    if(text_fd<0)return NULL;
    size_t length=strlen(asset->text),written=0;
    while(written<length){ssize_t amount=write(text_fd,asset->text+written,length-written);
        if(amount>0)written+=(size_t)amount;else if(amount<0&&errno==EINTR)continue;else break;}
    close(text_fd);if(written!=length){unlink(temporary);return NULL;}
    char *font_path=asset->font_file?sr_path_join(scene->base_dir,asset->font_file):NULL;
    char *font_value=sr_filter_escape(font_path?font_path:asset->font_family);
    char *text_path=sr_filter_escape(temporary);free(font_path);
    if(!font_value||!text_path){free(font_value);free(text_path);unlink(temporary);return NULL;}
    char input[128],filter[2048],color[64];
    snprintf(input,sizeof(input),"color=c=black@0.0:s=%ux%u:r=1,format=rgba",
             asset->width,asset->height);
    snprintf(color,sizeof(color),"0x%02X%02X%02X@%.9f",
             (unsigned)lrint(asset->color.r*255.0),
             (unsigned)lrint(asset->color.g*255.0),
             (unsigned)lrint(asset->color.b*255.0),asset->color.a);
    snprintf(filter,sizeof(filter),
             "drawtext=%s='%s':textfile='%s':fontsize=%.9g:fontcolor=%s:"
             "x=0:y=0:text_shaping=1:fix_bounds=1",
             asset->font_file?"fontfile":"font",font_value,text_path,
             asset->text_size,color);
    free(font_value);free(text_path);
    int output[2];if(pipe(output)!=0){unlink(temporary);return NULL;}
    pid_t pid=fork();if(pid<0){close(output[0]);close(output[1]);unlink(temporary);return NULL;}
    if(pid==0){dup2(output[1],STDOUT_FILENO);close(output[0]);close(output[1]);
        const char *argv[]={"ffmpeg","-nostdin","-v","error","-f","lavfi",
            "-i",input,"-vf",filter,"-frames:v","1","-f","rawvideo",
            "-pix_fmt","rgba","pipe:1",NULL};
        execvp(argv[0],(char *const *)argv);_exit(127);}
    close(output[1]);size_t pixels=(size_t)asset->width*asset->height;
    if ((asset->height && pixels / asset->height != asset->width) ||
        pixels > SIZE_MAX / 4) {
        close(output[0]);waitpid(pid,NULL,0);unlink(temporary);return NULL;
    }
    size_t size=pixels*4;uint8_t *rgba=sr_alloc(size);
    if(!rgba){close(output[0]);waitpid(pid,NULL,0);unlink(temporary);return NULL;}
    size_t received=0;while(received<size){ssize_t amount=read(output[0],rgba+received,size-received);
        if(amount>0)received+=(size_t)amount;else if(amount<0&&errno==EINTR)continue;else break;}
    close(output[0]);int status=0;pid_t waited;do{waited=waitpid(pid,&status,0);}while(waited<0&&errno==EINTR);
    unlink(temporary);if(received!=size||waited<0||!WIFEXITED(status)||WEXITSTATUS(status)!=0){
        sr_diag_error(diag,asset->source_line,"text",asset->font_file?"fontFile":"font",
                      "FFmpeg drawtext could not shape the UTF-8 text");free(rgba);return NULL;}
    /* Text colors are working-space values, like every XML color. */
    return sr_image_from_rgba8(scene,scene->project.working_color_space,rgba,
                               asset->width,asset->height);
}

SrStatus sr_assets_load(SrScene *scene, SrDiagnostics *diag) {
    for (size_t i = 0; i < scene->asset_count; ++i) {
        SrAsset *asset = &scene->assets[i];
        if (asset->decoded || asset->video || asset->type == SR_ASSET_AUDIO) {
            continue;
        }
        if(asset->type==SR_ASSET_MESH){char *path=sr_path_join(scene->base_dir,asset->source);
            if(!path)return SR_ERR_MEMORY;
            SrStatus status=sr_mesh_load_obj(path,&asset->mesh,asset->source_line,diag);
            free(path);if(status!=SR_OK)return status;continue;}
        if (asset->type == SR_ASSET_TEXT) {
            if (!sr_font_name_valid(asset->font_family)) {
                sr_diag_error(diag,asset->source_line,"text","font",
                              "font family contains unsupported characters");
                return SR_ERR_ASSET;
            }
            asset->decoded=sr_render_text_ffmpeg(scene,asset,diag);
            if(!asset->decoded)return SR_ERR_ASSET;
            continue;
        }
        if (asset->type == SR_ASSET_VECTOR) {
            SrStatus status = sr_procedural_asset(&scene->project, asset, diag);
            if (status != SR_OK) return status;
            continue;
        }
        char *path = sr_path_join(scene->base_dir, asset->source);
        if (!path) {
            return SR_ERR_MEMORY;
        }
        sr_diag_info(diag, "decoding shared asset '%s' from %s", asset->id, path);
        SrStatus status = asset->type == SR_ASSET_VIDEO
                              ? sr_open_video(scene, path, asset, diag)
                              : sr_load_image(scene, path, asset, diag);
        free(path);
        if (status != SR_OK) {
            return status;
        }
    }
    return SR_OK;
}

void sr_assets_unload(SrScene *scene) {
    for (size_t i = 0; i < scene->asset_count; ++i) {
        SrAsset *asset = &scene->assets[i];
        if (asset->decoded) {
            free(asset->decoded->px);
            free(asset->decoded);
            asset->decoded = NULL;
        }
        sr_video_close(asset->video);
        asset->video = NULL;
        free(asset->audio_pcm);
        asset->audio_pcm = NULL;
        asset->audio_frames = 0;
        asset->audio_decoded = false;
    }
}

void sr_assets_video_stats(const SrScene *scene, size_t *sources,
                           uint64_t totals[4]) {
    *sources = 0;
    for (size_t k = 0; k < 4; ++k) totals[k] = 0;
    for (size_t i = 0; i < scene->asset_count; ++i) {
        const SrVideoSource *video = scene->assets[i].video;
        if (!video) continue;
        const SrVideoStats *stats = sr_video_stats(video);
        ++*sources;
        totals[0] += stats->requests;
        totals[1] += stats->cache_hits;
        totals[2] += stats->decoded;
        totals[3] += stats->seeks;
    }
}

SrImage *sr_asset_get_frame(SrScene *scene, SrAsset *asset, double source_time,
                            SrDiagnostics *diag) {
    (void)scene;
    if (!asset) return NULL;
    if (asset->type != SR_ASSET_VIDEO) return asset->decoded;
    if (!asset->video) {
        sr_diag_error(diag, asset->source_line, "video", "src",
                      "video asset '%s' is not loaded", asset->id);
        return NULL;
    }
    double fps = (double)asset->fps_num / asset->fps_den;
    int64_t total = (int64_t)ceil(asset->duration * fps - 1e-12);
    int64_t index = (int64_t)floor(fmax(0.0, source_time) * fps + 1e-9);
    if (total > 0 && index >= total) index = total - 1;
    const SrImage *image = NULL;
    char err[256] = "";
    if (sr_video_frame(asset->video, index, &image, err, sizeof(err)) != SR_OK) {
        sr_diag_error(diag, asset->source_line, "video", "src",
                      "cannot decode frame %lld of '%s': %s", (long long)index,
                      asset->id, err);
        return NULL;
    }
    return (SrImage *)image;
}
