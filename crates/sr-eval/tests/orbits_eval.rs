//! The orbits draft at the evaluator (rules 2, 3, 4 and 6): an `object3D` with an `orbit` stands at its central
//! object's place plus the orbit's state at the frame's time of the space clock, an `nbody` member at the
//! integrated system's state, and a `cluster` adds spheres named by its prefix; the space frame maps to the scene
//! as (x, −z, y) × pixelsPerMeter about the space's origin, and a frame evaluated alone equals the frame played to.

use sr_eval::value::Value;
use sr_eval::{Evaluator, FrameGraph, FrameNode};
use sr_sim::nbody::{integrate, integrate_adaptive, plummer, system, Integrator, System, MAX_STEPS};
use sr_space::orbits::{orbit, orbit_state, period, Vec3, AU_M, G, GM_SUN};

const PPM: f64 = 1e-9;

fn document(composition: &str, space: &str, fps: &str, duration: &str) -> String {
    format!(
        r##"<scene version="1.8"><project width="64" height="64" fps="{fps}" duration="{duration}" background="#000000FF"/><composition>{composition}</composition><physics pixelsPerMeter="{PPM}"/>{space}</scene>"##
    )
}

fn evaluator_of(xml: &str) -> Evaluator {
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    let doc = sr_model::load_str(xml, &opts).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

fn evaluator(composition: &str, space: &str) -> Evaluator {
    evaluator_of(&document(composition, space, "24", "2"))
}

fn frame(ev: &Evaluator, t: f64) -> FrameGraph {
    let g = ev.evaluate(t);
    assert!(g.failures.is_empty() && g.problems.is_empty(), "{:?} {:?}", g.failures, g.problems);
    g
}

fn node<'a>(g: &'a FrameGraph, id: &str) -> &'a FrameNode {
    g.nodes.iter().find(|n| &*n.id == id).unwrap_or_else(|| panic!("no node {id}"))
}

fn position(n: &FrameNode) -> [f64; 3] {
    ["x", "y", "z"].map(|k| match n.props.get(k) {
        Some(Value::Num(v)) => *v,
        other => panic!("{}: {k} is {other:?}", n.id),
    })
}

/// Rule 6: space metres to scene units.
fn scene(p: Vec3) -> [f64; 3] {
    [p[0] * PPM, -p[2] * PPM, p[1] * PPM]
}

fn plus(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[track_caller]
fn close(a: [f64; 3], b: [f64; 3], tol: f64) {
    assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol), "{a:?} {b:?}");
}

const SUN: &str = r#"<object3D id="sun" primitive="sphere" radius="4" body="10" x="10" y="20" z="30"/>"#;

