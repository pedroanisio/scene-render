#include "xml_internal.h"
#include "scene_render/physics.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static bool decimal(ParseContext *ctx,const char *element,const XML_Char **attrs,
                    const char *name,double *target){const char *value=sr_xml_attr(attrs,name);
    if (!value) return true;
    if (!sr_parse_double(value,target)) {
        sr_xml_fail(ctx,element,name,"expected a finite decimal number");
        return false;
    }
    return true;
}

/* A soft body whose stable integration needs more than SR_SOFT_MAX_SUBSTEPS
 * substeps per fixed step is rejected. Validation assumes the anchor
 * spring of a rigid body (the conservative case) because <rigidBody> may
 * follow <softBody>. Checked at <softBody> against the step known then,
 * and again for every soft body when <physics> sets fixedStep. */
static bool soft_body_stable(ParseContext *ctx,const SrSoftBody *body,size_t line){
    double step=ctx->scene->physics.fixed_step;
    double needed=sr_soft_body_substeps(body,true,step);
    if(needed<=(double)SR_SOFT_MAX_SUBSTEPS)return true;
    if(!ctx->failed){
        sr_diag_error(ctx->diag,line,"softBody","stiffness/mass",
            "stiffness %g with mass %g over a %ux%u grid needs %.0f integration "
            "substeps per fixedStep %g (at most %d); lower stiffness, raise mass, "
            "or reduce fixedStep",body->stiffness,body->mass,body->rows,body->cols,
            needed,step,SR_SOFT_MAX_SUBSTEPS);
        ctx->failed=true;
        XML_StopParser(ctx->parser,XML_FALSE);
    }
    return false;
}

static bool soft_bodies_stable(ParseContext *ctx,const SrNode *node){
    if(node->soft_body.enabled&&!soft_body_stable(ctx,&node->soft_body,node->source_line))
        return false;
    for(size_t i=0;i<node->child_count;++i)
        if(!soft_bodies_stable(ctx,node->children[i]))return false;
    return true;
}

void sr_xml_start_physics(ParseContext *ctx,const XML_Char **attrs){
    const char *const allowed[]={"fixedStep","gravityX","gravityY","cache"};
    if(!sr_xml_attrs_allowed(ctx,"physics",attrs,allowed,4))return;
    SrPhysicsWorld *world=&ctx->scene->physics;world->enabled=true;
    if(!decimal(ctx,"physics",attrs,"fixedStep",&world->fixed_step)||
       !decimal(ctx,"physics",attrs,"gravityX",&world->gravity_x)||
       !decimal(ctx,"physics",attrs,"gravityY",&world->gravity_y))return;
    if(world->fixed_step<=0||world->fixed_step>1.0)
        SR_XML_FAIL_RETURN(ctx,"physics","fixedStep","expected a value in (0,1]");
    if(ctx->scene->root&&!soft_bodies_stable(ctx,ctx->scene->root))return;
    const char *cache=sr_xml_attr(attrs,"cache");if(cache){world->cache_path=sr_strdup(cache);if(!world->cache_path)SR_XML_FAIL_RETURN(ctx,"physics","cache","out of memory");}
    ctx->seen_physics=true;
}

