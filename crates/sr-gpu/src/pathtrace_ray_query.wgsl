// The nearest hit through the driver's acceleration structure (ray-tracing units where the GPU has them), in place of
// the walk over `nodes`. The top level holds a structure of triangles and one of boxes, a splat's or an instance's
// bounds, which `tri_hit` and `prototype_hit` intersect as the software walk does. Neither has inactive entries, so a
// primitive's record is `ray_records[custom data + primitive index]`: each top-level instance's custom data is the
// offset of its structure's part of the table.
@group(2) @binding(0) var scene_as: acceleration_structure;
@group(2) @binding(1) var<storage, read> ray_records: array<u32>;

fn trace(o: vec3<f32>, d: vec3<f32>, tmax: f32) -> Hit {
    var best: Hit;
    best.t = tmax;
    best.tri = 0xffffffffu;
    best.instance = 0xffffffffu;
    if (!(tmax > 1e-4)) { return best; }
    var q: ray_query;
    rayQueryInitialize(&q, scene_as, RayDesc(RAY_FLAG_NONE, 0xffu, 1e-4, tmax, o, d));
    while (rayQueryProceed(&q)) {
        let candidate = rayQueryGetCandidateIntersection(&q);
        let k = ray_records[candidate.instance_custom_data + candidate.primitive_index];
        var h = best;
        let committed = rayQueryGetCommittedIntersection(&q);
        if (committed.kind == RAY_QUERY_INTERSECTION_TRIANGLE) { h.t = min(h.t, committed.t); }
        let bound = h.t;
        if (tmat_of(k) == 0xfffffffeu) { prototype_hit(k, o, d, tmax, &h); }
        else { tri_hit(k, o, d, tmax, &h, 0xffffffffu); }
        if (h.t < bound) {
            best = h;
            // bound to a name first: naga 30 does not emit a field load in the call's argument (it fails validation)
            let t = h.t;
            rayQueryGenerateIntersection(&q, t);
        }
    }
    let committed = rayQueryGetCommittedIntersection(&q);
    if (committed.kind == RAY_QUERY_INTERSECTION_TRIANGLE) {
        var h: Hit;
        h.t = committed.t;
        h.u = committed.barycentrics.x;
        h.v = committed.barycentrics.y;
        h.tri = ray_records[committed.instance_custom_data + committed.primitive_index];
        h.instance = 0xffffffffu;
        return h;
    }
    // a box's hit, or none (`best` is then still the miss)
    return best;
}
