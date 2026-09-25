#define _POSIX_C_SOURCE 200809L
#include "scene_render/gpu.h"

#include <dlfcn.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

typedef int32_t ClInt;
typedef uint32_t ClUInt;
typedef uint64_t ClBits;
typedef intptr_t ClProperties;
typedef void *ClPlatform;
typedef void *ClDevice;
typedef void *ClContext;
typedef void *ClQueue;
typedef void *ClProgram;
typedef void *ClKernel;
typedef void *ClMemory;
typedef void *ClEvent;

enum { CL_SUCCESS=0,CL_TRUE=1,CL_DEVICE_NAME=0x102B,
       CL_PROGRAM_BUILD_LOG=0x1183 };
#define CL_DEVICE_TYPE_GPU ((ClBits)1U<<2)
#define CL_MEM_READ_WRITE ((ClBits)1U<<0)

typedef ClInt (*GetPlatformIDs)(ClUInt,ClPlatform*,ClUInt*);
typedef ClInt (*GetDeviceIDs)(ClPlatform,ClBits,ClUInt,ClDevice*,ClUInt*);
typedef ClContext (*CreateContext)(const ClProperties*,ClUInt,const ClDevice*,
    void (*)(const char*,const void*,size_t,void*),void*,ClInt*);
typedef ClQueue (*CreateQueue)(ClContext,ClDevice,ClBits,ClInt*);
typedef ClProgram (*CreateProgram)(ClContext,ClUInt,const char**,const size_t*,ClInt*);
typedef ClInt (*BuildProgram)(ClProgram,ClUInt,const ClDevice*,const char*,
                              void (*)(ClProgram,void*),void*);
typedef ClKernel (*CreateKernel)(ClProgram,const char*,ClInt*);
typedef ClMemory (*CreateBuffer)(ClContext,ClBits,size_t,void*,ClInt*);
typedef ClInt (*SetKernelArg)(ClKernel,ClUInt,size_t,const void*);
typedef ClInt (*WriteBuffer)(ClQueue,ClMemory,ClUInt,size_t,size_t,const void*,
                             ClUInt,const ClEvent*,ClEvent*);
typedef ClInt (*ReadBuffer)(ClQueue,ClMemory,ClUInt,size_t,size_t,void*,ClUInt,
                            const ClEvent*,ClEvent*);
typedef ClInt (*RunKernel)(ClQueue,ClKernel,ClUInt,const size_t*,const size_t*,
                           const size_t*,ClUInt,const ClEvent*,ClEvent*);
typedef ClInt (*Finish)(ClQueue);typedef ClInt (*Release)(void*);
typedef ClInt (*GetProgramInfo)(ClProgram,ClDevice,ClUInt,size_t,void*,size_t*);
typedef ClInt (*GetDeviceInfo)(ClDevice,ClUInt,size_t,void*,size_t*);

typedef struct {
    void *library;ClDevice device;ClContext context;ClQueue queue;
    ClProgram program;ClKernel kernel;char device_name[256];
    GetPlatformIDs get_platforms;GetDeviceIDs get_devices;CreateContext create_context;
    CreateQueue create_queue;CreateProgram create_program;BuildProgram build_program;
    CreateKernel create_kernel;CreateBuffer create_buffer;SetKernelArg set_argument;
    WriteBuffer write_buffer;ReadBuffer read_buffer;RunKernel run_kernel;Finish finish;
    Release release_memory,release_kernel,release_program,release_queue,release_context;
    GetProgramInfo get_program_info;GetDeviceInfo get_device_info;
} GpuImpl;