void sr_xml_start_force_field(ParseContext *ctx,const XML_Char **attrs){
    const char *const allowed[]={"id","type","x","y","forceX","forceY","strength","falloff"};
    if(!sr_xml_attrs_allowed(ctx,"forceField",attrs,allowed,8))return;
    const char *id=sr_xml_required(ctx,"forceField",attrs,"id");const char *type=sr_xml_required(ctx,"forceField",attrs,"type");if(ctx->failed)return;
    if(!sr_id_valid(id))SR_XML_FAIL_RETURN(ctx,"forceField","id","invalid identifier");
    if(sr_scene_id_exists(ctx->scene,id))SR_XML_FAIL_RETURN(ctx,"forceField","id","id must be globally unique");
    SrPhysicsWorld *world=&ctx->scene->physics;if(world->field_count==world->field_capacity){size_t cap=world->field_capacity?world->field_capacity*2:4;
        SrForceField *items=sr_realloc(world->fields,cap*sizeof(*items));if(!items)SR_XML_FAIL_RETURN(ctx,"forceField",NULL,"out of memory");
        memset(items+world->field_capacity,0,(cap-world->field_capacity)*sizeof(*items));world->fields=items;world->field_capacity=cap;}
    SrForceField *field=&world->fields[world->field_count++];field->id=sr_strdup(id);if(!field->id)SR_XML_FAIL_RETURN(ctx,"forceField",NULL,"out of memory");
    if(!strcmp(type,"radial"))field->type=SR_FIELD_RADIAL;
    else if(!strcmp(type,"vortex"))field->type=SR_FIELD_VORTEX;
    else if(!strcmp(type,"directional"))field->type=SR_FIELD_DIRECTIONAL;
    else SR_XML_FAIL_RETURN(ctx,"forceField","type","expected radial, vortex, or directional");
    if(!decimal(ctx,"forceField",attrs,"x",&field->x.base)||!decimal(ctx,"forceField",attrs,"y",&field->y.base)||
       !decimal(ctx,"forceField",attrs,"forceX",&field->force_x.base)||!decimal(ctx,"forceField",attrs,"forceY",&field->force_y.base)||
       !decimal(ctx,"forceField",attrs,"strength",&field->strength.base)||!decimal(ctx,"forceField",attrs,"falloff",&field->falloff))return;
    sr_xml_push(ctx,(ParseFrame){.kind=E_FORCE_FIELD,.field=field,.curve=SR_CURVE_LINEAR},"forceField");
}

void sr_xml_start_constraint(ParseContext *ctx,const XML_Char **attrs){
    const char *const allowed[]={"id","type","a","b","restLength","stiffness","damping","x","y"};
    if(!sr_xml_attrs_allowed(ctx,"constraint",attrs,allowed,9))return;
    const char *id=sr_xml_required(ctx,"constraint",attrs,"id");const char *type=sr_xml_required(ctx,"constraint",attrs,"type");
    const char *a=sr_xml_required(ctx,"constraint",attrs,"a");if(ctx->failed)return;
    bool pin=!strcmp(type,"pin");
    const char *b=pin?sr_xml_attr(attrs,"b"):sr_xml_required(ctx,"constraint",attrs,"b");if(ctx->failed)return;
    if(pin&&b)SR_XML_FAIL_RETURN(ctx,"constraint","b","a pin constraint takes one body 'a' and an anchor x/y");
    if(!pin&&(sr_xml_attr(attrs,"x")||sr_xml_attr(attrs,"y")))
        SR_XML_FAIL_RETURN(ctx,"constraint","x/y","x/y anchor requires type=pin");
    if(!sr_id_valid(id))SR_XML_FAIL_RETURN(ctx,"constraint","id","invalid identifier");
    if(sr_scene_id_exists(ctx->scene,id))SR_XML_FAIL_RETURN(ctx,"constraint","id","id must be globally unique");
    if(strcmp(type,"spring")&&strcmp(type,"distance")&&!pin)SR_XML_FAIL_RETURN(ctx,"constraint","type","expected spring, distance, or pin");
    SrPhysicsWorld *world=&ctx->scene->physics;if(world->constraint_count==world->constraint_capacity){size_t cap=world->constraint_capacity?world->constraint_capacity*2:4;
        SrConstraint *items=sr_realloc(world->constraints,cap*sizeof(*items));if(!items)SR_XML_FAIL_RETURN(ctx,"constraint",NULL,"out of memory");
        memset(items+world->constraint_capacity,0,(cap-world->constraint_capacity)*sizeof(*items));world->constraints=items;world->constraint_capacity=cap;}
    SrConstraint *constraint=&world->constraints[world->constraint_count++];constraint->source_line=sr_xml_line(ctx);constraint->id=sr_strdup(id);constraint->a_id=sr_strdup(a);
    constraint->b_id=b?sr_strdup(b):NULL;
    constraint->type=pin?SR_CONSTRAINT_PIN:!strcmp(type,"distance")?SR_CONSTRAINT_DISTANCE:SR_CONSTRAINT_SPRING;
    constraint->stiffness=!strcmp(type,"distance")?1000:20;constraint->damping=1;
    if(!constraint->id||!constraint->a_id||(b&&!constraint->b_id))SR_XML_FAIL_RETURN(ctx,"constraint",NULL,"out of memory");
    if(!decimal(ctx,"constraint",attrs,"restLength",&constraint->rest_length)||
       !decimal(ctx,"constraint",attrs,"stiffness",&constraint->stiffness)||
       !decimal(ctx,"constraint",attrs,"damping",&constraint->damping)||
       !decimal(ctx,"constraint",attrs,"x",&constraint->x)||
       !decimal(ctx,"constraint",attrs,"y",&constraint->y))return;
    constraint->rest_length_set=sr_xml_attr(attrs,"restLength")!=NULL;
    /* A pin without stiffness is rigid: projected exactly every step. */
    constraint->rigid=pin&&!sr_xml_attr(attrs,"stiffness");
    if(pin&&!sr_xml_attr(attrs,"damping"))constraint->damping=pin&&constraint->rigid?0:1;
    if(constraint->rest_length<0||constraint->stiffness<0||constraint->damping<0)
        SR_XML_FAIL_RETURN(ctx,"constraint","restLength/stiffness/damping","expected non-negative values");
}

