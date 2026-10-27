//! The scale draft drawn, rule 1 (sr-core `srep/srep-0000-scale.md`): a camera with `near="0.1" far="1e19"` keeps
//! two surfaces one part in 1e4 apart in order at every decade under both depth mappings, and a logarithmic camera
//! draws the image the reversed one draws, with depth of field (which reads the depth buffer back as distance) and
//! with the prepass of ambient occlusion and reflections.

mod common;
use common::*;

const SIZE: u32 = 64;

fn document(depth: &str, composition: &str, camera: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.8"><project width="{SIZE}" height="{SIZE}" fps="24" duration="1" background="#000000FF"/><materials><material id="red" baseColor="#FF0000FF" unlit="true"/><material id="green" baseColor="#00FF00FF" unlit="true"/></materials><composition><camera id="eye" x="0" y="0" z="0" near="0.1" far="1e19" depth="{depth}" {camera}/>{composition}</composition></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn rendered(d: &sr_model::Document) -> Option<Rendered> {
    let r = render_times(d, &[0.0])?;
    assert!(
        r.stats.errors.is_empty() && r.stats.unsupported.is_empty(),
        "{:?} {:?}",
        r.stats.errors,
        r.stats.unsupported
    );
    Some(r)
}

/// A red sphere at distance `z` and a green one of the same radius one part in 1e4 behind it.
fn pair(z: f64) -> String {
    let r = 0.3 * z;
    format!(
        r#"<object3D id="near" primitive="sphere" radius="{r:e}" z="{z:e}" material="red"/><object3D id="far" primitive="sphere" radius="{r:e}" z="{:e}" material="green"/>"#,
        z * (1.0 + 1e-4) + r * 1e-4
    )
}

#[test]
fn two_surfaces_one_part_in_1e4_apart_keep_their_order_at_every_decade() {
    for depth in ["reversed", "logarithmic"] {
        for k in [1, 4, 8, 12, 16, 18] {
            let Some(r) = rendered(&document(depth, &pair(10f64.powi(k)), "")) else { return };
            let (mut red, mut green) = (0, 0);
            for c in &r.px {
                red += (c[0] > 0.5 && c[1] < 0.1) as u32;
                green += (c[1] > 0.1) as u32;
            }
            assert!(red > 300, "{depth} 1e{k}: {red} red pixels");
            assert_eq!(green, 0, "{depth} 1e{k}: the farther sphere shows through in {green} pixels");
        }
    }
}

#[test]
fn a_logarithmic_camera_draws_the_reversed_camera_s_image() {
    // a sphere in focus before one out of focus: the blur reads the depth buffer back as distance
    let scene = r#"<object3D id="a" primitive="sphere" radius="3" x="-2" z="30" material="red"/><object3D id="b" primitive="sphere" radius="200" x="400" z="2000" material="green"/>"#;
    // ambient occlusion and reflections draw the prepass, which writes the depth first
    for camera in [
        "",
        r#"depthOfField="true" fStop="1.4" focusTarget="a""#,
        r#"ambientOcclusion="true" screenSpaceReflections="true""#,
    ] {
        let Some(rev) = rendered(&document("reversed", scene, camera)) else { return };
        let log = rendered(&document("logarithmic", scene, camera)).unwrap();
        let worst = rev
            .px
            .iter()
            .zip(&log.px)
            .map(|(a, b)| (0..4).map(|i| (a[i] - b[i]).abs()).fold(0f32, f32::max))
            .fold(0f32, f32::max);
        assert!(worst <= 1.0 / 255.0, "{camera:?}: the images differ by {}", worst * 255.0);
        assert!(rev.px.iter().any(|c| c[1] > 0.2), "{camera:?}: the far sphere is drawn");
    }
}

/// The red pixels' centroid left and right of the frame's centre, in pixels.
fn centres(r: &Rendered) -> [[f64; 2]; 2] {
    let mut sum = [[0.0f64; 3]; 2];
    for (i, c) in r.px.iter().enumerate() {
        if c[0] > 0.5 {
            let (x, y) = ((i as u32 % SIZE) as f64 + 0.5, (i as u32 / SIZE) as f64 + 0.5);
            let s = &mut sum[(x > SIZE as f64 / 2.0) as usize];
            *s = [s[0] + x, s[1] + y, s[2] + 1.0];
        }
    }
    sum.map(|s| {
        assert!(s[2] > 10.0, "a sphere is drawn: {s:?}");
        [s[0] / s[2], s[1] / s[2]]
    })
}

