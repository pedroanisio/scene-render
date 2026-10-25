// Phase functions of the media (the media draft, section 4), pure functions of a medium record's row 30 (volume.rs):
// p = (g_hg, g_d, alpha, w), the mixture (1 - w) HG(g_hg) + w Draine(g_d, alpha). hg is w = 0, draine w = 1,
// rayleigh draine with g = 0 and alpha = 1, fog the fitted lobes. mu is the cosine between propagation directions.
// 1 + g^2 - 2 g mu written as (1 - g)^2 + 2 g (1 - mu), which keeps its digits at a forward peak (g near 1, mu near 1).
fn phase_hg(mu: f32, g: f32) -> f32 {
    let denom=(1.0-g)*(1.0-g)+2.0*g*(1.0-clamp(mu,-1.0,1.0));
    return (1.0-g*g)/(4.0*PI*denom*sqrt(denom));
}
fn phase_draine(mu: f32, g: f32, alpha: f32) -> f32 {
    let c=clamp(mu,-1.0,1.0);
    let norm=(1.0-g*g)/(4.0*PI*(1.0+alpha*(1.0+2.0*g*g)/3.0));
    let denom=(1.0-g)*(1.0-g)+2.0*g*(1.0-c);
    return norm*(1.0+alpha*c*c)/(denom*sqrt(denom));
}
fn volume_phase(mu: f32, p: vec4<f32>) -> f32 {
    if (p.w==0.0) { return phase_hg(mu,p.x); }
    if (p.w==1.0) { return phase_draine(mu,p.y,p.z); }
    return (1.0-p.w)*phase_hg(mu,p.x)+p.w*phase_draine(mu,p.y,p.z);
}
// The cumulative of Draine in mu from -1, in the form of sr_volume::medium::cdf_draine that never divides by g.
fn cdf_draine(mu: f32, g: f32, alpha: f32) -> f32 {
    let c=clamp(mu,-1.0,1.0);
    let a=1.0+g*g;
    let k=(1.0-g*g)/(2.0*(1.0+alpha*(1.0+2.0*g*g)/3.0));
    let s1=1.0+g; let s2=sqrt(max(a-2.0*g*c,0.0)); let q=s1*s2;
    let i0=2.0*(1.0+c)/(q*(s1+s2));
    let r=(c-1.0)*(3.0*a+q)*(a*(c-1.0)+2.0*g*c)/(a+q)+c*(3.0*a-q);
    let i2=2.0*(1.0+c)*r/(3.0*q*(s1+s2)*(a+q));
    return k*(i0+alpha*i2);
}
// SREP 79 rule 6's inversion.
fn sample_hg(u: f32, g: f32) -> f32 {
    if (abs(g)<1e-3) { return 1.0-2.0*u; }
    let s=(1.0-g*g)/(1.0-g+2.0*g*u);
    return clamp((1.0+g*g-s*s)/(2.0*g),-1.0,1.0);
}
// The mu with cdf_draine(mu) = u, by bisection of [-1, 1]: 32 halvings reach 5e-10, below f32's spacing near 1.
fn sample_draine(u: f32, g: f32, alpha: f32) -> f32 {
    var lo=-1.0; var hi=1.0;
    for (var i=0u; i<32u; i++) {
        let mid=0.5*(lo+hi);
        if (cdf_draine(mid,g,alpha)<u) { lo=mid; } else { hi=mid; }
    }
    return 0.5*(lo+hi);
}
// mu from draws in [0, 1): `lobe` picks the Draine lobe below w (rule 20) and is read only when 0 < w < 1, `u` inverts.
fn phase_sample(lobe: f32, u: f32, p: vec4<f32>) -> f32 {
    if (p.w==0.0 || (p.w<1.0 && lobe>=p.w)) { return sample_hg(u,p.x); }
    return sample_draine(u,p.y,p.z);
}