void sr_xml_start_rigid_body(ParseContext *ctx,const XML_Char **attrs){
    const char *const allowed[]={"type","shape","mass","friction","restitution","linearDamping","angularDamping","velocityX","velocityY","angularVelocity","radius"};
    if (!sr_xml_attrs_allowed(ctx,"rigidBody",attrs,allowed,11)) return;
    ParseFrame *parent=sr_xml_parent(ctx);
    if(!parent||!parent->node)return;
    SrRigidBody *body=&parent->node->body;
    const char *type=sr_xml_attr(attrs,"type");if(!type||!strcmp(type,"dynamic"))body->type=SR_BODY_DYNAMIC;else if(!strcmp(type,"static"))body->type=SR_BODY_STATIC;else if(!strcmp(type,"kinematic"))body->type=SR_BODY_KINEMATIC;else SR_XML_FAIL_RETURN(ctx,"rigidBody","type","expected static, kinematic, or dynamic");
    const char *shape=sr_xml_attr(attrs,"shape");if(shape&&!strcmp(shape,"circle"))body->collider=SR_COLLIDER_CIRCLE;else if(shape&&strcmp(shape,"box"))SR_XML_FAIL_RETURN(ctx,"rigidBody","shape","expected box or circle");
    if(!decimal(ctx,"rigidBody",attrs,"mass",&body->mass)||!decimal(ctx,"rigidBody",attrs,"friction",&body->friction)||
       !decimal(ctx,"rigidBody",attrs,"restitution",&body->restitution)||!decimal(ctx,"rigidBody",attrs,"linearDamping",&body->linear_damping)||
       !decimal(ctx,"rigidBody",attrs,"angularDamping",&body->angular_damping)||!decimal(ctx,"rigidBody",attrs,"velocityX",&body->velocity_x)||
       !decimal(ctx,"rigidBody",attrs,"velocityY",&body->velocity_y)||!decimal(ctx,"rigidBody",attrs,"angularVelocity",&body->angular_velocity)||
       !decimal(ctx,"rigidBody",attrs,"radius",&body->radius))return;
    if(body->mass<=0||body->friction<0||body->friction>1||body->restitution<0||body->restitution>1)
        SR_XML_FAIL_RETURN(ctx,"rigidBody","mass/friction/restitution","invalid rigid body values");
    if(body->linear_damping<0||body->angular_damping<0)
        SR_XML_FAIL_RETURN(ctx,"rigidBody","linearDamping/angularDamping","expected non-negative damping");
    sr_xml_push(ctx,(ParseFrame){.kind=E_RIGID_BODY,.node=parent->node},"rigidBody");
}

