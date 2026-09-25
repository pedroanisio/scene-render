#define _POSIX_C_SOURCE 200809L
#include "scene_render/physics.h"

#include <errno.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    SrNode *node;
    double x, y, angle, vx, vy, angular_velocity;
    double inverse_mass;
} BodyState;

typedef struct {
    char magic[8];
    uint32_t version;
    uint32_t body_count;
    uint64_t sample_count;
    uint64_t signature;
    double fixed_step;
} CacheHeader;

static void collect(SrNode *node, BodyState **states, size_t *count,
                    size_t *capacity) {
    if (node->body.type != SR_BODY_NONE) {
        if (*count == *capacity) {
            size_t next = *capacity ? *capacity * 2 : 8;
            BodyState *grown = sr_realloc(*states, next * sizeof(*grown));
            if (!grown) return;
            *states = grown; *capacity = next;
        }
        SrRigidBody *body = &node->body;
        (*states)[*count] = (BodyState){node, node->transform.x.base,
            node->transform.y.base, node->transform.rotation.base,
            body->velocity_x, body->velocity_y, body->angular_velocity,
            body->type == SR_BODY_DYNAMIC && body->mass > 0.0 ?
                1.0 / body->mass : 0.0};
        ++*count;
    }
    for (size_t i = 0; i < node->child_count; ++i)
        collect(node->children[i], states, count, capacity);
}

static uint64_t hash_bytes(uint64_t hash, const void *data, size_t length) {
    const uint8_t *bytes = data;
    for (size_t i = 0; i < length; ++i) {
        hash ^= bytes[i]; hash *= UINT64_C(1099511628211);
    }
    return hash;
}

static uint64_t signature(const SrScene *scene, const BodyState *states,
                          size_t count) {
    uint64_t hash = UINT64_C(1469598103934665603);
    hash = hash_bytes(hash, SR_VERSION, strlen(SR_VERSION));
    hash = hash_bytes(hash, &scene->project.seed, sizeof(scene->project.seed));
    hash = hash_bytes(hash, &scene->project.duration, sizeof(double));
    hash = hash_bytes(hash, &scene->physics.fixed_step, sizeof(double));
    hash = hash_bytes(hash, &scene->physics.gravity_x, sizeof(double));
    hash = hash_bytes(hash, &scene->physics.gravity_y, sizeof(double));
    for (size_t i = 0; i < count; ++i) {
        hash = hash_bytes(hash, states[i].node->id, strlen(states[i].node->id));
        hash = hash_bytes(hash, &states[i].node->body,
                          sizeof(states[i].node->body));
        hash = hash_bytes(hash, &states[i].x, sizeof(double) * 3);
    }
    return hash;
}

static char *cache_path(const SrScene *scene) {
    return scene->physics.cache_path ?
        sr_path_join(scene->base_dir, scene->physics.cache_path) : NULL;
}

static bool load_cache(SrScene *scene, BodyState *states, size_t count,
                       uint64_t samples, uint64_t expected) {
    char *path = cache_path(scene);
    if (!path) return false;
    FILE *file = fopen(path, "rb"); free(path);
    if (!file) return false;
    CacheHeader header;
    bool ok = fread(&header, sizeof(header), 1, file) == 1 &&
        memcmp(header.magic, "SRPHYS1", 8) == 0 && header.version == 1 &&
        header.body_count == count && header.sample_count == samples &&
        header.signature == expected && header.fixed_step == scene->physics.fixed_step;
    for (size_t i = 0; ok && i < count; ++i) {
        states[i].node->physics_samples = sr_alloc(samples * sizeof(SrPhysicsSample));
        if (!states[i].node->physics_samples) { ok = false; break; }
        states[i].node->physics_sample_count = samples;
        for (uint64_t s = 0; s < samples; ++s) {
            double pose[3];
            if (fread(pose, sizeof(pose), 1, file) != 1) { ok = false; break; }
            states[i].node->physics_samples[s].enabled = true;
            states[i].node->physics_samples[s].x.base = pose[0];
            states[i].node->physics_samples[s].y.base = pose[1];
            states[i].node->physics_samples[s].rotation.base = pose[2];
        }
    }
    fclose(file);
    if (!ok) for (size_t i = 0; i < count; ++i) {
        free(states[i].node->physics_samples);
        states[i].node->physics_samples = NULL;
        states[i].node->physics_sample_count = 0;
    }
    return ok;
}

