#include "scene_render/lighting.h"

#include <math.h>

typedef struct { double x, y, z; } Vec3;

static const SrCamera *scene_camera(const SrScene *scene) {
    for (size_t i = 0; i < scene->camera_count; ++i)
        if (scene->cameras[i].active) return &scene->cameras[i];
    return NULL;
}

static Vec3 rotate_x(Vec3 p,double a){double c=cos(a),s=sin(a);return(Vec3){p.x,p.y*c-p.z*s,p.y*s+p.z*c};}
static Vec3 rotate_y(Vec3 p,double a){double c=cos(a),s=sin(a);return(Vec3){p.x*c+p.z*s,p.y,-p.x*s+p.z*c};}
static Vec3 rotate_z(Vec3 p,double a){double c=cos(a),s=sin(a);return(Vec3){p.x*c-p.y*s,p.x*s+p.y*c,p.z};}

static bool project(const SrScene *scene,const SrObject3D *object,double time,
                    double *x,double *y,double *scale){
    double ox=sr_anim_eval(&object->transform.x,time),oy=sr_anim_eval(&object->transform.y,time),oz=sr_anim_eval(&object->transform.z,time);
    const SrCamera *camera=scene_camera(scene);if(!camera){*x=ox;*y=oy;*scale=1;return true;}
    Vec3 p={ox-sr_anim_eval(&camera->x,time),oy-sr_anim_eval(&camera->y,time),oz-sr_anim_eval(&camera->z,time)};
    p=rotate_y(p,-sr_anim_eval(&camera->yaw,time)*SR_PI/180.0);p=rotate_x(p,-sr_anim_eval(&camera->pitch,time)*SR_PI/180.0);p=rotate_z(p,-sr_anim_eval(&camera->roll,time)*SR_PI/180.0);
    if(camera->orthographic){*scale=1;*x=scene->project.width*.5+p.x;*y=scene->project.height*.5-p.y;return true;}
    if (p.z < camera->near_plane || p.z > camera->far_plane) return false;
    double focal = scene->project.height * .5 /
                   tan(sr_anim_eval(&camera->fov,time) * SR_PI / 360.0);
    *scale = focal / p.z;
    *x = scene->project.width * .5 + p.x * *scale;
    *y = scene->project.height * .5 - p.y * *scale;
    return true;
}

static double clamp01(double value) {
    return fmax(0.0, fmin(1.0, value));
}
static Vec3 normalize(Vec3 value) {
    double length = sqrt(value.x * value.x + value.y * value.y + value.z * value.z);
    return length > 1e-12 ? (Vec3){value.x/length,value.y/length,value.z/length}
                          : (Vec3){0,0,1};
}
static double dot(Vec3 a, Vec3 b) { return a.x*b.x+a.y*b.y+a.z*b.z; }

static void over(SrFrame *frame, int x, int y, SrColor color, double alpha) {
    if (x < 0 || y < 0 || x >= (int)frame->width || y >= (int)frame->height) return;
    size_t at=((size_t)y*frame->width+(size_t)x)*4;
    double a=clamp01(alpha*color.a), inv=1.0-a;
    frame->rgba[at]=(uint8_t)lrint(clamp01(color.r*a+frame->rgba[at]/255.0*inv)*255);
    frame->rgba[at+1]=(uint8_t)lrint(clamp01(color.g*a+frame->rgba[at+1]/255.0*inv)*255);
    frame->rgba[at+2]=(uint8_t)lrint(clamp01(color.b*a+frame->rgba[at+2]/255.0*inv)*255);
    frame->rgba[at+3]=255;
}