void sr_xml_start_soft_body(ParseContext *ctx,const XML_Char **attrs){
    const char *const allowed[]={"mass","stiffness","damping","pressure","rows","cols","pin"};if(!sr_xml_attrs_allowed(ctx,"softBody",attrs,allowed,7))return;
    ParseFrame *parent=sr_xml_parent(ctx);if(!parent||!parent->node)return;SrSoftBody *body=&parent->node->soft_body;body->enabled=true;body->mass=1;body->stiffness=20;body->damping=.1;
    body->rows=body->cols=4;body->pin=SR_PIN_NONE;
    if(!decimal(ctx,"softBody",attrs,"mass",&body->mass)||!decimal(ctx,"softBody",attrs,"stiffness",&body->stiffness)||
       !decimal(ctx,"softBody",attrs,"damping",&body->damping)||!decimal(ctx,"softBody",attrs,"pressure",&body->pressure))return;
    if(body->mass<=0||body->stiffness<=0||body->damping<0)SR_XML_FAIL_RETURN(ctx,"softBody","mass/stiffness/damping","invalid soft body values");
    const char *value=sr_xml_attr(attrs,"rows");
    if(value&&(!sr_parse_u32(value,&body->rows)||body->rows<2||body->rows>16))SR_XML_FAIL_RETURN(ctx,"softBody","rows","expected an integer in [2,16]");
    value=sr_xml_attr(attrs,"cols");
    if(value&&(!sr_parse_u32(value,&body->cols)||body->cols<2||body->cols>16))SR_XML_FAIL_RETURN(ctx,"softBody","cols","expected an integer in [2,16]");
    value=sr_xml_attr(attrs,"pin");
    if(value){
        static const char *const names[]={"none","top","bottom","left","right","corners"};
        bool found=false;
        for(int i=0;i<6;++i)if(!strcmp(value,names[i])){body->pin=(SrPinMode)i;found=true;}
        if(!found)SR_XML_FAIL_RETURN(ctx,"softBody","pin","expected top, bottom, left, right, corners, or none");
    }
    if(!soft_body_stable(ctx,body,sr_xml_line(ctx)))return;
    sr_xml_push(ctx,(ParseFrame){.kind=E_SOFT_BODY,.node=parent->node},"softBody");
}

void sr_xml_start_deform(ParseContext *ctx,const XML_Char **attrs){if(attrs&&attrs[0])SR_XML_FAIL_RETURN(ctx,"deform",attrs[0],"deform has no attributes");
    ParseFrame *parent=sr_xml_parent(ctx);if(!parent||!parent->node)return;sr_xml_push(ctx,(ParseFrame){.kind=E_DEFORM,.node=parent->node},"deform");}

