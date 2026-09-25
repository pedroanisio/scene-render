#include "scene_render/lighting.h"
#include "scene_render/color.h"

#include <math.h>
#include <stdlib.h>

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

static double object_bound(const SrObject3D *object,double time){
    double scale=fmax(fabs(sr_anim_eval(&object->transform.scale_x,time)),
        fmax(fabs(sr_anim_eval(&object->transform.scale_y,time)),
             fabs(sr_anim_eval(&object->transform.scale_z,time))));
    return object->radius*scale;
}

static bool shadowed(const SrScene *scene,const SrObject3D *receiver,
                     Vec3 point,Vec3 direction,double maximum,double time){
    if(!receiver->receive_shadow)return false;
    for(size_t i=0;i<scene->object3d_count;++i){const SrObject3D *caster=&scene->objects3d[i];
        if(caster==receiver||!caster->cast_shadow)continue;
        Vec3 center={sr_anim_eval(&caster->transform.x,time),
                     sr_anim_eval(&caster->transform.y,time),
                     sr_anim_eval(&caster->transform.z,time)};
        Vec3 delta={center.x-point.x,center.y-point.y,center.z-point.z};
        double along=dot(delta,direction);
        if(along<=1e-5||along>=maximum)continue;
        double distance2=dot(delta,delta)-along*along;
        double radius=object_bound(caster,time);
        if(distance2<radius*radius)return true;
    }
    return false;
}

static double view_depth(const SrScene *scene,Vec3 point,double time){
    const SrCamera *camera=scene_camera(scene);if(!camera)return-point.z;
    point.x-=sr_anim_eval(&camera->x,time);point.y-=sr_anim_eval(&camera->y,time);
    point.z-=sr_anim_eval(&camera->z,time);
    point=rotate_y(point,-sr_anim_eval(&camera->yaw,time)*SR_PI/180.0);
    point=rotate_x(point,-sr_anim_eval(&camera->pitch,time)*SR_PI/180.0);
    point=rotate_z(point,-sr_anim_eval(&camera->roll,time)*SR_PI/180.0);
    return point.z;
}

static void over(const SrProject *project,SrFrame *frame,int x,int y,SrColor color,double alpha);
static SrColor shade(const SrScene *scene,const SrObject3D *object,
                     Vec3 point,Vec3 normal,double time);

typedef struct { double x,y,depth; Vec3 world,normal; } MeshVertex;

static Vec3 object_rotate(const SrObject3D *object,Vec3 value,double time){
    value=rotate_x(value,sr_anim_eval(&object->transform.rotation_x,time)*SR_PI/180.0);
    value=rotate_y(value,sr_anim_eval(&object->transform.rotation_y,time)*SR_PI/180.0);
    return rotate_z(value,sr_anim_eval(&object->transform.rotation,time)*SR_PI/180.0);
}

static bool project_mesh_vertex(const SrScene *scene,const SrObject3D *object,
                                const SrMeshTriangle *triangle,size_t corner,
                                double time,MeshVertex *result){
    double radius=object->radius;
    Vec3 local={triangle->position[corner][0]*radius*sr_anim_eval(&object->transform.scale_x,time),
                triangle->position[corner][1]*radius*sr_anim_eval(&object->transform.scale_y,time),
                triangle->position[corner][2]*radius*sr_anim_eval(&object->transform.scale_z,time)};
    local=object_rotate(object,local,time);
    result->world=(Vec3){local.x+sr_anim_eval(&object->transform.x,time),
                         local.y+sr_anim_eval(&object->transform.y,time),
                         local.z+sr_anim_eval(&object->transform.z,time)};
    result->normal=normalize(object_rotate(object,(Vec3){triangle->normal[corner][0],
        triangle->normal[corner][1],triangle->normal[corner][2]},time));
    const SrCamera *camera=scene_camera(scene);
    if(!camera){result->x=result->world.x;result->y=result->world.y;
        result->depth=-result->world.z;return true;}
    Vec3 point={result->world.x-sr_anim_eval(&camera->x,time),
                result->world.y-sr_anim_eval(&camera->y,time),
                result->world.z-sr_anim_eval(&camera->z,time)};
    point=rotate_y(point,-sr_anim_eval(&camera->yaw,time)*SR_PI/180.0);
    point=rotate_x(point,-sr_anim_eval(&camera->pitch,time)*SR_PI/180.0);
    point=rotate_z(point,-sr_anim_eval(&camera->roll,time)*SR_PI/180.0);
    if(point.z<camera->near_plane||point.z>camera->far_plane)return false;
    result->depth=point.z;
    if(camera->orthographic){result->x=scene->project.width*.5+point.x;
        result->y=scene->project.height*.5-point.y;return true;}
    double focal=scene->project.height*.5/tan(sr_anim_eval(&camera->fov,time)*SR_PI/360.0);
    result->x=scene->project.width*.5+point.x*focal/point.z;
    result->y=scene->project.height*.5-point.y*focal/point.z;return true;
}

