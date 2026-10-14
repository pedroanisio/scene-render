//! `ocean/@gravity` is a physical quantity, metres per second squared. The wave solver works in
//! the ocean's own units, so it converts through `physics/@pixelsPerMeter` and the scale of the
//! ocean node: the same physical water, written at another scene scale, makes the same waves
//! (P7-01).

use sr_eval::Evaluator;

/// The depths of the ocean `sea` at `t`, in the document's own units.
fn depths(xml: &str, t: f64) -> Vec<f64> {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    let ev = Evaluator::new(&doc, &Default::default()).unwrap();
    let frame = ev.evaluate(t);
    assert!(frame.problems.is_empty() && frame.failures.is_empty(), "{:?} {:?}", frame.problems, frame.failures);
    let sea = frame.nodes.iter().find(|n| &*n.id == "sea").unwrap().sim_ocean.as_ref().unwrap();
    sea.frame.cells.iter().map(|c| c.depth).collect()
}

/// A 16 m by 16 m sea, 4 m deep, one impulse; every length in the document is `k` times the
/// metric one, as written for `pixelsPerMeter="k"`, or as an ocean node of scale `s` would need.
fn sea(lengths: f64, ppm: f64, ocean_scale: f64) -> String {
    let l = lengths;
    let s = if ocean_scale == 1.0 {
        String::new()
    } else {
        format!(r#" scaleX="{o}" scaleY="{o}" scaleZ="{o}""#, o = ocean_scale)
    };
    format!(
        r#"<scene version="1.3"><project width="64" height="64" fps="10" duration="2"/><composition>
          <ocean id="sea" width="{w}" depth="{w}" cellSize="{c}" bottomDepth="{b}" dt="0.05"{s}>
            <waterImpulse time="0.2" radius="{r}" amplitude="{a}"/>
          </ocean></composition><physics gravityY="0" pixelsPerMeter="{ppm}" bounds="none"/></scene>"#,
        w = 16.0 * l,
        c = l,
        b = 4.0 * l,
        r = 4.0 * l,
        a = 0.8 * l,
    )
}

fn assert_same_water(reference: &[f64], other: &[f64], factor: f64) {
    assert_eq!(reference.len(), other.len());
    let moved = reference.iter().zip(other).filter(|(a, _)| ((**a) - 4.0).abs() > 1e-3).count();
    assert!(moved > 4, "the impulse must have reached the water ({moved} cells moved)");
    for (i, (a, b)) in reference.iter().zip(other).enumerate() {
        assert!((a * factor - b).abs() < 1e-6 * factor.max(1.0), "cell {i}: {a} * {factor} vs {b}");
    }
}

#[test]
fn gravity_is_metres_per_second_squared_whatever_pixels_per_meter_is() {
    // 1 scene unit = 1 m against 1 scene unit = 1 cm
    let metric = depths(&sea(1.0, 1.0, 1.0), 1.0);
    let scaled = depths(&sea(100.0, 100.0, 1.0), 1.0);
    assert_same_water(&metric, &scaled, 100.0);
}

#[test]
fn gravity_is_converted_through_the_scale_of_the_ocean_node() {
    // an ocean node that is 2 times larger in the world holds half the local units
    let metric = depths(&sea(1.0, 1.0, 1.0), 1.0);
    let halved = depths(&sea(0.5, 1.0, 2.0), 1.0);
    assert_same_water(&metric, &halved, 0.5);
}

#[test]
fn default_wave_speed_uses_physical_gravity_and_ocean_scale() {
    for (lengths, ppm, scale) in [(1.0, 1.0, 1.0), (100.0, 100.0, 1.0), (0.5, 1.0, 2.0)] {
        let base = sea(lengths, ppm, scale);
        let default = base.replace(
            "</ocean>",
            &format!(r#"<wave wavelength="{}" amplitude="{}" phase="0"/></ocean>"#, 8.0 * lengths, 0.1 * lengths),
        );
        let explicit =
            default.replace(r#"phase="0""#, &format!(r#"phase="0" speed="{}""#, (9.81f64 * 4.0).sqrt() * lengths));
        for (a, b) in depths(&default, 0.5).iter().zip(depths(&explicit, 0.5)) {
            assert!((a - b).abs() < 1e-10 * lengths.max(1.0), "{a} vs {b}");
        }
    }
}

#[test]
fn animated_ocean_pose_does_not_make_solver_units_depend_on_seek_order() {
    let xml = sea(1.0, 1.0, 1.0).replace(
        "</ocean>",
        r#"<animate property="scaleX"><key time="0" value="1"/><key time="1" value="2"/></animate></ocean>"#,
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = Evaluator::new(&doc, &Default::default()).unwrap();
    ev.evaluate(0.0);
    let g = ev.evaluate(1.0);
    let sea = g.nodes.iter().find(|n| &*n.id == "sea").unwrap().sim_ocean.as_ref().unwrap();
    assert_eq!(sea.frame.cells.iter().map(|c| c.depth).collect::<Vec<_>>(), depths(&xml, 1.0));
}