#[test]
fn an_orbit_stands_at_its_centre_plus_its_state_at_every_frame() {
    // one period over 24 frames: the space clock runs a period per scene second
    let p = period(GM_SUN, AU_M).unwrap();
    let earth = r#"<object3D id="earth" primitive="sphere" radius="2"><orbit around="sun" a="149597870700" e="0.3" i="7" raan="10" argp="20" anomaly="30"/></object3D>"#;
    let ev = evaluator(&format!("{SUN}{earth}"), &format!(r#"<space epoch="2451545.0" timeScale="tdb" rate="{p}"/>"#));
    let o = orbit(GM_SUN, AU_M, 0.3, 7.0, 10.0, 20.0, 30.0, 0.0).unwrap();
    for f in 0..=24 {
        let t = f as f64 / 24.0;
        let want = plus([10.0, 20.0, 30.0], scene(orbit_state(&o, t * p).unwrap().0));
        close(position(node(&frame(&ev, t), "earth")), want, 1e-9);
    }
    // the orbit closes: frame 24 is frame 0
    close(position(node(&frame(&ev, 1.0), "earth")), position(node(&frame(&ev, 0.0), "earth")), 1e-6);
}

#[test]
fn an_orbit_s_epoch_and_mu_and_a_centre_that_itself_orbits() {
    let rate = 86400.0;
    let objects = format!(
        r#"{SUN}<object3D id="earth" primitive="sphere"><orbit around="sun" a="149597870700" epoch="-43200"/></object3D><object3D id="moon" primitive="sphere"><orbit around="earth" mu="4.9e12" a="3.844e8" e="0.05"/></object3D>"#
    );
    let ev = evaluator(&objects, &format!(r#"<space epoch="2451545.0" timeScale="tdb" rate="{rate}"/>"#));
    let earth = orbit(GM_SUN, AU_M, 0.0, 0.0, 0.0, 0.0, 0.0, -43200.0).unwrap();
    let moon = orbit(4.9e12, 3.844e8, 0.05, 0.0, 0.0, 0.0, 0.0, 0.0).unwrap();
    for t in [0.0, 0.5, 1.25] {
        let g = frame(&ev, t);
        let e = plus([10.0, 20.0, 30.0], scene(orbit_state(&earth, t * rate).unwrap().0));
        close(position(node(&g, "earth")), e, 1e-9);
        close(position(node(&g, "moon")), plus(e, scene(orbit_state(&moon, t * rate).unwrap().0)), 1e-9);
    }
}

#[test]
fn a_centre_s_animated_place_is_its_place_at_the_frame() {
    let objects = r#"<object3D id="star" primitive="sphere"><animate property="x"><key time="0" value="0"/><key time="2" value="100"/></animate></object3D><object3D id="b" primitive="sphere"><orbit around="star" mu="1e20" a="1e10"/></object3D>"#;
    let ev = evaluator(objects, r#"<space epoch="2451545.0" timeScale="tdb" rate="0"/>"#);
    let o = orbit(1e20, 1e10, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0).unwrap();
    let g = frame(&ev, 1.0);
    // the centre's animated x, and its y and z as written
    let x = match node(&g, "star").props.get("x") {
        Some(Value::Num(x)) => *x,
        other => panic!("star: x is {other:?}"),
    };
    assert!(x > 1.0, "{x}");
    close(position(node(&g, "b")), plus([x, 0.0, 0.0], scene(orbit_state(&o, 0.0).unwrap().0)), 1e-9);
}

#[test]
fn orbits_around_each_other_are_a_failure_of_the_frame() {
    let objects = r#"<object3D id="a" primitive="sphere"><orbit around="b" mu="1e20" a="1e10"/></object3D><object3D id="b" primitive="sphere"><orbit around="a" mu="1e20" a="1e10"/></object3D>"#;
    let g = evaluator(objects, r#"<space epoch="2451545.0" timeScale="tdb"/>"#).evaluate(0.0);
    assert!(g.failures.iter().any(|f| f.contains("orbit") && f.contains("cycle")), "{:?}", g.failures);
}

/// The circular pair of the reference scaled to metres: masses 2e30 and 2e27 kg at 1 au.
fn pair() -> (String, System) {
    let (m0, m1) = (2e30, 2e27);
    let v = (G * (m0 + m1) / AU_M).sqrt();
    let (v0, v1) = (-v * m1 / (m0 + m1), v * m0 / (m0 + m1));
    let (x0, x1) = (-AU_M * m1 / (m0 + m1), AU_M * m0 / (m0 + m1));
    let members =
        format!(r#"<member ref="a" mass="{m0}" x="{x0}" vy="{v0}"/><member ref="b" mass="{m1}" x="{x1}" vy="{v1}"/>"#);
    let s = system(&[m0, m1], &[[x0, 0.0, 0.0], [x1, 0.0, 0.0]], &[[0.0, v0, 0.0], [0.0, v1, 0.0]], G, 0.0).unwrap();
    (members, s)
}

const AB: &str = r#"<object3D id="a" primitive="sphere"/><object3D id="b" primitive="sphere"/>"#;

#[test]
fn nbody_members_stand_at_the_integrated_state_under_each_integrator() {
    let (members, s) = pair();
    // 30 days a second at 24 fps: one step of 1.25 days a frame, coarse enough that the schemes differ
    let (rate, step) = (30.0 * 86400.0, 108000.0);
    let mut ends = Vec::new();
    for (name, integ) in
        [("leapfrog", Integrator::Leapfrog), ("wisdomHolman", Integrator::WisdomHolman), ("ias15", Integrator::Ias15)]
    {
        let space = format!(
            r#"<space epoch="2451545.0" timeScale="tdb" rate="{rate}"><nbody integrator="{name}" step="{step}">{members}</nbody></space>"#
        );
        let ev = evaluator(AB, &space);
        for f in [0u32, 1, 30] {
            let t = f64::from(f) / 24.0;
            let want = integrate(&s, integ, step, f64::from(f) * step).unwrap();
            let g = frame(&ev, t);
            close(position(node(&g, "a")), scene(want.positions[0]), 1e-9);
            close(position(node(&g, "b")), scene(want.positions[1]), 1e-9);
        }
        ends.push(position(node(&frame(&ev, 30.0 / 24.0), "b")));
    }
    // (and leapfrog is told apart from the others at this step; Wisdom-Holman is exact for a pair, as IAS15 is to
    // rounding)
    for (i, j) in [(0, 1), (0, 2)] {
        assert!((0..3).any(|k| (ends[i][k] - ends[j][k]).abs() > 1e-7), "{i} {j}: {ends:?}");
    }
}

#[test]
fn a_frame_alone_equals_the_frame_played_to() {
    let (members, _) = pair();
    for integrator in ["leapfrog", "ias15"] {
        let space = format!(
            r#"<space epoch="2451545.0" timeScale="tdb" rate="86400"><nbody integrator="{integrator}" step="150">{members}</nbody></space>"#
        );
        let played = evaluator(AB, &space);
        let mut last = [0.0; 3];
        for f in 0..=48 {
            last = position(node(&frame(&played, f as f64 / 24.0), "b"));
        }
        let alone = position(node(&frame(&evaluator(AB, &space), 2.0), "b"));
        assert_eq!(alone, last, "{integrator}");
        // and back again: an earlier frame after a later one
        assert_eq!(position(node(&frame(&played, 0.0), "b")), scene(pair().1.positions[1]), "{integrator}");
    }
}

#[test]
fn an_adaptive_nbody_follows_integrate_adaptive() {
    let (members, s) = pair();
    let space = format!(
        r#"<space epoch="2451545.0" timeScale="tdb" rate="86400"><nbody integrator="ias15" step="7" tolerance="1e-9">{members}</nbody></space>"#
    );
    let ev = evaluator(AB, &space);
    for f in [1u32, 7] {
        let (want, _) = integrate_adaptive(&s, f64::from(f) * 3600.0, 7.0, 1e-9, MAX_STEPS).unwrap();
        close(position(node(&frame(&ev, f64::from(f) / 24.0), "b")), scene(want.positions[1]), 1e-9);
    }
}

const EXCERPT_DIGEST: &str = "d113bb5117946244cf63de9d06287d5aac008acd1a561d28b760bcc7d4c3d1fd";

#[test]
fn a_member_s_body_gives_its_mass_and_its_ephemeris_state() {
    // the Earth and the Moon from the DE440 excerpt about the Earth, at the epoch
    let media = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus/media"));
    let space = r#"<space epoch="2451545.0" timeScale="tdb" origin="399" ephemeris="de"><nbody step="0.5"><member ref="a" body="399"/><member ref="b" body="301"/></nbody></space>"#;
    let xml = document(AB, space, "1", "1").replace(
        "<composition>",
        &format!(r#"<assets><ephemeris id="de" src="de440s-excerpt.bsp" digest="sha256:{EXCERPT_DIGEST}"/></assets><composition>"#),
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(media.clone()) })
        .unwrap_or_else(|e| panic!("{e}"));
    let ev = Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"));
    let orbiting = Evaluator::new(
        &sr_model::load_str(
            &xml.replace(
                "</composition>",
                r#"<object3D id="c" primitive="sphere"><orbit around="a" a="1e7"/></object3D></composition>"#,
            ),
            &sr_model::LoadOptions { verify_assets: true, base_dir: Some(media.clone()) },
        )
        .unwrap_or_else(|e| panic!("{e}")),
        &Default::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let spk =
        sr_space::Spk::open(&media.join("de440s-excerpt.bsp"), Some(&format!("sha256:{EXCERPT_DIGEST}"))).unwrap();
    let s = sr_space::Space::new("2451545.0", sr_space::TimeScale::Tdb, 1.0, 399, sr_space::Frame::Icrf).unwrap();
    let et = s.time_at(0.0).unwrap();
    let (r, v) = spk.state(301, 399, et).unwrap();
    let (gm_earth, gm_moon) = (sr_space::orbits::body_gm(399).unwrap(), sr_space::orbits::body_gm(301).unwrap());
    let start = system(
        &[gm_earth / G, gm_moon / G],
        &[[0.0; 3], r.map(|c| c * 1000.0)],
        &[[0.0; 3], v.map(|c| c * 1000.0)],
        G,
        0.0,
    )
    .unwrap();
    let g = frame(&ev, 0.0);
    close(position(node(&g, "b")), scene(start.positions[1]), 1e-12);
    // the Moon's distance at the epoch, 402448.64 km
    let b = position(node(&g, "b"));
    assert!(((b[0] * b[0] + b[1] * b[1] + b[2] * b[2]).sqrt() - 402448.64e3 * PPM).abs() < 1e-6, "{b:?}");
    let later = integrate(&start, Integrator::Leapfrog, 0.5, 1.0).unwrap();
    close(position(node(&frame(&ev, 1.0), "b")), scene(later.positions[1]), 1e-12);
    // an orbit about the member takes its body's GM
    let o = orbit(gm_earth, 1e7, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0).unwrap();
    let centre = scene(later.positions[0]);
    close(position(node(&frame(&orbiting, 1.0), "c")), plus(centre, scene(orbit_state(&o, 1.0).unwrap().0)), 1e-12);
}

#[test]
fn a_member_from_the_approximate_elements_moves_at_its_body_s_speed() {
    // the Sun about the Earth-Moon barycentre moves at the barycentre's speed about the Sun, 30.29 km/s two days
    // before the January perihelion; the light member's pull on it is negligible over 2 s
    let space = r#"<space epoch="2451545.0" timeScale="tdb" origin="3"><nbody step="0.5"><member ref="a" body="10"/><member ref="b" mass="1" x="1e12"/></nbody></space>"#;
    let ev = evaluator_of(&document(AB, space, "1", "4"));
    let (p0, p2) = (position(node(&frame(&ev, 0.0), "a")), position(node(&frame(&ev, 2.0), "a")));
    let speed = (0..3).map(|k| (p2[k] - p0[k]).powi(2)).sum::<f64>().sqrt() / PPM / 2.0;
    assert!((speed - 30.29e3).abs() < 50.0, "{speed}");
}

#[test]
fn an_orbit_about_a_member_takes_g_times_its_mass() {
    let (members, s) = pair();
    let objects = format!(r#"{AB}<object3D id="c" primitive="sphere"><orbit around="a" a="1e9"/></object3D>"#);
    let space =
        format!(r#"<space epoch="2451545.0" timeScale="tdb" rate="86400"><nbody step="150">{members}</nbody></space>"#);
    let ev = evaluator(&objects, &space);
    let o = orbit(G * 2e30, 1e9, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0).unwrap();
    let t = 3.0 / 24.0;
    let centre = scene(integrate(&s, Integrator::Leapfrog, 150.0, 3.0 * 3600.0).unwrap().positions[0]);
    close(position(node(&frame(&ev, t), "c")), plus(centre, scene(orbit_state(&o, 3.0 * 3600.0).unwrap().0)), 1e-9);
}

#[test]
fn a_cluster_adds_spheres_named_by_its_prefix() {
    let space = r#"<space epoch="2451545.0" timeScale="tdb" rate="0"><nbody step="150"><cluster count="40" mass="2e32" scale="1e12" seed="42" prefix="c" radius="0.5" x="1e12"/></nbody></space>"#;
    let ev = evaluator("", space);
    let g = frame(&ev, 0.0);
    let s = plummer(42, 40, 2e32, 1e12, G).unwrap();
    for i in [0usize, 3, 39] {
        let n = node(&g, &format!("c{i}"));
        assert_eq!(n.kind, "object3D");
        close(position(n), scene([s.positions[i][0] + 1e12, s.positions[i][1], s.positions[i][2]]), 1e-9);
    }
    assert!(g.nodes.iter().all(|n| &*n.id != "c40"));
    // the spheres are the cluster's radius
    let e = node(&g, "c3").elem.clone();
    let get = |k: &str| sr_model::element::Element::get_attr(&*e, k).map(|v| v.to_string());
    assert_eq!((get("primitive").as_deref(), get("radius").as_deref()), (Some("sphere"), Some("0.5")));
    // its drift: one step of 150 s a frame
    let ev =
        evaluator("", &space.replace(r#"rate="0""#, r#"rate="3600""#).replace(r#"x="1e12""#, r#"x="1e12" vx="1e5""#));
    let mut moved = s.clone();
    for (q, u) in moved.positions.iter_mut().zip(moved.velocities.iter_mut()) {
        q[0] += 1e12;
        u[0] += 1e5;
    }
    let want = integrate(&moved, Integrator::Leapfrog, 150.0, 150.0).unwrap();
    close(position(node(&frame(&ev, 1.0 / 24.0), "c3")), scene(want.positions[3]), 1e-9);
}

#[test]
fn a_cluster_body_named_like_another_node_is_an_error() {
    let space = r#"<space epoch="2451545.0" timeScale="tdb" rate="0"><nbody><cluster count="2" mass="1" scale="1" prefix="a" radius="1"/></nbody></space>"#;
    let xml = document(r#"<object3D id="a1" primitive="sphere"/>"#, space, "24", "1");
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: false, base_dir: None }).unwrap();
    let e = Evaluator::new(&doc, &Default::default()).expect_err("a1 is taken");
    assert!(e.to_string().contains("a1"), "{e}");
}
