# B1-3 skew transforms

Skew adds bounded, animated `skewX`/`skewY` to implemented 2D node hosts.
The matrix is T R Kx Ky S T(-anchor); zero axes omit multiplication so the
legacy path retains its arithmetic. Ancestors, masks and card plane geometry
inherit skew. Stack-local context state carries the strict-transform policy
without modifying the scene. Soft-body rest poses use the same matrix order;
physics cache version 6 fingerprints both bases. Rigid solver geometry keeps
its previous policy. Camera and object3D do not acquire skew properties.

The new math uses no allocation or input-sized loop. Existing key storage
owns/frees both tracks; the golden XML is added to loader OOM replay. The
shared B1-3 graph/surface limits remain a separate required preparation phase
for all new compositing features, including direct-C scenes. They are not
claimed complete by this milestone.

## Review and reproduced findings

1. Copying the draw context initially lost the root lighting handoff, causing
   translucent 3D objects to draw twice. Existing independent pixel tests
   `depth.translucent_object_batches` and `translucent_sphere_crosses_box`
   failed before the fix (`/tmp/b1-skew-lighting-repro.log`). The consumed
   lighting pointer now propagates back to the caller; both tests pass.
2. A skewed card with `rotationY=1e308` silently disappeared when its
   projection became nonfinite. The new runtime test failed first
   (`/tmp/b1-skew-validation-repro.log`). Skewed card homographies, inverse,
   plane normal/depth, sorting pivot and transformed bounds are now checked;
   failures return SR_ERR_RENDER even without a diagnostic sink.
3. Static new attributes are legal in 1.0, but the new animation property
   vocabulary requires 1.1. A failing test reproduced acceptance of a 1.0
   skew animation (`/tmp/b1-skew-version-repro.log`). A registry flag now
   enforces the property version, while static attributes retain the
   proposal's exception. The design and XML reference state this distinction.

The first five focused suites passed after the fixes. The expanded eight-case
skew suite covers independent matrix and pixel references, inherited masks,
key/static/runtime bounds, singular and nonfinite cases, 96 seeded XML
mutations, soft-body free-fall offsets, cache value/version invalidation,
rigid sample identity, explicit-zero identity, scene immutability and
one/four-thread warm/shuffled renders.

Three new 320x180 golden frames at 0, 12 and 23 were visually reviewed. The
sheet covers both axes and order, animated skew, a nested mask, a projected
media card, particles and a soft-body rest pose. Existing references and the
integration hash manifest are unchanged.

## Verification

One added test initially assumed an orthographic card used projective bounds;
the full runs correctly rejected that expectation. The fixture now explicitly
installs a perspective camera to exercise that consumer, and retains a
separate assertion for the orthographic skip behavior. This correction changes
the test only, not renderer behavior.

- Full Release, ASan/UBSan and coverage each passed 74 of 75 CTests, with
  only that test-fixture assertion failing. The corrected eight-case skew
  suite then **passed in all three configurations**, giving passing evidence
  for all 75 tests in each configuration. All other renderer/source code is
  identical to the full runs. All 21 golden frame-order checks, integration
  and OOM replay passed in each full run, with no sanitizer report.
- Coverage after the corrected suite is **92.06% lines / 76.32% branches**.
  Raised floors **90.05% / 74.25%** pass using those same counters.
- The oracle against preserved `7f02ec7` passes **309/309 previews across
  42 fixtures, 3/3 encodes and 2/2 expected rejections**. Existing goldens,
  integration hashes and the owner performance baseline are unchanged.
- Independent follow-up review closed all three findings and found no
  additional actionable issue. Its verification conditions are now satisfied.
  Source and test review also covered the benchmark and documented
  pending graph/surface limits.

Full logs are `/tmp/b1-skew-{release,asan,coverage,oracle}.log`; corrected
suite logs are `/tmp/b1-skew-{release,asan,coverage}-corrected.log`.

The dedicated feature benchmark compares `tests/data-skew.xml` with exactly
ten static skew attributes and five skew key values set to zero. Independent
XML comparison confirms every other attribute and element is unchanged.
Both variants matched all 24 hashes independently at one/four threads and
across five alternating timing pairs. Timings ran only after all other build
and render jobs finished; all jobs are now terminal.

The strict **2% baseline performance gate passes** with clear -1.8%, lighting
-21.1%, composite -20.5%, viewport -22.9%, effects -18.6% and conversion -18.9%.
Stage-total CPU is 8.302 seconds versus baseline 10.437 (-20.5%). These noisy
measurements do not establish a stable speedup. The baseline SHA-256 remains
`215804d165a2bf161c79e044dab98a01f64564995c1259c1b43e0b30d2a853b4`.
The log is `/tmp/b1-skew-perf.log`.

The feature fixture measured median compositor CPU **0.024844 seconds versus
0.019914 (+24.8%)** over 24 frames at four threads. Ranges were
0.023712–0.026277 and 0.018682–0.020720 respectively. Stage-total CPU was
0.036994 versus 0.029502 seconds (+25.4%), with ranges 0.034770–0.037844 and
0.027314–0.030629. This includes changed pixel coverage as well as matrix
evaluation, so it is a fixture cost rather than a constant per-node overhead.
The log is `/tmp/b1-skew-feature-perf.log`.
