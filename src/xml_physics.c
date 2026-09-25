#include "xml_internal.h"

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

void sr_xml_start_physics(ParseContext *ctx,const XML_Char **attrs){
    const char *const allowed[]={"fixedStep","gravityX","gravityY","cache"};
    if(!sr_xml_attrs_allowed(ctx,"physics",attrs,allowed,4))return;
    SrPhysicsWorld *world=&ctx->scene->physics;world->enabled=true;
    if(!decimal(ctx,"physics",attrs,"fixedStep",&world->fixed_step)||
       !decimal(ctx,"physics",attrs,"gravityX",&world->gravity_x)||
       !decimal(ctx,"physics",attrs,"gravityY",&world->gravity_y))return;
    if(world->fixed_step<=0||world->fixed_step>1.0)
        SR_XML_FAIL_RETURN(ctx,"physics","fixedStep","expected a value in (0,1]");
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
    if(!strcmp(type,"radial"))field->radial=true;else if(strcmp(type,"directional"))SR_XML_FAIL_RETURN(ctx,"forceField","type","expected radial or directional");
    if(!decimal(ctx,"forceField",attrs,"x",&field->x)||!decimal(ctx,"forceField",attrs,"y",&field->y)||
       !decimal(ctx,"forceField",attrs,"forceX",&field->force_x)||!decimal(ctx,"forceField",attrs,"forceY",&field->force_y)||
       !decimal(ctx,"forceField",attrs,"strength",&field->strength)||!decimal(ctx,"forceField",attrs,"falloff",&field->falloff))return;
}

void sr_xml_start_constraint(ParseContext *ctx,const XML_Char **attrs){
    const char *const allowed[]={"id","type","a","b","restLength","stiffness","damping"};
    if(!sr_xml_attrs_allowed(ctx,"constraint",attrs,allowed,7))return;
    const char *id=sr_xml_required(ctx,"constraint",attrs,"id");const char *type=sr_xml_required(ctx,"constraint",attrs,"type");
    const char *a=sr_xml_required(ctx,"constraint",attrs,"a");const char *b=sr_xml_required(ctx,"constraint",attrs,"b");if(ctx->failed)return;
    if(!sr_id_valid(id))SR_XML_FAIL_RETURN(ctx,"constraint","id","invalid identifier");
    if(sr_scene_id_exists(ctx->scene,id))SR_XML_FAIL_RETURN(ctx,"constraint","id","id must be globally unique");
    if(strcmp(type,"spring")&&strcmp(type,"distance"))SR_XML_FAIL_RETURN(ctx,"constraint","type","expected spring or distance");
    SrPhysicsWorld *world=&ctx->scene->physics;if(world->constraint_count==world->constraint_capacity){size_t cap=world->constraint_capacity?world->constraint_capacity*2:4;
        SrConstraint *items=sr_realloc(world->constraints,cap*sizeof(*items));if(!items)SR_XML_FAIL_RETURN(ctx,"constraint",NULL,"out of memory");
        memset(items+world->constraint_capacity,0,(cap-world->constraint_capacity)*sizeof(*items));world->constraints=items;world->constraint_capacity=cap;}
    SrConstraint *constraint=&world->constraints[world->constraint_count++];constraint->source_line=sr_xml_line(ctx);constraint->id=sr_strdup(id);constraint->a_id=sr_strdup(a);constraint->b_id=sr_strdup(b);
    constraint->stiffness=!strcmp(type,"distance")?1000:20;constraint->damping=1;
    if(!constraint->id||!constraint->a_id||!constraint->b_id)SR_XML_FAIL_RETURN(ctx,"constraint",NULL,"out of memory");
    if(!decimal(ctx,"constraint",attrs,"restLength",&constraint->rest_length)||
       !decimal(ctx,"constraint",attrs,"stiffness",&constraint->stiffness)||
       !decimal(ctx,"constraint",attrs,"damping",&constraint->damping))return;
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
    const char *const allowed[]={"mass","stiffness","damping","pressure"};if(!sr_xml_attrs_allowed(ctx,"softBody",attrs,allowed,4))return;
    ParseFrame *parent=sr_xml_parent(ctx);if(!parent||!parent->node)return;SrSoftBody *body=&parent->node->soft_body;body->enabled=true;body->mass=1;body->stiffness=20;body->damping=.1;
    if(!decimal(ctx,"softBody",attrs,"mass",&body->mass)||!decimal(ctx,"softBody",attrs,"stiffness",&body->stiffness)||
       !decimal(ctx,"softBody",attrs,"damping",&body->damping)||!decimal(ctx,"softBody",attrs,"pressure",&body->pressure))return;
    if(body->mass<=0||body->stiffness<=0||body->damping<0)SR_XML_FAIL_RETURN(ctx,"softBody","mass/stiffness/damping","invalid soft body values");
    sr_xml_push(ctx,(ParseFrame){.kind=E_SOFT_BODY,.node=parent->node},"softBody");
}

void sr_xml_start_deform(ParseContext *ctx,const XML_Char **attrs){if(attrs&&attrs[0])SR_XML_FAIL_RETURN(ctx,"deform",attrs[0],"deform has no attributes");
    ParseFrame *parent=sr_xml_parent(ctx);if(!parent||!parent->node)return;sr_xml_push(ctx,(ParseFrame){.kind=E_DEFORM,.node=parent->node},"deform");}

void sr_xml_start_modifier(ParseContext *ctx,const XML_Char **attrs){
    const char *const allowed[]={"type","amount","frequency","phase","axis"};if(!sr_xml_attrs_allowed(ctx,"modifier",attrs,allowed,5))return;
    ParseFrame *parent=sr_xml_parent(ctx);const char *type=sr_xml_required(ctx,"modifier",attrs,"type");if(!parent||!parent->node||!type)return;
    SrModifier modifier={0};modifier.frequency.base=1;modifier.axis='y';
    if(!strcmp(type,"bend"))modifier.type=SR_MOD_BEND;else if(!strcmp(type,"twist"))modifier.type=SR_MOD_TWIST;else if(!strcmp(type,"wave"))modifier.type=SR_MOD_WAVE;
    else if(!strcmp(type,"squash"))modifier.type=SR_MOD_SQUASH;else if(!strcmp(type,"stretch"))modifier.type=SR_MOD_STRETCH;else SR_XML_FAIL_RETURN(ctx,"modifier","type","unsupported modifier type");
    if(!decimal(ctx,"modifier",attrs,"amount",&modifier.amount.base)||!decimal(ctx,"modifier",attrs,"frequency",&modifier.frequency.base)||
       !decimal(ctx,"modifier",attrs,"phase",&modifier.phase.base)) return;
    const char *axis=sr_xml_attr(attrs,"axis");
    if(axis){if(strlen(axis)!=1||(axis[0]!='x'&&axis[0]!='y'))SR_XML_FAIL_RETURN(ctx,"modifier","axis","expected x or y");modifier.axis=axis[0];}
    if(sr_node_add_modifier(parent->node,modifier)!=SR_OK)
        SR_XML_FAIL_RETURN(ctx,"modifier",NULL,"out of memory");
    SrModifier *stored=&parent->node->modifiers[parent->node->modifier_count-1];
    sr_xml_push(ctx,(ParseFrame){.kind=E_MODIFIER,.node=parent->node,.modifier=stored},"modifier");
}