void sr_xml_start_modifier(ParseContext *ctx,const XML_Char **attrs){
    const char *const allowed[]={"type","amount","frequency","phase","axis","rows","cols"};if(!sr_xml_attrs_allowed(ctx,"modifier",attrs,allowed,7))return;
    ParseFrame *parent=sr_xml_parent(ctx);const char *type=sr_xml_required(ctx,"modifier",attrs,"type");if(!parent||!parent->node||!type)return;
    SrModifier modifier={0};modifier.frequency.base=1;modifier.axis='y';
    if(!strcmp(type,"bend"))modifier.type=SR_MOD_BEND;else if(!strcmp(type,"twist"))modifier.type=SR_MOD_TWIST;else if(!strcmp(type,"wave"))modifier.type=SR_MOD_WAVE;
    else if(!strcmp(type,"squash"))modifier.type=SR_MOD_SQUASH;else if(!strcmp(type,"stretch"))modifier.type=SR_MOD_STRETCH;
    else if(!strcmp(type,"mesh-warp"))modifier.type=SR_MOD_MESH_WARP;else SR_XML_FAIL_RETURN(ctx,"modifier","type","unsupported modifier type");
    if(modifier.type!=SR_MOD_MESH_WARP&&(sr_xml_attr(attrs,"rows")||sr_xml_attr(attrs,"cols")))
        SR_XML_FAIL_RETURN(ctx,"modifier","rows/cols","rows/cols require type=mesh-warp");
    if(modifier.type==SR_MOD_MESH_WARP){
        modifier.rows=modifier.cols=4;
        const char *value=sr_xml_attr(attrs,"rows");
        if(value&&(!sr_parse_u32(value,&modifier.rows)||modifier.rows<2||modifier.rows>16))SR_XML_FAIL_RETURN(ctx,"modifier","rows","expected an integer in [2,16]");
        value=sr_xml_attr(attrs,"cols");
        if(value&&(!sr_parse_u32(value,&modifier.cols)||modifier.cols<2||modifier.cols>16))SR_XML_FAIL_RETURN(ctx,"modifier","cols","expected an integer in [2,16]");
    }
    if(!decimal(ctx,"modifier",attrs,"amount",&modifier.amount.base)||!decimal(ctx,"modifier",attrs,"frequency",&modifier.frequency.base)||
       !decimal(ctx,"modifier",attrs,"phase",&modifier.phase.base)) return;
    const char *axis=sr_xml_attr(attrs,"axis");
    if(axis){if(strlen(axis)!=1||(axis[0]!='x'&&axis[0]!='y'))SR_XML_FAIL_RETURN(ctx,"modifier","axis","expected x or y");modifier.axis=axis[0];}
    /* Allocated only once every attribute is valid: until the modifier is
     * attached to the node nothing else would free it. */
    if(modifier.type==SR_MOD_MESH_WARP){
        modifier.points=sr_alloc((size_t)modifier.rows*modifier.cols*2*sizeof(*modifier.points));
        if(!modifier.points)SR_XML_FAIL_RETURN(ctx,"modifier",NULL,"out of memory");
    }
    if(sr_node_add_modifier(parent->node,modifier)!=SR_OK){
        free(modifier.points);
        SR_XML_FAIL_RETURN(ctx,"modifier",NULL,"out of memory");}
    SrModifier *stored=&parent->node->modifiers[parent->node->modifier_count-1];
    sr_xml_push(ctx,(ParseFrame){.kind=E_MODIFIER,.node=parent->node,.modifier=stored},"modifier");
}

void sr_xml_start_point(ParseContext *ctx,const XML_Char **attrs){
    const char *const allowed[]={"row","col","x","y"};
    if(!sr_xml_attrs_allowed(ctx,"point",attrs,allowed,4))return;
    ParseFrame *parent=sr_xml_parent(ctx);
    const char *row_text=sr_xml_required(ctx,"point",attrs,"row");
    const char *col_text=sr_xml_required(ctx,"point",attrs,"col");
    if(!parent||!parent->modifier||ctx->failed)return;
    SrModifier *modifier=parent->modifier;
    if(modifier->type!=SR_MOD_MESH_WARP)SR_XML_FAIL_RETURN(ctx,"point",NULL,"point requires a mesh-warp modifier");
    uint32_t row,col;
    if(!sr_parse_u32(row_text,&row)||row>=modifier->rows)SR_XML_FAIL_RETURN(ctx,"point","row","row is outside the control grid");
    if(!sr_parse_u32(col_text,&col)||col>=modifier->cols)SR_XML_FAIL_RETURN(ctx,"point","col","col is outside the control grid");
    SrAnimValue *point=&modifier->points[2*((size_t)row*modifier->cols+col)];
    if(!decimal(ctx,"point",attrs,"x",&point[0].base)||!decimal(ctx,"point",attrs,"y",&point[1].base))return;
    sr_xml_push(ctx,(ParseFrame){.kind=E_POINT,.node=parent->node,.point=point,.curve=SR_CURVE_LINEAR},"point");
}