#[test]
fn spheres_2_17_apart_at_1e20_are_where_a_camera_at_the_origin_puts_them() {
    // rule 4: positions are rebased about the eye in binary64, so a camera between two spheres 2^17 pixels apart at
    // 1e20 sees what a camera at the origin sees of spheres at ±2^16; binary32 world coordinates are 2^43 apart there
    let scene = |x: f64| {
        let xml = format!(
            r##"<scene version="1.8"><project width="{SIZE}" height="{SIZE}" fps="24" duration="1" background="#000000FF"/><materials><material id="red" baseColor="#FF0000FF" unlit="true"/></materials><composition><camera id="eye" x="{x:e}" y="0" z="-200000" near="10" far="1e7"/><object3D id="a" primitive="sphere" radius="16384" x="{:e}" material="red"/><object3D id="b" primitive="sphere" radius="16384" x="{:e}" material="red"/></composition></scene>"##,
            x - 65536.0,
            x + 65536.0
        );
        let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
        sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
    };
    let Some(want) = rendered(&scene(0.0)) else { return };
    let want = centres(&want);
    let far = scene(1e20);
    for t in [0.0, 0.5] {
        let r = render_times(&far, &[t]).unwrap();
        let got = centres(&r);
        for k in 0..2 {
            for c in 0..2 {
                assert!((got[k][c] - want[k][c]).abs() <= 0.5, "t {t}: {got:?} against {want:?}");
            }
        }
    }
}

#[test]
fn a_scene_moved_3e9_pixels_draws_the_image_it_draws_at_the_origin() {
    // the floating origin moves every position the pass draws: objects and their parents, the camera's look-at and
    // focus targets, lights with their shadows and look-at constraints, and simulated particles; along x and along
    // the view axis, where the focus target's depth is read
    let scene = |dx: f64, focus: &str| {
        let x = |v: f64| format!("{:e}", v + dx);
        let xml = format!(
            r##"<scene version="1.8"><project width="{SIZE}" height="{SIZE}" fps="24" duration="1" background="#000000FF"/><materials><material id="grey" baseColor="#B0B0B0FF" roughness="0.6"/><material id="red" baseColor="#FF3030FF" unlit="true"/></materials><composition><camera id="eye" x="{}" y="-40" z="{}" near="1" far="2000" target="ball" depthOfField="true" fStop="1.4" focusTarget="{focus}"/><object3D id="floor" primitive="box" width="400" height="4" depth="400" x="{}" y="40" z="{}" material="grey"/><object3D id="ball" primitive="sphere" radius="25" x="{}" z="{}" material="grey"/><object3D id="back" primitive="sphere" radius="60" x="{}" y="-20" z="{}" material="grey"/><object3D id="moon" primitive="sphere" radius="8" parent="ball" x="45" y="-10" material="grey"/><particles3D id="dust" material="red" rate="0" size="6" x="{}" y="-30" z="{}" velocityX="10" lifetime="3" dt="0.1"><burst time="0" count="3"/></particles3D></composition><lights><light id="fill" type="ambient" intensity="0.2"/><light id="lamp" type="point" x="{}" y="-150" z="{}" intensity="40" range="2000" castShadow="true"/><light id="spot" type="spot" x="{}" y="-200" z="{}" spotAngle="30" intensity="2000"><transformConstraint type="look-at" target="moon"/></light></lights></scene>"##,
            x(0.0),
            x(-300.0),
            x(0.0),
            x(0.0),
            x(0.0),
            x(0.0),
            x(-80.0),
            x(400.0),
            x(-40.0),
            x(-20.0),
            x(60.0),
            x(-120.0),
            x(-200.0),
            x(-200.0)
        );
        let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
        sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
    };
    let Some(here) = rendered(&scene(0.0, "ball")) else { return };
    let there = rendered(&scene(3e9, "ball")).unwrap();
    let worst = here
        .px
        .iter()
        .zip(&there.px)
        .map(|(a, b)| (0..4).map(|i| (a[i] - b[i]).abs()).fold(0f32, f32::max))
        .fold(0f32, f32::max);
    assert!(here.px.iter().filter(|c| c[0] > 0.05).count() > 500, "the scene is drawn");
    assert!(worst <= 2.0 / 255.0, "the images differ by {} / 255", worst * 255.0);
}

