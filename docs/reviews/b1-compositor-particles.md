# B1-3 particle resource accounting

This increment connects particle collector output, keyed walk scratch and the
scene-owned rate cache to the shared compositor budget. It does not complete
the broader compositing resource contract or enable another schema construct.
The reviewed policy is in `../design/b1-compositing-resources.md`.

## Implemented

Final preparation validates particle counts, finalized track storage, bounded
clock metadata and seed-id length; it aggregates authored rate-cache capacity
into prepared bytes, whether the cache is allocated or warm. Structural XML
preflight stays separate from final preparation. Invalidation clears caches
through the old plan inventory before edits; successful preparation clears newly
admitted caches after every bound passes and before publishing the plan. A
failed first preparation retains unpublished legacy caches for normal cleanup.

The cache remains one raw, mutex-protected scene allocation. The bounded
compositor evaluator charges its full zeroing and cold integration work on every
keyed evaluation, independent of prior frames. It carries the ledger through
collector growth, copied boundary totals, checkpoint marks and the fixed block
regeneration buffer. The compositor releases returned output after queuing
copied particle values; existing shared mask-copy lifetime is unchanged.

Lifetime-bound scans, seed scanning/hashing, cold keyed integration, deterministic
candidate visits, record copying/reversal and draw preparation are charged before
work. Actual allocations include prefixes and old-plus-new growth. Work/capacity
rejections are render failures; actual allocation failures remain memory errors.
The public NULL-ledger evaluator retains its existing arithmetic and behavior.

## Review and tests

Design review identified three closed-form paths that silently accepted an
unsupported emission index: static rate, before-first-key and after-last-key.
The CLI reproduced all three before implementation, returning success for a
1e16 rate: `/tmp/b1-particles-review-repro.log`. Bounded evaluation now returns
render failure with source line/element/attribute on all three; direct legacy
calls retain their prior result. Error reporting omits an overlong emitter id
rather than defeating its bounded scan with an unbounded diagnostic rescan.

Eight focused cases cover exact authored capacity differences, metadata and ID
limits, short diagnostics, independent cold-grid and per-candidate work
calculations, old/warm/shuffled one/four-thread output and exact quotas, extended
rate/lifetime/color tracks, invalidate/edit/shrink versus a fresh emitter,
closed-form index failures, cleanup after exhaustion and aggregate cache-byte
rejection/recovery without allocating gigabytes. The aggregate test borrows one
immutable finalized key array and restores sole ownership before destruction.

OOM replay covers **19 allocations** across cold cache, keyed walk scratch,
collector growth, queue arrays and shared masks. Every injected allocation
failure returns memory error and leaks nothing; cache cleanup in the replay
harness occurs outside rendering. The same compositor then renders successfully.
Focused logs: `/tmp/b1-particles-focused-final.log` and
`/tmp/b1-particles-extended.log`.

Production diff review found no additional issues. A clock-test precondition
finding was reproduced separately: the NaN assertion inherited an already
failed ledger (status 5 instead of a clean status 0). Evidence:
`/tmp/b1-particles-clock-repro.log`. The test now resets the ledger independently
for NaN and both signs of a time beyond the named limit, so earlier resource
exhaustion cannot mask missing clock validation.


Release and ASan/UBSan each pass **81/81 CTests**, including all 21 golden
frame-order cases, integration and OOM. The independently reset clock cases
also pass after the final rebuild in both configurations. The oracle against
the preserved `09a8427` executable matches **309 previews across 42 scenes,
three encodes and two expected rejections**. Renaming the two existing ceiling
macros to the `SR_MAX_PARTICLE_*` convention changes no arithmetic: the final
Release executable is byte-identical to the oracle-tested executable.

Logs: `/tmp/b1-particles-release-oracle.log`, `/tmp/b1-particles-asan.log`,
`/tmp/b1-particles-release-final.log` and `/tmp/b1-particles-asan-final.log`.
Follow-up review closes the independent-clock test finding; there are no
remaining review findings, subject to the verification gates.