static void save_cache(const SrScene *scene, BodyState *states, size_t count,
                       uint64_t samples, uint64_t hash) {
    char *path = cache_path(scene); if (!path) return;
    FILE *file = fopen(path, "wb"); free(path); if (!file) return;
    CacheHeader header = {{'S','R','P','H','Y','S','1','\0'}, 1, (uint32_t)count,
                          samples, hash, scene->physics.fixed_step};
    bool ok = fwrite(&header, sizeof(header), 1, file) == 1;
    for (size_t i = 0; ok && i < count; ++i) for (uint64_t s = 0; s < samples; ++s) {
        SrPhysicsSample *sample = &states[i].node->physics_samples[s];
        double pose[3] = {sample->x.base, sample->y.base, sample->rotation.base};
        if (fwrite(pose, sizeof(pose), 1, file) != 1) { ok = false; break; }
    }
    fclose(file);
}

static void dimensions(const SrNode *node, double *width, double *height) {
    if (node->type == SR_NODE_MEDIA && node->asset) {
        *width = node->asset->width * fabs(node->transform.scale_x.base);
        *height = node->asset->height * fabs(node->transform.scale_y.base);
    } else {
        *width = node->shape_width * fabs(node->transform.scale_x.base);
        *height = node->shape_height * fabs(node->transform.scale_y.base);
    }
    if (*width <= 0.0) *width = node->body.radius > 0 ? node->body.radius * 2 : 1;
    if (*height <= 0.0) *height = node->body.radius > 0 ? node->body.radius * 2 : 1;
}

static BodyState *state_for(BodyState *states, size_t count, SrNode *node) {
    for (size_t i = 0; i < count; ++i) if (states[i].node == node) return &states[i];
    return NULL;
}

static void apply_constraints(SrScene *scene, BodyState *states, size_t count) {
    for (size_t i = 0; i < scene->physics.constraint_count; ++i) {
        SrConstraint *constraint = &scene->physics.constraints[i];
        BodyState *a = state_for(states, count, constraint->a);
        BodyState *b = state_for(states, count, constraint->b);
        if (!a || !b) continue;
        double dx = b->x - a->x, dy = b->y - a->y;
        double length = hypot(dx, dy); if (length < 1e-9) continue;
        double nx = dx / length, ny = dy / length;
        double relative = (b->vx-a->vx)*nx + (b->vy-a->vy)*ny;
        double force = (length-constraint->rest_length)*constraint->stiffness +
                       relative*constraint->damping;
        if (a->inverse_mass) { a->vx += force*nx*a->inverse_mass*scene->physics.fixed_step;
                               a->vy += force*ny*a->inverse_mass*scene->physics.fixed_step; }
        if (b->inverse_mass) { b->vx -= force*nx*b->inverse_mass*scene->physics.fixed_step;
                               b->vy -= force*ny*b->inverse_mass*scene->physics.fixed_step; }
    }
}

static void collide(BodyState *a, BodyState *b) {
    if (!a->inverse_mass && !b->inverse_mass) return;
    double aw, ah, bw, bh; dimensions(a->node,&aw,&ah); dimensions(b->node,&bw,&bh);
    double dx=b->x-a->x,dy=b->y-a->y,nx=0,ny=0,penetration=0;
    bool circles=a->node->body.collider==SR_COLLIDER_CIRCLE ||
                 b->node->body.collider==SR_COLLIDER_CIRCLE;
    if(circles){double ar=a->node->body.radius>0?a->node->body.radius:fmax(aw,ah)/2;
        double br=b->node->body.radius>0?b->node->body.radius:fmax(bw,bh)/2;
        double distance=hypot(dx,dy);if(distance>=ar+br)return;
        if(distance<1e-9){nx=1;ny=0;}else{nx=dx/distance;ny=dy/distance;}penetration=ar+br-distance;
    }else{double overlap_x=(aw+bw)/2-fabs(dx),overlap_y=(ah+bh)/2-fabs(dy);if(overlap_x<=0||overlap_y<=0)return;
        if(overlap_x<overlap_y){nx=dx<0?-1:1;penetration=overlap_x;}else{ny=dy<0?-1:1;penetration=overlap_y;}}
    double inv=a->inverse_mass+b->inverse_mass;if(inv<=0)return;
    if(a->inverse_mass){a->x-=nx*penetration*a->inverse_mass/inv;a->y-=ny*penetration*a->inverse_mass/inv;}
    if(b->inverse_mass){b->x+=nx*penetration*b->inverse_mass/inv;b->y+=ny*penetration*b->inverse_mass/inv;}
    double relative=(b->vx-a->vx)*nx+(b->vy-a->vy)*ny;if(relative>=0)return;
    double restitution=fmin(a->node->body.restitution,b->node->body.restitution);
    double impulse=-(1+restitution)*relative/inv;
    if(a->inverse_mass){a->vx-=impulse*nx*a->inverse_mass;a->vy-=impulse*ny*a->inverse_mass;}
    if(b->inverse_mass){b->vx+=impulse*nx*b->inverse_mass;b->vy+=impulse*ny*b->inverse_mass;}
    double tx=-ny,ty=nx,tangent=(b->vx-a->vx)*tx+(b->vy-a->vy)*ty;
    double friction=sqrt(a->node->body.friction*b->node->body.friction);
    double fi=fmax(-impulse*friction,fmin(impulse*friction,-tangent/inv));
    if(a->inverse_mass){a->vx-=fi*tx*a->inverse_mass;a->vy-=fi*ty*a->inverse_mass;}
    if(b->inverse_mass){b->vx+=fi*tx*b->inverse_mass;b->vy+=fi*ty*b->inverse_mass;}
}