#[test]
fn a_parented_camera_and_an_orbit_s_trail_move_with_the_scene() {
    // a camera parented to a rig and an orbiting sphere's trail, moved along x and along the view axis
    let ppm = 200.0 / 149_597_870_700.0;
    let scene = |dx: f64| {
        let xml = format!(
            r##"<scene version="1.8"><project width="{SIZE}" height="{SIZE}" fps="24" duration="1" background="#000000FF"/><materials><material id="white" baseColor="#FFFFFFFF" unlit="true"/></materials><composition><object3D id="rig" primitive="sphere" radius="1" x="{d:e}" y="20" z="{d:e}" visible="false"/><camera id="eye" y="-20" z="-400" near="1" far="5000"><transformConstraint type="parent" target="rig"/></camera><object3D id="sun" primitive="sphere" radius="1" body="10" x="{d:e}" z="{d:e}" visible="false"/><object3D id="sun-mark" primitive="sphere" radius="30" x="{d:e}" z="{d:e}" material="white"/><object3D id="earth" primitive="sphere" radius="6" material="white"><orbit around="sun" a="149597870700" e="0.3" i="90" argp="40"/><trail length="1e7" samples="64" width="4" color="#FF0000FF"/></object3D><object3D id="far" primitive="sphere" radius="120" x="{:e}" y="60" z="{:e}" material="white"/></composition><physics pixelsPerMeter="{ppm}"/><space epoch="2451545.0" timeScale="tdb" rate="2e7"/></scene>"##,
            dx - 150.0,
            dx + 900.0,
            d = dx
        );
        let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
        sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
    };
    // half a second in, the trail covers the last ten million seconds of the orbit
    let Some(here) = render_times(&scene(0.0), &[0.5]) else { return };
    let there = render_times(&scene(3e9), &[0.5]).unwrap();
    for r in [&here, &there] {
        assert!(
            r.stats.errors.is_empty() && r.stats.unsupported.is_empty(),
            "{:?} {:?}",
            r.stats.errors,
            r.stats.unsupported
        );
    }
    let worst = here
        .px
        .iter()
        .zip(&there.px)
        .map(|(a, b)| (0..4).map(|i| (a[i] - b[i]).abs()).fold(0f32, f32::max))
        .fold(0f32, f32::max);
    assert!(here.px.iter().filter(|c| c[0] > 0.5 && c[1] < 0.2).count() > 20, "the trail is drawn");
    assert!(here.px.iter().filter(|c| c[1] > 0.5).count() > 30, "the spheres are drawn");
    assert!(worst <= 2.0 / 255.0, "the images differ by {} / 255", worst * 255.0);
}

