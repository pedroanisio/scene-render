#include "scene_render/effects.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>

static uint8_t byte_value(double value){return(uint8_t)lrint(fmax(0.0,fmin(1.0,value))*255.0);}

static void blur_pass(const uint8_t *source,uint8_t *target,uint32_t width,uint32_t height,int radius,bool horizontal){
    uint32_t outer=horizontal?height:width,inner=horizontal?width:height;
    for(uint32_t o=0;o<outer;++o)for(uint32_t i=0;i<inner;++i){int first=(int)i-radius,last=(int)i+radius;
        int count=0,sum[4]={0};for(int p=first;p<=last;++p){int q=p<0?0:p>=(int)inner?(int)inner-1:p;
            size_t at=horizontal?((size_t)o*width+(uint32_t)q)*4:((size_t)q*width+o)*4;
            for (int c = 0; c < 4; ++c) sum[c] += source[at+c];
            ++count;
        }
        size_t out=horizontal?((size_t)o*width+i)*4:((size_t)i*width+o)*4;
        for(int c=0;c<4;++c)target[out+c]=(uint8_t)(sum[c]/count);}
}

static SrStatus blur_image(SrFrame *frame,int radius,uint8_t **result){
    size_t bytes=(size_t)frame->width*frame->height*4;uint8_t *a=sr_alloc(bytes),*b=sr_alloc(bytes);
    if(!a||!b){free(a);free(b);return SR_ERR_MEMORY;}memcpy(a,frame->rgba,bytes);
    blur_pass(a,b,frame->width,frame->height,radius,true);blur_pass(b,a,frame->width,frame->height,radius,false);free(b);*result=a;return SR_OK;
}

static void grade(SrFrame *frame,const SrEffect *effect,double intensity){size_t pixels=(size_t)frame->width*frame->height;
    for(size_t i=0;i<pixels;++i){double r=frame->rgba[i*4]/255.0,g=frame->rgba[i*4+1]/255.0,b=frame->rgba[i*4+2]/255.0;
        double l=.2126*r+.7152*g+.0722*b;r=l+(r-l)*effect->saturation;g=l+(g-l)*effect->saturation;b=l+(b-l)*effect->saturation;
        r=(r-.5)*effect->contrast+.5+effect->brightness;g=(g-.5)*effect->contrast+.5+effect->brightness;b=(b-.5)*effect->contrast+.5+effect->brightness;
        frame->rgba[i*4]=byte_value(frame->rgba[i*4]/255.0+(r-frame->rgba[i*4]/255.0)*intensity);
        frame->rgba[i*4+1]=byte_value(frame->rgba[i*4+1]/255.0+(g-frame->rgba[i*4+1]/255.0)*intensity);
        frame->rgba[i*4+2]=byte_value(frame->rgba[i*4+2]/255.0+(b-frame->rgba[i*4+2]/255.0)*intensity);}}

static void vignette(SrFrame *frame,double intensity){for(uint32_t y=0;y<frame->height;++y)for(uint32_t x=0;x<frame->width;++x){
    double nx=2.0*(x+.5)/frame->width-1,ny=2.0*(y+.5)/frame->height-1;double factor=fmax(0.0,1.0-intensity*.55*(nx*nx+ny*ny));
    size_t at=((size_t)y*frame->width+x)*4;frame->rgba[at]=(uint8_t)(frame->rgba[at]*factor);frame->rgba[at+1]=(uint8_t)(frame->rgba[at+1]*factor);frame->rgba[at+2]=(uint8_t)(frame->rgba[at+2]*factor);}}

static void flare(SrFrame *frame,const SrEffect *effect,double intensity,double radius){double sx=frame->width*.72,sy=frame->height*.28;
    for(int ghost=0;ghost<4;++ghost){double t=(ghost+1)/5.0,cx=sx+(frame->width*.5-sx)*t*1.7,cy=sy+(frame->height*.5-sy)*t*1.7,rr=fmax(2.0,radius*(1+.35*ghost));
        for(int y=(int)(cy-rr);y<=(int)(cy+rr);++y)for(int x=(int)(cx-rr);x<=(int)(cx+rr);++x){if(x<0||y<0||x>=(int)frame->width||y>=(int)frame->height)continue;
            double dx=x+.5-cx,dy=y+.5-cy,d=sqrt(dx*dx+dy*dy)/rr;if(d>1)continue;double a=(1-d)*intensity*.18;size_t at=((size_t)y*frame->width+x)*4;
            frame->rgba[at]=byte_value(frame->rgba[at]/255.0+effect->color.r*a);frame->rgba[at+1]=byte_value(frame->rgba[at+1]/255.0+effect->color.g*a);frame->rgba[at+2]=byte_value(frame->rgba[at+2]/255.0+effect->color.b*a);}}}

SrStatus sr_effects_apply(const SrScene *scene,double time,SrFrame *frame,SrDiagnostics *diag){(void)diag;
    for(size_t e=0;e<scene->effect_count;++e){const SrEffect *effect=&scene->effects[e];if(!effect->enabled)continue;
        double intensity=fmax(0.0,sr_anim_eval(&effect->intensity,time));int radius=(int)fmin(64.0,fmax(1.0,sr_anim_eval(&effect->radius,time)));
        if (effect->type == SR_EFFECT_COLOR_GRADE) {
            grade(frame, effect, intensity);
        } else if (effect->type == SR_EFFECT_VIGNETTE) {
            vignette(frame, intensity);
        } else if (effect->type == SR_EFFECT_LENS_FLARE) {
            flare(frame, effect, intensity, radius);
        } else {
            uint8_t *blurred = NULL;
            SrStatus status = blur_image(frame, radius, &blurred);
            if (status != SR_OK) return status;
            size_t pixels = (size_t)frame->width * frame->height;
            if (effect->type == SR_EFFECT_BLUR) {
                for (size_t i = 0; i < pixels * 4; ++i)
                    frame->rgba[i] = (uint8_t)lrint(frame->rgba[i] +
                        (blurred[i] - frame->rgba[i]) * fmin(1.0, intensity));
            } else {
                for (size_t i = 0; i < pixels; ++i) {
                    double luminance = (blurred[i*4] + blurred[i*4+1] +
                                        blurred[i*4+2]) / (3.0 * 255.0);
                    if (luminance >= effect->threshold)
                        for (int c = 0; c < 3; ++c)
                            frame->rgba[i*4+c] = byte_value(
                                frame->rgba[i*4+c]/255.0 +
                                blurred[i*4+c]/255.0*intensity*.5);
                }
            }
            free(blurred);
        }
    }
    return SR_OK;}
