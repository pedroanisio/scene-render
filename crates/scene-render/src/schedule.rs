//! `schedule`: which adapter renders which frames of a path-traced document.
//!
//! A frame costs the path segments of every path-traced camera (`inspect`'s `path_costs`), and an adapter traces at a
//! rate in segments per second that the caller measured (an encode report's `paths.traced` over its wall time). With
//! rates, each frame goes to the adapter that would finish it first, so the last adapter finishes as early as it can,
//! and the frames are handed out as contiguous ranges. Without, an adapter's speed is unknown and every frame goes to
//! the best one: hardware with ray queries, then other hardware, then software.

use std::ops::Range;

/// One adapter as `gpus` reports it, in its ranked order, with the rate the caller measured on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Adapter {
    pub name: String,
    pub backend: String,
    pub software: bool,
    pub ray_query: bool,
    /// Path segments traced per second; `None` when not measured.
    pub rate: Option<f64>,
}

/// The frames one adapter renders.
#[derive(Debug, Clone, PartialEq)]
pub struct Share {
    /// Index into the adapter list.
    pub adapter: usize,
    pub frames: Range<u64>,
    /// Predicted trace time from the adapter's rate; `None` without a rate or a path-traced camera.
    pub seconds: Option<f64>,
}

/// Why no schedule could be made.
#[derive(Debug, Clone, PartialEq)]
pub enum ScheduleError {
    NoAdapter,
    /// The adapter's rate is not a positive, finite number.
    BadRate(String),
}

impl std::fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScheduleError::NoAdapter => write!(f, "no adapter was found"),
            ScheduleError::BadRate(name) => {
                write!(f, "the rate of {name} must be a positive number of segments per second")
            }
        }
    }
}

/// The adapters that count: one per physical device, as `gpus` lists the same device once per backend and ranks the
/// native backend first.
pub fn distinct(adapters: &[Adapter]) -> Vec<usize> {
    let mut keep: Vec<usize> = Vec::new();
    for (i, a) in adapters.iter().enumerate() {
        if !keep.iter().any(|&k| same_device(&adapters[k].name, &a.name)) {
            keep.push(i);
        }
    }
    keep
}

/// Whether two adapter names are one device: the same name, one name followed by `/` and the driver's suffix
/// (NVIDIA's OpenGL driver names the RTX 3050 that Vulkan calls `NVIDIA GeForce RTX 3050`
/// `NVIDIA GeForce RTX 3050/PCIe/SSE2`), or one name after `Mesa ` (Mesa's OpenGL driver names the GPU that its
/// Vulkan driver calls `Intel(R) Arc(tm) Graphics (MTL)` `Mesa Intel(R) Arc(tm) Graphics (MTL)`).
pub fn same_device(a: &str, b: &str) -> bool {
    let suffixed = |long: &str, short: &str| long.strip_prefix(short).is_some_and(|rest| rest.starts_with('/'));
    let mesa = |long: &str, short: &str| long.strip_prefix("Mesa ") == Some(short);
    a == b || suffixed(a, b) || suffixed(b, a) || mesa(a, b) || mesa(b, a)
}