static double edge(double ax,double ay,double bx,double by,double px,double py){
    return(px-ax)*(by-ay)-(py-ay)*(bx-ax);
}

static void render_mesh(const SrScene *scene,const SrObject3D *object,double time,
                        SrFrame *frame,double *depth){
    if(!object->mesh_asset||!object->mesh_asset->mesh)return;
    const SrMesh *mesh=object->mesh_asset->mesh;
    for(size_t index=0;index<mesh->triangle_count;++index){MeshVertex v[3];
        if(!project_mesh_vertex(scene,object,&mesh->triangles[index],0,time,&v[0])||
           !project_mesh_vertex(scene,object,&mesh->triangles[index],1,time,&v[1])||
           !project_mesh_vertex(scene,object,&mesh->triangles[index],2,time,&v[2]))continue;
        double area=edge(v[0].x,v[0].y,v[1].x,v[1].y,v[2].x,v[2].y);
        if(fabs(area)<1e-12)continue;
        int min_x=(int)fmax(0.0,floor(fmin(v[0].x,fmin(v[1].x,v[2].x))));
        int max_x=(int)fmin(frame->width-1.0,ceil(fmax(v[0].x,fmax(v[1].x,v[2].x))));
        int min_y=(int)fmax(0.0,floor(fmin(v[0].y,fmin(v[1].y,v[2].y))));
        int max_y=(int)fmin(frame->height-1.0,ceil(fmax(v[0].y,fmax(v[1].y,v[2].y))));
        for(int y=min_y;y<=max_y;++y)for(int x=min_x;x<=max_x;++x){
            double a=edge(v[1].x,v[1].y,v[2].x,v[2].y,x+.5,y+.5)/area;
            double b=edge(v[2].x,v[2].y,v[0].x,v[0].y,x+.5,y+.5)/area;
            double c=1.0-a-b;if(a<0||b<0||c<0)continue;double z=a*v[0].depth+b*v[1].depth+c*v[2].depth;
            size_t at=(size_t)y*frame->width+(size_t)x;if(z>=depth[at])continue;depth[at]=z;
            Vec3 point={a*v[0].world.x+b*v[1].world.x+c*v[2].world.x,
                        a*v[0].world.y+b*v[1].world.y+c*v[2].world.y,
                        a*v[0].world.z+b*v[1].world.z+c*v[2].world.z};
            Vec3 normal=normalize((Vec3){a*v[0].normal.x+b*v[1].normal.x+c*v[2].normal.x,
                a*v[0].normal.y+b*v[1].normal.y+c*v[2].normal.y,
                a*v[0].normal.z+b*v[1].normal.z+c*v[2].normal.z});
            over(&scene->project,frame,x,y,shade(scene,object,point,normal,time),1);}
    }
}

/* Premultiplied source-over of a straight working-space color. */
static void over(const SrProject *project, SrFrame *frame, int x, int y,
                 SrColor color, double alpha) {
    if (x < 0 || y < 0 || x >= (int)frame->width || y >= (int)frame->height) return;
    color.a = clamp01(alpha * color.a);
    float source[4];
    sr_color_to_blend(project, color, source);
    sr_blend_px(SR_BLEND_NORMAL, &frame->px[((size_t)y*frame->width+(size_t)x)*4],
                source);
}