static void simulate_step(SrScene *scene, BodyState *states, size_t count) {
    double dt=scene->physics.fixed_step;
    apply_constraints(scene,states,count);
    for(size_t i=0;i<count;++i){BodyState *state=&states[i];
        if (state->node->body.type == SR_BODY_KINEMATIC) {
            state->x += state->vx * dt;
            state->y += state->vy * dt;
            state->angle += state->angular_velocity * dt;
            continue;
        }
        if(!state->inverse_mass)continue;
        double ax=scene->physics.gravity_x,ay=scene->physics.gravity_y;
        for(size_t f=0;f<scene->physics.field_count;++f){SrForceField *field=&scene->physics.fields[f];
            if(field->radial){double dx=field->x-state->x,dy=field->y-state->y,d=hypot(dx,dy);if(d>1e-9){double scale=field->strength/pow(1+d,fmax(0.0,field->falloff));ax+=dx/d*scale;ay+=dy/d*scale;}}
            else{ax+=field->force_x;ay+=field->force_y;}}
        state->vx=(state->vx+ax*dt)*fmax(0.0,1-state->node->body.linear_damping*dt);
        state->vy=(state->vy+ay*dt)*fmax(0.0,1-state->node->body.linear_damping*dt);
        state->angular_velocity*=fmax(0.0,1-state->node->body.angular_damping*dt);
        state->x+=state->vx*dt;state->y+=state->vy*dt;state->angle+=state->angular_velocity*dt;}
    for(size_t iteration=0;iteration<3;++iteration)for(size_t i=0;i<count;++i)for(size_t j=i+1;j<count;++j)collide(&states[i],&states[j]);
}

SrStatus sr_physics_prepare(SrScene *scene,SrDiagnostics *diag){
    if (!scene->physics.enabled) return SR_OK;
    BodyState *states = NULL;
    size_t count = 0, capacity = 0;
    collect(scene->root, &states, &count, &capacity);
    if(!count){free(states);return SR_OK;}uint64_t samples=(uint64_t)ceil(scene->project.duration/scene->physics.fixed_step)+1;
    uint64_t hash=signature(scene,states,count);if(load_cache(scene,states,count,samples,hash)){sr_diag_info(diag,"loaded physics cache");free(states);return SR_OK;}
    for(size_t i=0;i<count;++i){states[i].node->physics_samples=sr_alloc(samples*sizeof(SrPhysicsSample));if(!states[i].node->physics_samples){free(states);return SR_ERR_MEMORY;}states[i].node->physics_sample_count=samples;}
    for(uint64_t s=0;s<samples;++s){for(size_t i=0;i<count;++i){SrPhysicsSample *sample=&states[i].node->physics_samples[s];sample->enabled=true;sample->x.base=states[i].x;sample->y.base=states[i].y;sample->rotation.base=states[i].angle;}if(s+1<samples)simulate_step(scene,states,count);}
    save_cache(scene,states,count,samples,hash);sr_diag_info(diag,"simulated %llu fixed physics steps",(unsigned long long)(samples-1));free(states);return SR_OK;
}

bool sr_physics_pose(const SrScene *scene,const SrNode *node,double time,double *x,double *y,double *rotation){
    if (!scene->physics.enabled || !node->physics_sample_count) return false;
    double exact = fmax(0.0, time) / scene->physics.fixed_step;
    uint64_t first=(uint64_t)floor(exact);if(first>=node->physics_sample_count)first=node->physics_sample_count-1;uint64_t second=first+1<node->physics_sample_count?first+1:first;double t=second==first?0:exact-first;
    const SrPhysicsSample *a=&node->physics_samples[first],*b=&node->physics_samples[second];*x=a->x.base+(b->x.base-a->x.base)*t;*y=a->y.base+(b->y.base-a->y.base)*t;*rotation=a->rotation.base+(b->rotation.base-a->rotation.base)*t;return true;
}
