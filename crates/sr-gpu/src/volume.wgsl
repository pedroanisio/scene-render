// Participating media packed into tverts. Each domain has 64 vec4 rows; each
// sparse 8^3 brick has one signed integer key row and 128 scalar-data rows.
fn volume_base(i: u32) -> u32 { return pp.media.x + i * 64u; }
fn volume_matrix(base: u32) -> mat4x4<f32> {
    return mat4x4(tverts[base],tverts[base+1u],tverts[base+2u],tverts[base+3u]);
}
fn volume_interval(base: u32, origin: vec3<f32>, direction: vec3<f32>, distance: f32) -> vec2<f32> {
    let inverse = volume_matrix(base);
    let o = (inverse * vec4(origin,1.0)).xyz;
    let d = (inverse * vec4(direction,0.0)).xyz;
    let lo = tverts[base+4u].xyz; let hi = tverts[base+5u].xyz;
    if (any(lo >= hi)) { return vec2(0.0); }
    var near = 0.0; var far = distance;
    for (var axis=0u; axis<3u; axis++) {
        if (d[axis] == 0.0) {
            if (o[axis]<lo[axis] || o[axis]>hi[axis]) { return vec2(0.0); }
        } else {
            let a=(lo[axis]-o[axis])/d[axis]; let b=(hi[axis]-o[axis])/d[axis];
            near=max(near,min(a,b)); far=min(far,max(a,b));
        }
    }
    if (far <= near) { return vec2(0.0); }
    return vec2(near,far);
}
fn volume_key_less(a: vec3<i32>, b: vec3<i32>) -> bool {
    return a.x<b.x || (a.x==b.x && (a.y<b.y || (a.y==b.y && a.z<b.z)));
}
// Base row of the domain whose density grid is being sampled when it has a brick directory (rows
// base+23 and base+29 of its record: the lowest brick key, the table offset and the extent), else
// NO_DIRECTORY, and the voxel is found by searching the sorted brick list.
const NO_DIRECTORY: u32 = 0xffffffffu;
var<private> directory_base: u32 = NO_DIRECTORY;
fn volume_voxel(info: vec4<f32>, index: vec3<i32>) -> f32 {
    let key=vec3<i32>(floor(vec3<f32>(index)/8.0));
    if (directory_base!=NO_DIRECTORY) {
        let rel=key-bitcast<vec3<i32>>(tverts[directory_base+23u].xyz);
        let dims=bitcast<vec3<u32>>(tverts[directory_base+29u].xyz);
        if (any(rel<vec3<i32>(0)) || any(vec3<u32>(rel)>=dims)) { return info.x; }
        let entry=u32(rel.x)+dims.x*(u32(rel.y)+dims.y*u32(rel.z));
        let row=bitcast<u32>(tverts[bitcast<u32>(tverts[directory_base+23u].w)+entry/4u][entry%4u]);
        if (row==NO_DIRECTORY) { return info.x; }
        let q=vec3<u32>(index-key*8);
        let scalar=q.x+q.y*8u+q.z*64u;
        return tverts[row+1u+scalar/4u][scalar%4u];
    }
    var lo=0u; var hi=bitcast<u32>(info.y); let offset=bitcast<u32>(info.z);
    loop {
        if (lo>=hi) { break; }
        let mid=lo+(hi-lo)/2u;
        let k=bitcast<vec3<i32>>(tverts[offset+mid*129u].xyz);
        if (volume_key_less(k,key)) { lo=mid+1u; } else { hi=mid; }
    }
    if (lo>=bitcast<u32>(info.y)) { return info.x; }
    let row=offset+lo*129u;
    if (any(bitcast<vec3<i32>>(tverts[row].xyz)!=key)) { return info.x; }
    let q=vec3<u32>(index-key*8);
    let scalar=q.x+q.y*8u+q.z*64u;
    return tverts[row+1u+scalar/4u][scalar%4u];
}
fn volume_grid_sample(info: vec4<f32>, p: vec3<f32>) -> f32 {
    if (any(abs(p)>vec3(8388600.0))) { return info.x; }
    let lo=vec3<i32>(floor(p)); let f=fract(p);
    var value=0.0;
    // One loop over the eight corners, bounded at run time (pp.media.w is 8): drivers that unroll it inline the
    // sparse search eight times at every call site, which costs NVIDIA's OpenGL compiler minutes and gigabytes
    // and exhausts the software Vulkan driver's memory. The corners go in the order of x, then y, then z.
    for (var corner=0u; corner<pp.media.w; corner++) {
        let delta=vec3<i32>(i32(corner&1u),i32((corner>>1u)&1u),i32(corner>>2u));
        let weights=select(vec3(1.0)-f,f,delta==vec3(1));
        value+=volume_voxel(info,lo+delta)*weights.x*weights.y*weights.z;
    }
    return value;
}
fn volume_grid(base: u32, point: vec3<f32>) -> f32 {
    return volume_grid_sample(tverts[base+4u],(volume_matrix(base)*vec4(point,1.0)).xyz);
}
// The point of the domain advected back along its velocity to the frame `endpoint` (0 or 1) by `stages` midpoint
// stages. One loop over the stages and the velocity components, bounded at run time, keeps a single copy of the
// sparse search however drivers inline and unroll: each pass samples one component, the last one of a stage moves
// the point.
fn volume_advected_point(base: u32, point: vec3<f32>, endpoint: u32) -> vec3<f32> {
    if (tverts[base+14u].z<0.5) { return point; }
    let row=base+40u+endpoint*8u; let elapsed=tverts[row+7u].x;
    if (elapsed==0.0) { return point; }
    let local=(volume_matrix(base)*vec4(point,1.0)).xyz;
    let m=volume_matrix(row);
    let components=u32(tverts[row+7u].y);
    var traced=local; var p=(m*vec4(local,1.0)).xyz; var velocity=vec3(0.0);
    for (var j=0u; j<u32(tverts[row+7u].z)*components; j++) {
        let c=j%components;
        velocity=select(velocity,vec3(volume_grid_sample(tverts[row+4u+c],p)),vec3(c)==vec3(0u,1u,2u));
        if (c+1u==components) {
            traced=local-(elapsed*(0.5*f32(j/components+1u)))*velocity;
            p=(m*vec4(traced,1.0)).xyz; velocity=vec3(0.0);
        }
    }
    return (volume_matrix(base+56u)*vec4(traced,1.0)).xyz;
}
// Density (x) and, when `emission` is set, emitted radiance (yzw) of the domain at `point`; a point outside the domain
// has neither. Every frame's grids (density at rows 8 and 24, temperature at 16 and 32) are sampled at its point
// advected once, in one loop bounded at run time, so a march that needs both reaches the sparse search once.
fn volume_sample(base: u32, point: vec3<f32>, emission: bool) -> vec4<f32> {
    let local=(volume_matrix(base)*vec4(point,1.0)).xyz;
    if (any(local<tverts[base+4u].xyz) || any(local>tverts[base+5u].xyz)) { return vec4(0.0); }
    let thermal=tverts[base+21u];
    let kinds=select(1u,2u,emission && thermal.y>0.5);
    let frames=select(1u,2u,tverts[base+14u].y>0.5);
    var value=vec2(0.0); var sampled=point;
    for (var j=0u; j<frames*kinds; j++) {
        let frame=j/kinds; let kind=j%kinds;
        // the advected point is found first: velocity grids have no directory
        if (kind==0u) { sampled=volume_advected_point(base,point,frame); }
        if (j==0u && tverts[base+29u].w>0.5) { directory_base=base; }
        let sample=volume_grid(base+8u+kind*8u+frame*16u,sampled);
        directory_base=NO_DIRECTORY;
        let blended=select(sample,mix(value[kind],sample,tverts[base+14u].x),frame==1u);
        value=select(value,vec2(blended),vec2(kind)==vec2(0u,1u));
    }
    var rgb=vec3(0.0);
    if (emission) {
        rgb=tverts[base+7u].rgb;
        if (kinds==2u) {
            let kelvin=clamp(value.y*thermal.x,0.0,50000.0);
            let index=sqrt(kelvin/50000.0)*1024.0;
            let lo=min(u32(index),1023u); let fraction=index-f32(lo);
            let offset=bitcast<u32>(thermal.z);
            rgb+=mix(tverts[offset+lo].rgb,tverts[offset+lo+1u].rgb,fraction);
        }
    }
    return vec4(value.x*tverts[base+4u].w,rgb);
}
fn volume_density(base: u32, point: vec3<f32>) -> f32 { return volume_sample(base,point,false).x; }
fn volume_phase(cosine: f32, g: f32) -> f32 {
    let denom=1.0+g*g-2.0*g*clamp(cosine,-1.0,1.0);
    return (1.0-g*g)/(4.0*PI*denom*sqrt(denom));
}
// Skip map of a domain's density grid (see volume.rs): the cell holding `point` in 4-voxel cells
// of grid index space. Returns the ray parameter where the ray leaves that cell when it holds only
// negligible density, else -1. The caller must have checked that the domain has no advection and
// no second frame, so a sample reads only this grid.
fn volume_empty_until(base: u32, o: vec3<f32>, d: vec3<f32>, point: vec3<f32>) -> f32 {
    let m=volume_matrix(base+8u);
    let p=(m*vec4(point,1.0)).xyz;
    let c=vec3<i32>(floor(p/4.0));
    let first=tverts[base+15u];
    let dims=bitcast<vec3<u32>>(tverts[base+22u].xyz);
    let rel=c-bitcast<vec3<i32>>(first.xyz);
    if (all(rel>=vec3<i32>(0)) && all(vec3<u32>(rel)<dims)) {
        let cell=u32(rel.x)+dims.x*(u32(rel.y)+dims.y*u32(rel.z));
        let word=bitcast<u32>(tverts[bitcast<u32>(first.w)+cell/128u][(cell/32u)%4u]);
        if (((word>>(cell%32u))&1u)!=0u) { return -1.0; }
    }
    let oi=(m*vec4(o,1.0)).xyz; let di=(m*vec4(d,0.0)).xyz;
    var t=3.0e38;
    for (var axis=0u; axis<3u; axis++) {
        if (di[axis]>0.0) { t=min(t,(f32(c[axis]*4+4)-oi[axis])/di[axis]); }
        else if (di[axis]<0.0) { t=min(t,(f32(c[axis]*4)-oi[axis])/di[axis]); }
    }
    return t;
}
// Extinction is additive across overlapping domains; their transmittances multiply. Shadow rays from surfaces and
// the light grids' nodes march with this; volume_scattering marches the shadow rays of the media's own light.
// Samples in cells whose density is negligible are jumped over (stopping a little short of the
// cell's far side to stay clear of rounding). The map is built so that, along any ray through the
// domain, everything left out adds less than 1e-10 to the optical depth (volume.rs,
// SKIPPED_OPTICAL_DEPTH): about 300 times below half a unit in the last place of an f32 near 1, so
// far below the resolution of the transmittance, but not zero.
fn volume_transmittance(o: vec3<f32>, d: vec3<f32>, distance: f32) -> f32 {
    var optical_depth=0.0;
    for (var i=0u; i<pp.media.y; i++) {
        let base=volume_base(i); let interval=volume_interval(base,o,d,distance);
        if (tverts[base+13u].x<0.5) { continue; }
        if (interval.y<=interval.x) { continue; }
        let n=max(1u,u32(ceil((interval.y-interval.x)/tverts[base+12u].w)));
        let ds=(interval.y-interval.x)/f32(n);
        let skipping=tverts[base+22u].w>0.5;
        var k=0u;
        loop {
            if (k>=n) { break; }
            let point=o+d*(interval.x+(f32(k)+0.5)*ds);
            if (skipping) {
                let until=volume_empty_until(base,o,d,point);
                if (until>-0.5) {
                    let last=floor((until-0.02-interval.x)/ds-0.5);
                    k=max(k+1u,u32(clamp(last+1.0,0.0,f32(n))));
                    continue;
                }
            }
            optical_depth+=volume_density(base,point)*tverts[base+5u].w*ds;
            k++;
        }
    }
    return exp(-optical_depth);
}
//@incident-grid-begin
// Without light grids every lit domain's light is marched (w is 0). The shader with grids takes this out for
// volume_grid.wgsl's function (grid_source in pathtrace.rs), which reads them where they apply.
fn volume_incident_grid(p: vec3<f32>, outgoing: vec3<f32>, g: f32, shadows: bool, base: u32) -> vec4<f32> {
    return vec4(0.0);
}
//@incident-grid-end
// The events of volume_scattering's walk.
const VOLUME_DOMAIN: u32 = 0u;
const VOLUME_LIT: u32 = 1u;
const VOLUME_SLOT: u32 = 2u;
const VOLUME_CROSSING: u32 = 3u;
const VOLUME_STEP: u32 = 4u;
const VOLUME_ARRIVAL: u32 = 5u;
const VOLUME_SURFACE: u32 = 6u;
const VOLUME_ARRIVED: u32 = 7u;
const NO_DOMAIN: u32 = 0xffffffffu;
// The media at `point`: the light they emit and scatter towards `outgoing`, per unit length (rgb), and their
// extinction (w). With `single` a domain's base, only the light that domain scatters there, unweighted by its
// extinction and albedo. A lit domain's light comes from its grids where they apply, else from each light and the
// environment (the slots), dimmed by the media its shadow ray marches through (stepping over the cells the skip map
// leaves out) and by the surfaces in its way.
// Drivers inline every call, and a march nested in a march copies the sparse search into the inner one: NVIDIA's
// OpenGL compiler then takes gigabytes and minutes. So one loop walks every event (each domain's sample at the point,
// each slot, each domain its shadow ray crosses and each of its steps), and the density, the light and the surfaces
// in the way are each sampled from one place.
fn volume_scattering(point: vec3<f32>, outgoing: vec3<f32>, single: u32) -> vec4<f32> {
    var sigma=0.0; var source=vec3(0.0);
    var event=VOLUME_DOMAIN; var i=0u; var base=0u;
    // the lit domain: what its light is scaled by, its phase asymmetry, whether it is shadowed, the light so far
    var scatter=vec3(0.0); var g=0.0; var shadows=false; var light=vec3(0.0);
    // the slot: the direction and reach of its shadow ray, the light it brings, whether surfaces cast shadows, the phase
    let lights=u32(pp.ambient.w); let slots=lights+select(0u,1u,pp.env.x>0.5);
    var slot=0u; var dir=vec3(0.0); var reach=0.0; var arriving=vec3(0.0); var casts=false; var phase=0.0;
    // the shadow ray's march: the next domain, the one marched, its steps, their size, its entry, skipping, the
    // step and the optical depth so far
    var j=0u; var crossed=0u; var n=0u; var ds=0.0; var entry=0.0; var skipping=false; var k=0u; var depth=0.0;
    // the shadow ray's walk past surfaces (as visibility does): where it is, how far it has left, what gets through
    var at_surface=vec3(0.0); var remaining=0.0; var through=1.0; var crossings=0u;
    if (MEDIUM_LIGHTING && single!=NO_DOMAIN) {
        base=single; scatter=vec3(1.0); g=tverts[base+6u].w; shadows=tverts[base+13u].y>0.5; event=VOLUME_LIT;
    }
    loop {
        if (MEDIUM_LIGHTING && event==VOLUME_LIT) {
            let cached=volume_incident_grid(point,outgoing,g,shadows,base);
            light=cached.rgb; slot=select(0u,slots,cached.w>0.5); event=VOLUME_SLOT;
            continue;
        }
        if (MEDIUM_LIGHTING && event==VOLUME_SLOT) {
            if (slot>=slots) {
                source+=scatter*light;
                if (single!=NO_DOMAIN) { break; }
                i++; event=VOLUME_DOMAIN;
                continue;
            }
            if (slot<lights) {
                let lt=plights[slot];
                if (lt.pos.w<0.0 || light_lobes(lt).x==0.0) { slot++; continue; }
                // Authored ambient is already an isotropic local radiance approximation.
                if (lt.pos.w==0.0) { light+=lt.color.rgb; slot++; continue; }
                let sample=light_sample(lt,point);
                dir=sample.xyz; reach=max(0.0,sample.w-1e-3); casts=lt.size.y>0.5;
                arriving=light_radiance(lt,sample.xyz,sample.w); phase=volume_phase(dot(-sample.xyz,outgoing),g);
            } else {
                let z=2.0*rnd()-1.0; let a=2.0*PI*rnd(); let r=sqrt(max(0.0,1.0-z*z));
                dir=vec3(r*cos(a),r*sin(a),z); reach=1e30; casts=true;
                let q=normalize((pp.env_rot*vec4(dir,0.0)).xyz);
                let uv=vec2((atan2(q.x,q.z)+PI)/(2.0*PI),acos(clamp(-q.y,-1.0,1.0))/PI);
                arriving=textureSampleLevel(env_tex,smp,uv,0.0).rgb*pp.env.y;
                phase=4.0*PI*volume_phase(dot(-dir,outgoing),g);
            }
            j=0u; depth=0.0; event=select(VOLUME_ARRIVAL,VOLUME_CROSSING,shadows);
            continue;
        }
        if (MEDIUM_LIGHTING && event==VOLUME_CROSSING) {
            if (j>=pp.media.y) { event=VOLUME_ARRIVAL; continue; }
            crossed=volume_base(j); j++;
            let interval=volume_interval(crossed,point,dir,reach);
            if (tverts[crossed+13u].x<0.5 || interval.y<=interval.x) { continue; }
            n=max(1u,u32(ceil((interval.y-interval.x)/tverts[crossed+12u].w)));
            ds=(interval.y-interval.x)/f32(n); entry=interval.x; skipping=tverts[crossed+22u].w>0.5; k=0u;
            event=VOLUME_STEP;
            continue;
        }
        if (MEDIUM_LIGHTING && event==VOLUME_ARRIVAL) {
            at_surface=point; remaining=reach; through=1.0; crossings=0u;
            event=select(VOLUME_ARRIVED,VOLUME_SURFACE,shadows && casts);
            continue;
        }
        if (MEDIUM_LIGHTING && event==VOLUME_SURFACE) {
            // one surface the shadow ray meets, as one step of visibility's walk
            event=VOLUME_ARRIVED;
            if (crossings>=64u) { through=0.0; continue; }
            crossings++;
            let hit=trace(at_surface,dir,remaining);
            if (hit.tri==0xffffffffu) { continue; }
            let t=hit.tri;
            if (tmat_of(t)!=0xffffffffu) {
                let m=mats[hit_material(hit)];
                var opacity=m.base.a;
                if (m.texture_params.y!=0.0) {
                    let b=vec3(1.0-hit.u-hit.v,hit.u,hit.v);
                    let uv=tuv(t,0u,0u)*b.x+tuv(t,1u,0u)*b.y+tuv(t,2u,0u)*b.z;
                    let vertex_alpha=tverts[t*24u+2u].a*b.x+tverts[t*24u+8u].a*b.y+tverts[t*24u+14u].a*b.z;
                    opacity*=map_sample(m.maps[0],uv,m.borders[0]).a*vertex_alpha;
                    if (m.texture_params.y==1.0) { opacity=select(0.0,1.0,opacity>=m.texture_params.x); }
                }
                if (m.extra.y>0.5) { through*=1.0-clamp(opacity,0.0,1.0); }
                if (through<1e-5) { through=0.0; continue; }
            }
            let advance=hit.t+1e-3;
            at_surface+=dir*advance;
            remaining-=advance;
            if (remaining>0.0) { event=VOLUME_SURFACE; }
            continue;
        }
        if (MEDIUM_LIGHTING && event==VOLUME_ARRIVED) {
            var visible=1.0;
            if (shadows) {
                visible=exp(-depth);
                if (casts) { visible*=through; }
            }
            if (slot<lights) { light+=arriving*visible*phase; } else { light+=arriving*(phase*visible); }
            slot++; event=VOLUME_SLOT;
            continue;
        }
        var at=point; var sampled_base=base;
        if (MEDIUM_LIGHTING && event==VOLUME_STEP) {
            if (k>=n) { event=VOLUME_CROSSING; continue; }
            at=point+dir*(entry+(f32(k)+0.5)*ds);
            if (skipping) {
                let until=volume_empty_until(crossed,point,dir,at);
                if (until>-0.5) {
                    let last=floor((until-0.02-entry)/ds-0.5);
                    k=max(k+1u,u32(clamp(last+1.0,0.0,f32(n))));
                    continue;
                }
            }
            sampled_base=crossed;
        }
        if (event==VOLUME_DOMAIN) {
            if (i>=pp.media.y) { break; }
            base=volume_base(i); sampled_base=base;
        }
        let sampled=volume_sample(sampled_base,at,event==VOLUME_DOMAIN);
        if (MEDIUM_LIGHTING && event==VOLUME_STEP) {
            depth+=sampled.x*tverts[crossed+5u].w*ds; k++;
            continue;
        }
        let density=sampled.x;
        if (density<=0.0) { i++; continue; }
        let extinction=density*tverts[base+5u].w;
        sigma+=extinction; source+=density*sampled.yzw;
        let albedo=tverts[base+6u];
        if (MEDIUM_LIGHTING && any(albedo.rgb>vec3(0.0))) {
            scatter=extinction*albedo.rgb; g=albedo.w; shadows=tverts[base+13u].y>0.5; event=VOLUME_LIT;
            continue;
        }
        i++;
    }
    return vec4(source,sigma);
}
// Returns premultiplied radiance and surviving transmittance (not opacity).
fn volume_transport(o: vec3<f32>, d: vec3<f32>, distance: f32) -> vec4<f32> {
    var color=vec3(0.0); var trans=1.0; var cursor=0.0;
    // At most two boundaries per domain. Split at every entry/exit so thin layers
    // cannot fall between samples and widely separated domains skip empty space.
    for (var boundary=0u; boundary<=pp.media.y*2u; boundary++) {
        var next=distance; var step=1e30; var occupied=false;
        for (var i=0u; i<pp.media.y; i++) {
            let base=volume_base(i); let interval=volume_interval(base,o,d,distance);
            if (interval.y<=interval.x || interval.y<=cursor) { continue; }
            if (interval.x>cursor) { next=min(next,interval.x); }
            else { next=min(next,interval.y); step=min(step,tverts[base+12u].w); occupied=true; }
        }
        if (next<=cursor) { break; }
        if (occupied) {
            let n=max(1u,u32(ceil((next-cursor)/step))); let ds=(next-cursor)/f32(n);
            for (var k=0u; k<n; k++) {
                let point=o+d*(cursor+(f32(k)+0.5)*ds);
                let media=volume_scattering(point,-d,NO_DOMAIN); let sigma=media.w; let source=media.rgb;
                let optical_depth=sigma*ds;
                let attenuation=exp(-optical_depth);
                // Series avoids cancellation at small optical depths, including vacuum emission.
                var weight=ds*(1.0-0.5*optical_depth+optical_depth*optical_depth/6.0);
                if (optical_depth>1e-3) { weight=(1.0-attenuation)/sigma; }
                color+=trans*source*weight; trans*=attenuation;
            }
        }
        cursor=next;
        if (cursor>=distance) { break; }
    }
    return vec4(color,trans);
}
