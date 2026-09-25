#include "scene_render/vector_path.h"

#include <ctype.h>
#include <math.h>
#include <stdlib.h>

typedef struct { double x, y; } Point;
typedef struct { Point *points; size_t count, capacity; } Contour;
typedef struct { Contour *items; size_t count, capacity; } Path;

static void path_free(Path *path) {
    for (size_t i=0;i<path->count;++i) free(path->items[i].points);
    free(path->items);
}

static Contour *add_contour(Path *path) {
    if (path->count == path->capacity) {
        size_t capacity=path->capacity?path->capacity*2:4;
        Contour *items=sr_realloc(path->items,capacity*sizeof(*items));
        if(!items)return NULL;
        for(size_t i=path->capacity;i<capacity;++i)items[i]=(Contour){0};
        path->items=items;path->capacity=capacity;
    }
    return &path->items[path->count++];
}

static bool add_point(Contour *contour, Point point) {
    if(contour->count==contour->capacity){size_t capacity=contour->capacity?contour->capacity*2:16;
        Point *points=sr_realloc(contour->points,capacity*sizeof(*points));
        if(!points)return false;
        contour->points=points;contour->capacity=capacity;}
    contour->points[contour->count++]=point;return true;
}

static void separators(const char **cursor) {
    while (isspace((unsigned char)**cursor) || **cursor==',') ++*cursor;
}

static bool number(const char **cursor, double *value) {
    separators(cursor);char *tail=NULL;*value=strtod(*cursor,&tail);
    if(tail==*cursor||!isfinite(*value))return false;
    *cursor=tail;return true;
}

static bool pair(const char **cursor,double *x,double *y){return number(cursor,x)&&number(cursor,y);}

static bool flatten_cubic(Contour *contour,Point start,Point a,Point b,Point end){
    for(int i=1;i<=16;++i){double t=i/16.0,u=1.0-t;
        Point p={u*u*u*start.x+3*u*u*t*a.x+3*u*t*t*b.x+t*t*t*end.x,
                 u*u*u*start.y+3*u*u*t*a.y+3*u*t*t*b.y+t*t*t*end.y};
        if(!add_point(contour,p))return false;}return true;
}

static bool flatten_quadratic(Contour *contour,Point start,Point control,Point end){
    for(int i=1;i<=12;++i){double t=i/12.0,u=1.0-t;
        Point p={u*u*start.x+2*u*t*control.x+t*t*end.x,
                 u*u*start.y+2*u*t*control.y+t*t*end.y};
        if(!add_point(contour,p))return false;}return true;
}

static bool parse_path(const char *text,Path *path) {
    const char *cursor=text;char command=0;Contour *contour=NULL;Point current={0},first={0};
    while(1){separators(&cursor);if(!*cursor)break;
        if(isalpha((unsigned char)*cursor))command=*cursor++;
        else if(!command)return false;
        bool relative=islower((unsigned char)command)!=0;char op=(char)toupper((unsigned char)command);
        if(op=='Z'){if(contour&&contour->count&&
            (current.x!=first.x||current.y!=first.y)&&!add_point(contour,first))return false;
            current=first;command=0;continue;}
        if(op=='M'||op=='L'){double x,y;if(!pair(&cursor,&x,&y))return false;
            if(relative){x+=current.x;y+=current.y;}current=(Point){x,y};
            if(op=='M'){contour=add_contour(path);if(!contour)return false;first=current;
                command=relative?'l':'L';}
            if(!contour||!add_point(contour,current))return false;
            continue;}
        if(!contour||!contour->count)return false;
        if(op=='H'){double x;if(!number(&cursor,&x))return false;if(relative)x+=current.x;
            current.x=x;if(!add_point(contour,current))return false;continue;}
        if(op=='V'){double y;if(!number(&cursor,&y))return false;if(relative)y+=current.y;
            current.y=y;if(!add_point(contour,current))return false;continue;}
        if(op=='C'){double ax,ay,bx,by,x,y;if(!pair(&cursor,&ax,&ay)||!pair(&cursor,&bx,&by)||!pair(&cursor,&x,&y))return false;
            if(relative){ax+=current.x;ay+=current.y;bx+=current.x;by+=current.y;x+=current.x;y+=current.y;}
            Point end={x,y};if(!flatten_cubic(contour,current,(Point){ax,ay},(Point){bx,by},end))return false;current=end;continue;}
        if(op=='Q'){double cx,cy,x,y;if(!pair(&cursor,&cx,&cy)||!pair(&cursor,&x,&y))return false;
            if(relative){cx+=current.x;cy+=current.y;x+=current.x;y+=current.y;}
            Point end={x,y};if(!flatten_quadratic(contour,current,(Point){cx,cy},end))return false;current=end;continue;}
        return false;
    }
    if(!path->count)return false;
    for(size_t i=0;i<path->count;++i)if(path->items[i].count<3)return false;
    return true;
}

bool sr_vector_path_valid(const char *text) {
    Path path={0};
    bool valid=text&&parse_path(text,&path);
    path_free(&path);
    return valid;
}

static bool contains(const Path *path,double x,double y){bool inside=false;
    for(size_t c=0;c<path->count;++c){const Contour *contour=&path->items[c];
        for(size_t i=0,j=contour->count-1;i<contour->count;j=i++){
            Point a=contour->points[i],b=contour->points[j];
            if(((a.y>y)!=(b.y>y))&&x<(b.x-a.x)*(y-a.y)/(b.y-a.y)+a.x)inside=!inside;}}
    return inside;
}

static uint8_t channel(double value){return(uint8_t)lrint(fmax(0.0,fmin(1.0,value))*255.0);}

SrStatus sr_vector_path_render(const char *text,SrImage *image,SrColor color,
                               size_t source_line,SrDiagnostics *diag){
    Path path={0};if(!text||!parse_path(text,&path)){path_free(&path);
        sr_diag_error(diag,source_line,"vector","path",
                      "invalid path; supported commands are M/L/H/V/C/Q/Z");
        return SR_ERR_ASSET;}
    const double offset[2]={.25,.75};
    for(uint32_t y=0;y<image->height;++y)for(uint32_t x=0;x<image->width;++x){unsigned hits=0;
        for(size_t sy=0;sy<2;++sy)for(size_t sx=0;sx<2;++sx)
            if(contains(&path,x+offset[sx],y+offset[sy]))++hits;
        if(!hits)continue;
        size_t at=((size_t)y*image->width+x)*4;
        image->rgba[at]=channel(color.r);image->rgba[at+1]=channel(color.g);
        image->rgba[at+2]=channel(color.b);image->rgba[at+3]=channel(color.a*hits/4.0);}
    path_free(&path);return SR_OK;
}