static const char kernel_source[]=
"float dec(float v,int s){if(s==2||s==3){float a=s==2?1.0992968268f:1.099f,b=s==2?.0180539685f:.018f;return v<4.5f*b?v/4.5f:pow((v+a-1.f)/a,1.f/.45f);}return v<=.04045f?v/12.92f:pow((v+.055f)/1.055f,2.4f);}"
"float enc(float v,int s){v=clamp(v,0.f,1.f);if(s==2||s==3){float a=s==2?1.0992968268f:1.099f,b=s==2?.0180539685f:.018f;return v<b?v*4.5f:a*pow(v,.45f)-(a-1.f);}return v<=.0031308f?v*12.92f:1.055f*pow(v,1.f/2.4f)-.055f;}"
"float3 xyz(float3 v,int s){if(s==1)return(float3)(dot(v,(float3)(.48657095f,.26566769f,.19821729f)),dot(v,(float3)(.22897456f,.69173852f,.07928691f)),dot(v,(float3)(0.f,.04511338f,1.04394437f)));if(s==2)return(float3)(dot(v,(float3)(.63695805f,.14461690f,.16888098f)),dot(v,(float3)(.26270021f,.67799807f,.05930172f)),dot(v,(float3)(0.f,.02807269f,1.06098506f)));return(float3)(dot(v,(float3)(.4124564f,.3575761f,.1804375f)),dot(v,(float3)(.2126729f,.7151522f,.072175f)),dot(v,(float3)(.0193339f,.119192f,.9503041f)));}"
"float3 rgb(float3 v,int s){if(s==1)return(float3)(dot(v,(float3)(2.4934969f,-.9313836f,-.4027108f)),dot(v,(float3)(-.829489f,1.762664f,.0236247f)),dot(v,(float3)(.0358458f,-.0761724f,.9568845f)));if(s==2)return(float3)(dot(v,(float3)(1.7166512f,-.3556708f,-.2533663f)),dot(v,(float3)(-.6666844f,1.6164812f,.0157685f)),dot(v,(float3)(.0176399f,-.0427706f,.9421031f)));return(float3)(dot(v,(float3)(3.2404542f,-1.5371385f,-.4985314f)),dot(v,(float3)(-.969266f,1.8760108f,.041556f)),dot(v,(float3)(.0556434f,-.2040259f,1.0572252f)));}"
"int samegamut(int a,int b){return a==b||((a==0||a==3)&&(b==0||b==3));}"
"uchar code(float v){return convert_uchar_sat(floor(clamp(v,0.f,1.f)*255.f+.5f));}"
"__kernel void convert(__global const float4*in,__global uchar4*out,int linear,int working,int target,uint count){uint i=get_global_id(0);if(i>=count)return;float4 p=in[i];uchar alpha=code(p.w);if(!(p.w>0.f)){out[i]=(uchar4)(0,0,0,alpha);return;}float3 v=p.xyz/p.w;if(!linear&&working==target){out[i]=(uchar4)(code(v.x),code(v.y),code(v.z),alpha);return;}if(!linear){v=clamp(v,0.f,1.f);v=(float3)(dec(v.x,working),dec(v.y,working),dec(v.z,working));}if(!samegamut(working,target))v=rgb(xyz(v,working),target);v=(float3)(enc(v.x,target),enc(v.y,target),enc(v.z,target));out[i]=(uchar4)(code(v.x),code(v.y),code(v.z),alpha);}";

static bool symbol(void *library, const char *name, void *target, size_t size) {
    void *value = dlsym(library, name);
    if (!value) return false;
    memcpy(target, &value, size);
    return true;
}

#define LOAD(impl,field,name) symbol((impl)->library,(name),&(impl)->field,sizeof((impl)->field))

static void release_impl(GpuImpl *impl) {
    if (!impl) return;
    if (impl->kernel) impl->release_kernel(impl->kernel);
    if (impl->program) impl->release_program(impl->program);
    if (impl->queue) impl->release_queue(impl->queue);
    if (impl->context) impl->release_context(impl->context);
    if (impl->library) dlclose(impl->library);
    free(impl);
}

bool sr_gpu_open(SrGpu *gpu,SrDiagnostics *diag){if(!gpu)return false;*gpu=(SrGpu){0};
    GpuImpl *impl=sr_alloc(sizeof(*impl));if(!impl)return false;
    impl->library=dlopen("libOpenCL.so.1",RTLD_NOW|RTLD_LOCAL);if(!impl->library){free(impl);return false;}
    bool loaded=LOAD(impl,get_platforms,"clGetPlatformIDs")&&LOAD(impl,get_devices,"clGetDeviceIDs")&&
        LOAD(impl,create_context,"clCreateContext")&&LOAD(impl,create_queue,"clCreateCommandQueue")&&
        LOAD(impl,create_program,"clCreateProgramWithSource")&&LOAD(impl,build_program,"clBuildProgram")&&
        LOAD(impl,create_kernel,"clCreateKernel")&&LOAD(impl,create_buffer,"clCreateBuffer")&&
        LOAD(impl,set_argument,"clSetKernelArg")&&LOAD(impl,write_buffer,"clEnqueueWriteBuffer")&&
        LOAD(impl,read_buffer,"clEnqueueReadBuffer")&&LOAD(impl,run_kernel,"clEnqueueNDRangeKernel")&&
        LOAD(impl,finish,"clFinish")&&LOAD(impl,release_memory,"clReleaseMemObject")&&
        LOAD(impl,release_kernel,"clReleaseKernel")&&LOAD(impl,release_program,"clReleaseProgram")&&
        LOAD(impl,release_queue,"clReleaseCommandQueue")&&LOAD(impl,release_context,"clReleaseContext")&&
        LOAD(impl,get_program_info,"clGetProgramBuildInfo")&&LOAD(impl,get_device_info,"clGetDeviceInfo");
    if(!loaded){release_impl(impl);return false;}ClUInt count=0;
    if(impl->get_platforms(0,NULL,&count)!=CL_SUCCESS||!count){release_impl(impl);return false;}
    ClPlatform *platforms=sr_alloc(count*sizeof(*platforms));if(!platforms){release_impl(impl);return false;}
    if(impl->get_platforms(count,platforms,NULL)!=CL_SUCCESS){free(platforms);release_impl(impl);return false;}
    bool found=false;for(ClUInt i=0;i<count&&!found;++i)
        if(impl->get_devices(platforms[i],CL_DEVICE_TYPE_GPU,1,&impl->device,NULL)==CL_SUCCESS)found=true;
    free(platforms);if(!found){release_impl(impl);return false;}ClInt error=0;
    impl->context=impl->create_context(NULL,1,&impl->device,NULL,NULL,&error);
    if(!impl->context||error!=CL_SUCCESS){release_impl(impl);return false;}
    impl->queue=impl->create_queue(impl->context,impl->device,0,&error);
    impl->program=impl->create_program(impl->context,1,(const char*[]){kernel_source},NULL,&error);
    if(!impl->queue||!impl->program||error!=CL_SUCCESS){release_impl(impl);return false;}
    if(impl->build_program(impl->program,1,&impl->device,"-cl-std=CL1.2",NULL,NULL)!=CL_SUCCESS){
        char log[2048]={0};impl->get_program_info(impl->program,impl->device,CL_PROGRAM_BUILD_LOG,sizeof(log)-1,log,NULL);
        sr_diag_warning(diag,0,NULL,NULL,"OpenCL kernel build failed: %s",log);release_impl(impl);return false;}
    impl->kernel=impl->create_kernel(impl->program,"convert",&error);
    if(!impl->kernel||error!=CL_SUCCESS){release_impl(impl);return false;}
    impl->get_device_info(impl->device,CL_DEVICE_NAME,sizeof(impl->device_name)-1,impl->device_name,NULL);
    gpu->implementation=impl;return true;}