static SrColor shade(const SrScene *scene, const SrObject3D *object,
                     Vec3 point, Vec3 normal, double time) {
    SrMaterial fallback={.base_color={0.7,0.7,0.7,1},.roughness=.5};
    const SrMaterial *material=object->material?object->material:&fallback;
    double r=material->emissive.r,g=material->emissive.g,b=material->emissive.b;
    Vec3 view={0,0,1};
    for(size_t i=0;i<scene->light_count;++i){const SrLight *light=&scene->lights[i];
        double intensity=fmax(0.0,sr_anim_eval(&light->intensity,time));
        Vec3 direction={0,0,1};double attenuation=1.0,maximum=INFINITY;
        if(light->type==SR_LIGHT_AMBIENT){r+=light->color.r*intensity;g+=light->color.g*intensity;b+=light->color.b*intensity;continue;}
        if(light->type==SR_LIGHT_DIRECTIONAL){double yaw=sr_anim_eval(&light->yaw,time)*SR_PI/180.0;
            double pitch=sr_anim_eval(&light->pitch,time)*SR_PI/180.0;
            direction=normalize((Vec3){-sin(yaw)*cos(pitch),sin(pitch),cos(yaw)*cos(pitch)});
        }else{Vec3 delta={sr_anim_eval(&light->x,time)-point.x,sr_anim_eval(&light->y,time)-point.y,sr_anim_eval(&light->z,time)-point.z};
            double distance=sqrt(dot(delta,delta));maximum=distance;direction=normalize(delta);
            attenuation=pow(clamp01(1.0-distance/fmax(light->range,1e-9)),fmax(light->falloff,0.0));
            if(light->type==SR_LIGHT_SPOT){double yaw=sr_anim_eval(&light->yaw,time)*SR_PI/180.0,pitch=sr_anim_eval(&light->pitch,time)*SR_PI/180.0;
                Vec3 aim=normalize((Vec3){sin(yaw)*cos(pitch),-sin(pitch),-cos(yaw)*cos(pitch)});
                Vec3 from_light={-direction.x,-direction.y,-direction.z};double cone=cos(light->spot_angle*SR_PI/360.0);
                attenuation*=clamp01((dot(from_light,aim)-cone)/fmax(1.0-cone,1e-9));}}
        if(light->cast_shadow&&shadowed(scene,object,point,direction,maximum,time))
            attenuation*=0.2;
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
        if(dx*dx+dy*dy<=1.0)over(&scene->project,frame,px,py,(SrColor){0,0,0,1},.3);}
}

SrStatus sr_lighting_render(SrScene *scene,double time,SrFrame *frame,SrDiagnostics *diag){
    const SrCamera *camera=scene_camera(scene);
    if(camera&&!camera->orthographic){double fov=sr_anim_eval(&camera->fov,time);
        if(fov<=1.0||fov>=179.0){sr_diag_error(diag,camera->source_line,"camera","fov",
            "animated field of view must remain between 1 and 179 degrees");return SR_ERR_RENDER;}}
    size_t pixels=(size_t)frame->width*frame->height;double *depth=NULL;
    if(scene->object3d_count){depth=sr_alloc(pixels*sizeof(*depth));
        if(!depth){sr_diag_error(diag,0,NULL,NULL,"cannot allocate 3D depth buffer");return SR_ERR_MEMORY;}
        for(size_t i=0;i<pixels;++i)depth[i]=INFINITY;}
    for(size_t i=0;i<scene->object3d_count;++i)shadow(scene,&scene->objects3d[i],time,frame);
    for(size_t i=0;i<scene->object3d_count;++i){SrObject3D *object=&scene->objects3d[i];
        if(object->primitive==SR_OBJECT_MESH){render_mesh(scene,object,time,frame,depth);continue;}
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
            if (!inside||x<0||y<0||x>=(int)frame->width||y>=(int)frame->height)continue;
            Vec3 point = {world_x + normal.x * object->radius,
                          world_y - normal.y * object->radius,
                          cz + normal.z * object->radius};
            size_t at=(size_t)y*frame->width+(size_t)x;
            double object_depth=view_depth(scene,point,time);
            if(object_depth>=depth[at])continue;
            depth[at]=object_depth;
            SrColor color = shade(scene, object, point, normal, time);
            over(&scene->project, frame, x, y, color, 1);
        }
    }
    free(depth);return SR_OK;
}