/// Splits `frames` frames of `segments` path segments each across `adapters` (see the module documentation). The
/// shares are in rendering order and cover `0..frames` exactly once; an adapter without frames has no share.
pub fn schedule(adapters: &[Adapter], frames: u64, segments: u64) -> Result<Vec<Share>, ScheduleError> {
    if adapters.is_empty() {
        return Err(ScheduleError::NoAdapter);
    }
    if let Some(a) = adapters.iter().find(|a| a.rate.is_some_and(|r| !(r.is_finite() && r > 0.0))) {
        return Err(ScheduleError::BadRate(a.name.clone()));
    }
    if frames == 0 {
        return Ok(Vec::new());
    }
    // rendering order: hardware before software, ray queries first, then the faster, then the ranked order
    let mut order = distinct(adapters);
    let rate = |i: usize| adapters[i].rate.unwrap_or(0.0);
    order.sort_by(|&i, &j| {
        let (a, b) = (&adapters[i], &adapters[j]);
        (a.software, !a.ray_query).cmp(&(b.software, !b.ray_query)).then(rate(j).total_cmp(&rate(i))).then(i.cmp(&j))
    });
    let rated: Vec<usize> = order.iter().copied().filter(|&i| adapters[i].rate.is_some()).collect();
    if rated.is_empty() || segments == 0 {
        let seconds = adapters[order[0]].rate.filter(|_| segments > 0).map(|r| frames as f64 * segments as f64 / r);
        return Ok(vec![Share { adapter: order[0], frames: 0..frames, seconds }]);
    }
    // each frame goes to the adapter that would finish it first, which for frames of equal cost minimises the time the
    // last adapter finishes; a tie goes to the earlier adapter in rendering order
    let mut counts = vec![0u64; rated.len()];
    for _ in 0..frames {
        let mut best = 0;
        for k in 1..rated.len() {
            // (counts[k] + 1) / rate(k) < (counts[best] + 1) / rate(best), without dividing
            if (counts[k] + 1) as f64 * rate(rated[best]) < (counts[best] + 1) as f64 * rate(rated[k]) {
                best = k;
            }
        }
        counts[best] += 1;
    }
    let mut start = 0;
    let mut shares = Vec::new();
    for (k, &i) in rated.iter().enumerate() {
        if counts[k] == 0 {
            continue;
        }
        let end = start + counts[k];
        shares.push(Share {
            adapter: i,
            frames: start..end,
            seconds: Some(counts[k] as f64 * segments as f64 / rate(i)),
        });
        start = end;
    }
    Ok(shares)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(name: &str, backend: &str, software: bool, ray_query: bool, rate: Option<f64>) -> Adapter {
        Adapter { name: name.into(), backend: backend.into(), software, ray_query, rate }
    }

    fn covers(shares: &[Share], frames: u64) {
        let mut next = 0;
        for s in shares {
            assert_eq!(s.frames.start, next, "{shares:?}");
            assert!(s.frames.end > s.frames.start, "{shares:?}");
            next = s.frames.end;
        }
        assert_eq!(next, frames, "{shares:?}");
    }

    #[test]
    fn without_rates_every_frame_goes_to_the_hardware_adapter_with_ray_queries() {
        let list = [
            gpu("Big GPU", "Vulkan", false, false, None),
            gpu("RT GPU", "Vulkan", false, true, None),
            gpu("llvmpipe", "Vulkan", true, false, None),
        ];
        let s = schedule(&list, 120, 1_000).unwrap();
        assert_eq!(s, vec![Share { adapter: 1, frames: 0..120, seconds: None }]);
    }

    #[test]
    fn without_rates_hardware_beats_software_and_the_ranked_order_breaks_ties() {
        let list = [
            gpu("llvmpipe", "Vulkan", true, false, None),
            gpu("A", "Vulkan", false, false, None),
            gpu("B", "Vulkan", false, false, None),
        ];
        assert_eq!(schedule(&list, 10, 1).unwrap(), vec![Share { adapter: 1, frames: 0..10, seconds: None }]);
        let soft = [gpu("llvmpipe", "Vulkan", true, false, None)];
        assert_eq!(schedule(&soft, 10, 1).unwrap(), vec![Share { adapter: 0, frames: 0..10, seconds: None }]);
    }

    #[test]
    fn rates_split_the_frames_in_proportion_and_predict_the_time() {
        let list = [gpu("Slow", "Vulkan", false, false, Some(1.0e6)), gpu("Fast", "Vulkan", false, true, Some(3.0e6))];
        let s = schedule(&list, 100, 1_000_000).unwrap();
        covers(&s, 100);
        // the ray-query adapter renders first, then the other
        assert_eq!(s[0], Share { adapter: 1, frames: 0..75, seconds: Some(25.0) });
        assert_eq!(s[1], Share { adapter: 0, frames: 75..100, seconds: Some(25.0) });
    }

    #[test]
    fn equal_adapters_share_the_frames_left_over_in_rendering_order() {
        let list = [
            gpu("A", "Vulkan", false, false, Some(1.0)),
            gpu("B", "Vulkan", false, false, Some(1.0)),
            gpu("C", "Vulkan", false, false, Some(1.0)),
        ];
        let s = schedule(&list, 10, 7).unwrap();
        covers(&s, 10);
        let counts: Vec<u64> = s.iter().map(|x| x.frames.end - x.frames.start).collect();
        assert_eq!(counts, vec![4, 3, 3], "ties go to the ranked order");
    }

    #[test]
    fn an_adapter_without_a_rate_gets_no_frames_when_others_have_one() {
        let list = [gpu("Unmeasured", "Vulkan", false, true, None), gpu("Measured", "Vulkan", false, false, Some(5.0))];
        assert_eq!(schedule(&list, 8, 10).unwrap(), vec![Share { adapter: 1, frames: 0..8, seconds: Some(16.0) }]);
    }

    #[test]
    fn a_frame_stays_on_the_fast_adapter_when_the_slow_one_would_finish_it_later() {
        // measured shape: the RTX 3050 at 25 times llvmpipe's rate; one frame on llvmpipe outlasts all 24 on the RTX
        let list =
            [gpu("RTX", "Vulkan", false, true, Some(2.5e9)), gpu("llvmpipe", "Vulkan", true, false, Some(1.0e8))];
        let s = schedule(&list, 24, 13_271_040_000).unwrap();
        assert_eq!(s.len(), 1, "{s:?}");
        assert_eq!(s[0].frames, 0..24);
        // the 25th frame finishes at the same time on either, and the rendering order keeps it on the RTX; the 26th
        // finishes earlier on llvmpipe
        let s = schedule(&list, 25, 1).unwrap();
        assert_eq!(s.iter().map(|x| x.frames.clone()).collect::<Vec<_>>(), vec![0..25]);
        let s = schedule(&list, 26, 1).unwrap();
        assert_eq!(s.iter().map(|x| x.frames.clone()).collect::<Vec<_>>(), vec![0..25, 25..26]);
    }

    #[test]
    fn a_rate_too_small_for_one_frame_gets_no_share() {
        let list = [gpu("Fast", "Vulkan", false, false, Some(1.0e9)), gpu("Slow", "Vulkan", false, false, Some(1.0))];
        let s = schedule(&list, 4, 1_000).unwrap();
        assert_eq!(s, vec![Share { adapter: 0, frames: 0..4, seconds: Some(4.0e-6) }]);
    }

    #[test]
    fn a_document_without_path_traced_cameras_is_not_split() {
        let list = [gpu("A", "Vulkan", false, false, Some(1.0)), gpu("B", "Vulkan", false, true, Some(1.0))];
        assert_eq!(schedule(&list, 24, 0).unwrap(), vec![Share { adapter: 1, frames: 0..24, seconds: None }]);
    }

    #[test]
    fn the_same_device_on_two_backends_counts_once() {
        let list = [
            gpu("RTX", "Vulkan", false, true, Some(2.0)),
            gpu("RTX", "Gl", false, false, Some(2.0)),
            gpu("Other", "Vulkan", false, false, Some(2.0)),
        ];
        assert_eq!(distinct(&list), vec![0, 2]);
        // NVIDIA's OpenGL driver appends the bus and instruction set to the name its Vulkan driver gives
        let nvidia = [
            gpu("NVIDIA GeForce RTX 3050", "Vulkan", false, true, None),
            gpu("llvmpipe (LLVM 19.1.7, 256 bits)", "Vulkan", true, false, None),
            gpu("NVIDIA GeForce RTX 3050/PCIe/SSE2", "Gl", false, false, None),
        ];
        assert_eq!(distinct(&nvidia), vec![0, 1]);
        assert!(same_device("NVIDIA GeForce RTX 3050", "NVIDIA GeForce RTX 3050/PCIe/SSE2"));
        assert!(!same_device("NVIDIA GeForce RTX 3050", "NVIDIA GeForce RTX 3050 Ti"));
        let s = schedule(&list, 10, 1).unwrap();
        covers(&s, 10);
        assert_eq!(s.iter().map(|x| x.adapter).collect::<Vec<_>>(), vec![0, 2]);
    }

    #[test]
    fn mesa_opengl_names_the_device_its_vulkan_driver_names() {
        // as `gpus` lists an Intel Arc laptop: Mesa's OpenGL driver puts `Mesa ` before the name its Vulkan driver gives
        let arc = [
            gpu("Intel(R) Arc(tm) Graphics (MTL)", "Vulkan", false, true, Some(2.0)),
            gpu("llvmpipe (LLVM 22.1.8, 256 bits)", "Vulkan", true, false, None),
            gpu("Mesa Intel(R) Arc(tm) Graphics (MTL)", "Gl", false, false, Some(2.0)),
        ];
        assert_eq!(distinct(&arc), vec![0, 1]);
        assert!(same_device("Mesa Intel(R) Arc(tm) Graphics (MTL)", "Intel(R) Arc(tm) Graphics (MTL)"));
        assert!(same_device("Intel(R) Arc(tm) Graphics (MTL)", "Mesa Intel(R) Arc(tm) Graphics (MTL)"));
        assert!(!same_device("Mesa Intel(R) Arc(tm) Graphics (MTL)", "Intel(R) Arc(tm) Graphics"));
        assert!(!same_device("Mesa Intel(R) Arc(tm) Graphics (MTL)", "Mesa Intel(R) Arc(tm) Graphics"));
        // the device renders every frame, rather than half of them on each backend as if it were two
        assert_eq!(schedule(&arc, 10, 1).unwrap(), vec![Share { adapter: 0, frames: 0..10, seconds: Some(5.0) }]);
    }

    #[test]
    fn invalid_rates_and_empty_lists_are_errors() {
        assert_eq!(schedule(&[], 10, 1), Err(ScheduleError::NoAdapter));
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let list = [gpu("A", "Vulkan", false, false, Some(bad))];
            assert_eq!(schedule(&list, 10, 1), Err(ScheduleError::BadRate("A".into())), "{bad}");
        }
    }

    #[test]
    fn no_frames_gives_no_shares() {
        let list = [gpu("A", "Vulkan", false, false, Some(1.0))];
        assert_eq!(schedule(&list, 0, 1).unwrap(), vec![]);
    }
}
