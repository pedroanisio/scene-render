//! Shutter sub-frames evaluated in parallel draw the same frame as the serial provider.
mod common;

use std::sync::atomic::{AtomicUsize, Ordering};

use sr_gpu::Renderer;

fn blurred() -> sr_model::Document {
    common::doc(
        r##"width="96" height="54" background="#000000" motionBlur="true" shutterAngle="180" motionBlurSamples="12" adaptiveMotionBlur="false""##,
        "",
        r##"<shape id="bar" shape="rect" width="6" height="54" y="0" fill="#ffffff">
              <animate property="x"><key time="0" value="4"/><key time="2" value="84"/></animate></shape>"##,
    )
}

#[test]
fn parallel_sub_frames_match_the_serial_ones_and_evaluate_each_time_once() {
    let Some(gpu) = common::gpu() else { return };
    let d = blurred();
    let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
    let p = ev.program();
    let g = ev.evaluate(0.5);
    let mut serial = Renderer::new(gpu.clone(), p);
    let frame = serial.render_with(&g, p, Some(&mut |t| ev.evaluate(t)));
    let expected = serial.read(&frame.texture);
    assert_eq!(frame.stats.subframes, 12, "every shutter sample is a sub-frame");
    assert!(frame.stats.subframe_seconds > 0.0);
    let smeared = (0..96).filter(|&x| expected[18 * 96 + x][0] > 0.01).count();
    assert!(smeared > 8, "the bar smears over its motion: {smeared} columns lit");

    let calls = AtomicUsize::new(0);
    let provider = |t: f64| {
        calls.fetch_add(1, Ordering::Relaxed);
        ev.evaluate(t)
    };
    let mut parallel = Renderer::new(gpu, p);
    let frame = parallel.render_with_parallel(&g, p, &provider);
    assert_eq!(parallel.read(&frame.texture), expected);
    assert_eq!(frame.stats.subframes, 12);
    assert_eq!(calls.load(Ordering::Relaxed), 12, "each sample time is evaluated once");
    assert!(frame.stats.subframe_seconds > 0.0, "the time spent evaluating is still measured");
}