#[test]
fn the_air_is_the_same_either_side_of_a_floating_origin_cell_s_boundary() {
    // a camera 5 pixels either side of a boundary between the floating origin's cells at 2^20 (rebased about two
    // origins 1024 apart) sees one wall through one physical sky's air and fog, whose altitudes are the world's:
    // sideways through the air's march, and up through the fog's closed form at 1000 pixels a metre (2^20 pixels, 1 km)
    let scene = |x: f64, y: f64, sky: &str, ppm: f64, d: f64| {
        let xml = format!(
            r##"<scene version="1.8"><project width="{SIZE}" height="{SIZE}" fps="24" duration="1" background="#000000FF"/><materials><material id="u" baseColor="#808080FF" unlit="true"/></materials><composition><camera id="c" x="{x}" y="{y}" far="{}"/><object3D id="w" primitive="box" width="{}" height="{}" depth="10" x="{x}" y="{y}" z="{d}" material="u"/></composition><lights><light id="sky" type="dome" sky="physical" sunIntensity="1" date="2003-10-17T12:30:30-07:00" latitude="39.742476" longitude="-105.1786" moon="false" {sky}/></lights><physics pixelsPerMeter="{ppm}"/></scene>"##,
            d * 2.5,
            d * 0.75,
            d * 0.75
        );
        let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
        sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
    };
    // the boundary between the cells about 2^20 and 2^20 + 1024
    let edge = 1048576.0 + 512.0;
    let air = r#"fogDensity="0.0002" fogHeight="200""#;
    let fog = r#"atmosphere="none" fogDensity="0.0005" fogHeight="300""#;
    let cases: [(&dyn Fn(f64) -> sr_model::Document, &str); 2] = [
        (&|k| scene(edge + k, -300.0, air, 1.0, 8000.0), "sideways through the air"),
        (&|k| scene(0.0, -edge - k, fog, 1000.0, 8e6), "up through the fog"),
    ];
    for (at, what) in cases {
        let Some(short) = rendered(&at(-5.0)) else { return };
        let past = rendered(&at(5.0)).unwrap();
        let clear = short.at(SIZE / 2, SIZE / 2);
        assert!(clear[0] > 0.05, "{what}: the wall is drawn: {clear:?}");
        let worst = short
            .px
            .iter()
            .zip(&past.px)
            .map(|(a, b)| (0..3).map(|i| (a[i] - b[i]).abs() / a[i].abs().max(1e-3)).fold(0f32, f32::max))
            .fold(0f32, f32::max);
        assert!(worst <= 0.01, "{what}: the images differ by {} %", worst * 100.0);
    }
}

#[test]
fn a_volume_and_a_black_hole_are_the_same_either_side_of_a_floating_origin_cell_s_boundary() {
    // a smoke volume and a black hole's geodesics, with their cameras, 5 pixels either side of a boundary between the
    // floating origin's cells, draw what they draw at the document's origin
    let volume = |x: f64| {
        format!(
            r##"<composition><camera id="c" x="{x}" y="4" z="-20"/><object3D id="cloud" primitive="volume" x="{x}" y="4"><pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1" boundary="open" pressureIterations="20"><pyroSource shape="box" width="4" height="4" depth="2" densityRate="10"/></pyro><medium extinction="0.1" emissionColor="#FF4000" emissionScale="1" albedo="#000000"/></object3D></composition>"##
        )
    };
    let hole = |x: f64| {
        format!(
            r##"<composition><camera id="eye" x="{}" y="{}" z="0" target="hole" fov="40" pathSamples="4" geodesics="true"/><blackHole id="hole" mass="1" x="{x}"/><accretionDisk id="disk" blackHole="hole" outerRadius="16" temperatureScale="6000"/></composition>"##,
            x + 60.0 * 5f64.to_radians().sin(),
            -60.0 * 5f64.to_radians().cos()
        )
    };
    // the boundary between the cells about 2^20 and 2^20 + 1024
    let edge = 1048576.0 + 512.0;
    type Composition<'a> = &'a dyn Fn(f64) -> String;
    let cases: [(Composition, &str, f64); 2] = [(&volume, "the volume", 0.5), (&hole, "the black hole", 0.0)];
    for (composition, what, t) in cases {
        let doc = |k: f64| {
            let xml = format!(
                r##"<scene version="1.8"><project width="{SIZE}" height="{SIZE}" fps="10" duration="2" background="#000000FF"/>{}</scene>"##,
                composition(edge + k)
            );
            sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets())
                .unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
        };
        let Some(here) = render_times(&doc(-edge), &[t]) else { return };
        assert!(here.px.iter().filter(|c| c[0] > 0.05).count() > 20, "{what} is drawn");
        for k in [-5.0, 5.0] {
            let there = render_times(&doc(k), &[t]).unwrap();
            assert!(there.stats.errors.is_empty(), "{what}: {:?}", there.stats.errors);
            let worst = here
                .px
                .iter()
                .zip(&there.px)
                .map(|(a, b)| (0..4).map(|i| (a[i] - b[i]).abs()).fold(0f32, f32::max))
                .fold(0f32, f32::max);
            assert!(worst <= 2.0 / 255.0, "{what} at {k}: the images differ by {} / 255", worst * 255.0);
        }
    }
}