static SrColor shade(const SrScene *scene, const SrObject3D *object,
                     Vec3 point, Vec3 normal, double time) {
    SrMaterial fallback={.base_color={0.7,0.7,0.7,1},.roughness=.5};
    const SrMaterial *material=object->material?object->material:&fallback;
    double r=material->emissive.r,g=material->emissive.g,b=material->emissive.b;
    Vec3 view={0,0,1};
    for(size_t i=0;i<scene->light_count;++i){const SrLight *light=&scene->lights[i];
        double intensity=fmax(0.0,sr_anim_eval(&light->intensity,time));
        Vec3 direction={0,0,1};double attenuation=1.0;
        if(light->type==SR_LIGHT_AMBIENT){r+=light->color.r*intensity;g+=light->color.g*intensity;b+=light->color.b*intensity;continue;}
        if(light->type==SR_LIGHT_DIRECTIONAL){double yaw=sr_anim_eval(&light->yaw,time)*SR_PI/180.0;
            double pitch=sr_anim_eval(&light->pitch,time)*SR_PI/180.0;
            direction=normalize((Vec3){-sin(yaw)*cos(pitch),sin(pitch),cos(yaw)*cos(pitch)});
        }else{Vec3 delta={sr_anim_eval(&light->x,time)-point.x,sr_anim_eval(&light->y,time)-point.y,sr_anim_eval(&light->z,time)-point.z};
            double distance=sqrt(dot(delta,delta));direction=normalize(delta);
            attenuation=pow(clamp01(1.0-distance/fmax(light->range,1e-9)),fmax(light->falloff,0.0));
            if(light->type==SR_LIGHT_SPOT){double yaw=sr_anim_eval(&light->yaw,time)*SR_PI/180.0,pitch=sr_anim_eval(&light->pitch,time)*SR_PI/180.0;
                Vec3 aim=normalize((Vec3){sin(yaw)*cos(pitch),-sin(pitch),-cos(yaw)*cos(pitch)});
                Vec3 from_light={-direction.x,-direction.y,-direction.z};double cone=cos(light->spot_angle*SR_PI/360.0);
                attenuation*=clamp01((dot(from_light,aim)-cone)/fmax(1.0-cone,1e-9));}}
        double diffuse=fmax(0.0,dot(normal,direction));
        Vec3 halfv=normalize((Vec3){direction.x+view.x,direction.y+view.y,direction.z+view.z});
        double exponent=2.0+126.0*(1.0-material->roughness);
        double spec=pow(fmax(0.0,dot(normal,halfv)),exponent)*(0.04+0.96*material->metallic);
        double energy=intensity*attenuation;
        r+=light->color.r*energy*(material->base_color.r*diffuse+spec);
        g+=light->color.g*energy*(material->base_color.g*diffuse+spec);
        b+=light->color.b*energy*(material->base_color.b*diffuse+spec);
    }
    return (SrColor){clamp01(r),clamp01(g),clamp01(b),material->base_color.a};
}

static void shadow(const SrScene *scene,const SrObject3D *object,double time,SrFrame *frame){
    bool enabled=false;for(size_t i=0;i<scene->light_count;++i)if(scene->lights[i].cast_shadow){enabled=true;break;}
    if(!enabled||!object->cast_shadow)return;
    double x,y,projection_scale;if(!project(scene,object,time,&x,&y,&projection_scale))return;x+=25*projection_scale;y+=30*projection_scale;
    double radius=object->radius*fabs(sr_anim_eval(&object->transform.scale_x,time))*projection_scale;
    for(int py=(int)(y-radius*.3);py<=(int)(y+radius*.3);++py)for(int px=(int)(x-radius);px<=(int)(x+radius);++px){
        double dx=(px-x)/fmax(radius,1.0),dy=(py-y)/fmax(radius*.3,1.0);
        if(dx*dx+dy*dy<=1.0)over(frame,px,py,(SrColor){0,0,0,1},.3);}
}

SrStatus sr_lighting_render(SrScene *scene,double time,SrFrame *frame,SrDiagnostics *diag){
    (void)diag;
    for(size_t i=0;i<scene->object3d_count;++i)shadow(scene,&scene->objects3d[i],time,frame);
    for(size_t i=0;i<scene->object3d_count;++i){SrObject3D *object=&scene->objects3d[i];
        double world_x=sr_anim_eval(&object->transform.x,time),world_y=sr_anim_eval(&object->transform.y,time),cz=sr_anim_eval(&object->transform.z,time);
        double cx,cy,projection_scale;if(!project(scene,object,time,&cx,&cy,&projection_scale))continue;
        double sx=fabs(sr_anim_eval(&object->transform.scale_x,time)),sy=fabs(sr_anim_eval(&object->transform.scale_y,time));
        double rx=fmax(1.0,object->radius*sx*projection_scale),ry=fmax(1.0,object->radius*sy*projection_scale);
        for(int y=(int)floor(cy-ry);y<=(int)ceil(cy+ry);++y)for(int x=(int)floor(cx-rx);x<=(int)ceil(cx+rx);++x){
            double nx=(x+.5-cx)/rx,ny=(y+.5-cy)/ry;
            double angle=-sr_anim_eval(&object->transform.rotation,time)*SR_PI/180.0;
            double rotated_x=nx*cos(angle)-ny*sin(angle);
            double rotated_y=nx*sin(angle)+ny*cos(angle);
            nx=rotated_x;ny=rotated_y;
            bool inside=fabs(nx)<=1&&fabs(ny)<=1;
            Vec3 normal={0,0,1};
            if(object->primitive==SR_OBJECT_SPHERE){inside=nx*nx+ny*ny<=1;normal=(Vec3){nx,-ny,sqrt(fmax(0.0,1-nx*nx-ny*ny))};}
            if (!inside) continue;
            Vec3 point = {world_x + normal.x * object->radius,
                          world_y - normal.y * object->radius,
                          cz + normal.z * object->radius};
            SrColor color = shade(scene, object, point, normal, time);
            over(frame, x, y, color, 1);
        }
    }
    return SR_OK;
}