void sr_gpu_close(SrGpu *gpu){if(gpu&&gpu->implementation){release_impl(gpu->implementation);gpu->implementation=NULL;}}
const char *sr_gpu_device_name(const SrGpu *gpu){GpuImpl *impl=gpu?gpu->implementation:NULL;return impl?impl->device_name:"unavailable";}

SrStatus sr_gpu_convert_frame(SrGpu *gpu,const SrFrame *frame,uint8_t *rgba8,
                              const SrProject *project,SrColorSpace target,
                              SrDiagnostics *diag){
    GpuImpl *impl=gpu?gpu->implementation:NULL;
    if(!impl||!frame||!frame->px||!rgba8||!project)return SR_ERR_ARGUMENT;
    size_t pixels=(size_t)frame->width*frame->height;
    if(pixels>UINT32_MAX){
        sr_diag_warning(diag,0,NULL,NULL,"OpenCL frame exceeds 32-bit work-item limit");
        return SR_ERR_RENDER;
    }
    size_t in_bytes=pixels*4*sizeof(float),out_bytes=pixels*4;ClInt error=0,out_error=0;
    ClMemory input=impl->create_buffer(impl->context,CL_MEM_READ_WRITE,in_bytes,NULL,&error);
    ClMemory output=impl->create_buffer(impl->context,CL_MEM_READ_WRITE,out_bytes,NULL,&out_error);
    if(!input||!output||error!=CL_SUCCESS||out_error!=CL_SUCCESS){
        if(input)impl->release_memory(input);
        if(output)impl->release_memory(output);
        sr_diag_warning(diag,0,NULL,NULL,"OpenCL frame buffer allocation failed");
        return SR_ERR_RENDER;
    }
    ClUInt count=(ClUInt)pixels;
    int linear=project->linear_light?1:0,working=project->working_color_space,target_value=target;
    error=impl->write_buffer(impl->queue,input,CL_TRUE,0,in_bytes,frame->px,0,NULL,NULL);
    error|=impl->set_argument(impl->kernel,0,sizeof(input),&input);
    error|=impl->set_argument(impl->kernel,1,sizeof(output),&output);
    error|=impl->set_argument(impl->kernel,2,sizeof(linear),&linear);
    error|=impl->set_argument(impl->kernel,3,sizeof(working),&working);
    error|=impl->set_argument(impl->kernel,4,sizeof(target_value),&target_value);
    error|=impl->set_argument(impl->kernel,5,sizeof(count),&count);size_t global=count;
    error|=impl->run_kernel(impl->queue,impl->kernel,1,NULL,&global,NULL,0,NULL,NULL);
    /* Read into the caller's buffer only once everything succeeded. */
    uint8_t *converted=error==CL_SUCCESS?sr_alloc(out_bytes):NULL;
    if(converted)error|=impl->read_buffer(impl->queue,output,CL_TRUE,0,out_bytes,converted,0,NULL,NULL);
    error|=impl->finish(impl->queue);
    impl->release_memory(input);impl->release_memory(output);
    if(error!=CL_SUCCESS||!converted){free(converted);sr_diag_warning(diag,0,NULL,NULL,"OpenCL frame conversion failed (%d)",error);return SR_ERR_RENDER;}
    memcpy(rgba8,converted,out_bytes);free(converted);
    return SR_OK;}
