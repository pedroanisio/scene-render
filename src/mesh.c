#define _POSIX_C_SOURCE 200809L
#include "scene_render/mesh.h"

#include <ctype.h>
#include <errno.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct { double x,y,z; } Vec3;
typedef struct { Vec3 *items; size_t count,capacity; } VecList;
typedef struct { long vertex,normal; bool has_normal; } FaceIndex;

static bool vec_add(VecList *list,Vec3 value){if(list->count==list->capacity){
    size_t capacity=list->capacity?list->capacity*2:64;Vec3 *items=sr_realloc(list->items,capacity*sizeof(*items));
    if(!items)return false;
    list->items=items;list->capacity=capacity;}
    list->items[list->count++]=value;return true;}

static bool triangle_add(SrMesh *mesh,SrMeshTriangle triangle){
    if(mesh->triangle_count==mesh->triangle_capacity){size_t capacity=mesh->triangle_capacity?mesh->triangle_capacity*2:128;
        SrMeshTriangle *items=sr_realloc(mesh->triangles,capacity*sizeof(*items));
        if(!items)return false;
        mesh->triangles=items;mesh->triangle_capacity=capacity;}
    mesh->triangles[mesh->triangle_count++]=triangle;return true;
}

static Vec3 subtract(Vec3 a,Vec3 b){return(Vec3){a.x-b.x,a.y-b.y,a.z-b.z};}
static Vec3 normalize(Vec3 v){double length=sqrt(v.x*v.x+v.y*v.y+v.z*v.z);
    return length>1e-15?(Vec3){v.x/length,v.y/length,v.z/length}:(Vec3){0,0,1};}
static Vec3 cross(Vec3 a,Vec3 b){return(Vec3){a.y*b.z-a.z*b.y,a.z*b.x-a.x*b.z,a.x*b.y-a.y*b.x};}

static bool parse_index(const char *token,FaceIndex *index){char *tail=NULL;errno=0;
    index->vertex=strtol(token,&tail,10);if(errno||tail==token||index->vertex==0)return false;
    index->has_normal=false;index->normal=0;if(*tail=='/'){++tail;if(*tail&&*tail!='/'){
        strtol(tail,&tail,10);if(errno)return false;}if(*tail=='/'){++tail;if(*tail){char *normal_tail=NULL;
            index->normal=strtol(tail,&normal_tail,10);if(errno||normal_tail==tail||*normal_tail||index->normal==0)return false;
            index->has_normal=true;tail=normal_tail;}}}return *tail=='\0';}

static bool resolve(long value,size_t count,size_t *index){long result=value>0?value-1:(long)count+value;
    if(result<0||(size_t)result>=count)return false;
    *index=(size_t)result;return true;}

static bool add_face(SrMesh *mesh,const VecList *vertices,const VecList *normals,
                     FaceIndex *face,size_t count){
    for(size_t corner=1;corner+1<count;++corner){size_t selected[3]={0,corner,corner+1};
        SrMeshTriangle triangle={0};bool supplied=true;
        for(size_t i=0;i<3;++i){FaceIndex f=face[selected[i]];size_t vertex;
            if(!resolve(f.vertex,vertices->count,&vertex))return false;
            Vec3 p=vertices->items[vertex];
            triangle.position[i][0]=p.x;triangle.position[i][1]=p.y;triangle.position[i][2]=p.z;
            size_t normal;if(!f.has_normal||!resolve(f.normal,normals->count,&normal)){supplied=false;continue;}
            Vec3 n=normalize(normals->items[normal]);triangle.normal[i][0]=n.x;triangle.normal[i][1]=n.y;triangle.normal[i][2]=n.z;}
        if(!supplied){Vec3 a={triangle.position[0][0],triangle.position[0][1],triangle.position[0][2]};
            Vec3 b={triangle.position[1][0],triangle.position[1][1],triangle.position[1][2]};
            Vec3 c={triangle.position[2][0],triangle.position[2][1],triangle.position[2][2]};
            Vec3 n=normalize(cross(subtract(b,a),subtract(c,a)));for(size_t i=0;i<3;++i){
                triangle.normal[i][0]=n.x;triangle.normal[i][1]=n.y;triangle.normal[i][2]=n.z;}}
        if(!triangle_add(mesh,triangle))return false;}return true;
}

static SrStatus obj_error(const char *path,size_t line,size_t source_line,
                          SrDiagnostics *diag,const char *reason){
    sr_diag_error(diag,source_line,"mesh","src","OBJ %s:%zu: %s",path,line,reason);
    return SR_ERR_ASSET;
}

SrStatus sr_mesh_load_obj(const char *path,SrMesh **result,size_t source_line,
                          SrDiagnostics *diag){FILE *file=fopen(path,"rb");
    if(!file){sr_diag_error(diag,source_line,"mesh","src","cannot open '%s': %s",path,strerror(errno));return SR_ERR_ASSET;}
    VecList vertices={0},normals={0};SrMesh *mesh=sr_alloc(sizeof(*mesh));char *line=NULL;size_t capacity=0,line_number=0;SrStatus status=SR_OK;
    if(!mesh){fclose(file);return SR_ERR_MEMORY;}
    while(status==SR_OK&&getline(&line,&capacity,file)>=0){++line_number;char *p=line;while(isspace((unsigned char)*p))++p;
        if(*p=='\0'||*p=='#')continue;
        if(p[0]=='v'&&isspace((unsigned char)p[1])){Vec3 value;char extra;
            if(sscanf(p+1," %lf %lf %lf %c",&value.x,&value.y,&value.z,&extra)!=3||!vec_add(&vertices,value))
                status=obj_error(path,line_number,source_line,diag,"invalid vertex");}
        else if(p[0]=='v'&&p[1]=='n'&&isspace((unsigned char)p[2])){Vec3 value;char extra;
            if(sscanf(p+2," %lf %lf %lf %c",&value.x,&value.y,&value.z,&extra)!=3||!vec_add(&normals,value))
                status=obj_error(path,line_number,source_line,diag,"invalid normal");}
        else if(p[0]=='f'&&isspace((unsigned char)p[1])){FaceIndex face[256];size_t count=0;char *save=NULL;
            for(char *token=strtok_r(p+1," \t\r\n",&save);token;token=strtok_r(NULL," \t\r\n",&save)){
                if(count==256||!parse_index(token,&face[count++])){status=obj_error(path,line_number,source_line,diag,"invalid face index");break;}}
            if(status==SR_OK&&(count<3||!add_face(mesh,&vertices,&normals,face,count)))
                status=obj_error(path,line_number,source_line,diag,"face references an invalid vertex or normal");}
    }
    if(status==SR_OK&&ferror(file))status=obj_error(path,line_number,source_line,diag,"read error");
    if(status==SR_OK&&mesh->triangle_count==0)status=obj_error(path,line_number,source_line,diag,"no triangle faces found");
    free(line);free(vertices.items);free(normals.items);fclose(file);
    if(status!=SR_OK){free(mesh->triangles);free(mesh);return status;}*result=mesh;return SR_OK;
}