Final coverage passes **81/81**, at **92.33% lines / 77.23% branches**.
Both floors are raised to **90.30% / 75.20%** and pass against the same final
counters. No sanitizer reports appear. Logs:
`/tmp/b1-particles-coverage-final.log` and `/tmp/b1-particles-coverage-floors.log`.

Performance investigation retains all results. The initial strict 2% gate
misses on clear (+2.3%, noisy; total -16.1%), and a nine-run gate also misses
on clear (+8.0%, noisy; total -11.8%). No clear-stage implementation changed.
Nine alternating runs per executable against `09a8427` measure clear
0.194503 to 0.190784 seconds (-1.91%; ranges 0.173612–0.219634 /
0.178299–0.208877), compositor 1.497711 to 1.579396 (+5.45%; ranges
1.294882–1.619702 / 1.354863–1.664895), and total 8.694092 to 8.858149
(+1.89%; ranges 7.652024–9.204180 / 7.930336–9.472446). The prior executable's
separate nine-run baseline control passes (clear -2.3%, total -18.6%). These
large overlapping ranges show material run-to-run variation; they do not prove
that the increment has no cost or that either version is faster.

Logs: `/tmp/b1-particles-perf.log`, `/tmp/b1-particles-perf-final.log`,
`/tmp/b1-particles-paired-perf.log` and `/tmp/b1-particles-perf-reference.log`.
The paired driver is `/tmp/b1-particles-paired-perf.py`. Every timing run starts
after preceding build/test/oracle/render jobs finish, with no competing renderer.
The owner baseline is unchanged. The final fixed 31-run gate also **fails**:
clear is 0.204 seconds versus 0.197 (+3.5%, noisy), above the 2% budget.
Lighting is -14.2%, compositor -7.2%, viewport -10.8%, effects -10.8% and
conversion -9.8%; total stage CPU is 9.325 versus 10.437 seconds (-10.7%).
Log: `/tmp/b1-particles-perf-31.log`. The performance gate remains open;
passing correctness checks and lower total CPU do not waive the clear-stage
failure. This checkpoint is not ready for merge, and the baseline was not
re-recorded to make the gate pass.

The feature-cost benchmark compares `09a8427` with this increment using five
alternating runs of 24 frames at four threads. Its particle fixture contains
eight animated-rate emitters in an isolated hard-light group. Every frame hash
matches before/after at one and four threads for both fixtures.

| Fixture | Compositor CPU before / after | Change | Total stage CPU change |
| --- | --- | --- | --- |
| Particle emitters | 0.023923 / 0.024614 s | +2.89% | +2.92% |
| Color blends | 0.181403 / 0.176619 s | -2.64% | -3.15% |

The particle compositor difference is about 0.029 ms per frame. Its ranges are
0.022398–0.024909 / 0.024353–0.028383 seconds; total ranges are
0.025002–0.028504 / 0.027634–0.032442. Color-blend compositor ranges are
0.170379–0.204837 / 0.170131–0.185066; total ranges are
0.175081–0.210108 / 0.174920–0.189845. These overlapping ranges do not
establish a stable speedup or eliminate the unresolved baseline failure.
Driver: `/tmp/b1-particles-benchmark.py`; log:
`/tmp/b1-particles-feature-perf.log`.

The functionally checked executable is preserved at
`/tmp/scene-render-b1-compositor-particles-checked`, SHA-256
`c89a96591b73adf510f247cfd28e8a965bdcaa20ca52af3ee0c99e94fe7e1f38`.
The unchanged owner baseline SHA-256 is
`215804d165a2bf161c79e044dab98a01f64564995c1259c1b43e0b30d2a853b4`.
No golden or integration hash changed. All verification jobs are terminal.

## Remaining integrations

Other animation evaluation, card sorting/bounds/warps, private effects/DOF and
lighting/shadow/fragment scratch, path raster scratch, renderer composition/
viewport/global-effect scope, and aggregate new authored/path ownership remain
incomplete. Advanced masks, remaining blend operators, dependency captures,
mattes and adjustments are still required, followed by B1-4 through B1-6 and
the complete batch audit. No complete resource-contract claim is made.
