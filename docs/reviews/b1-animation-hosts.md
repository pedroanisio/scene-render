# B1-1 material and audio animation review

Base: `0735d72`. Scope: the four existing material properties and audio-track
volume/pan, including XML contexts, property rows, ownership and consumers.

The guide §8 read-only reviewer inspected the implementation and tests for
determinism, frame-order state, thread safety, bounds, OOM paths, fingerprints
and static 1.0 output changes. No implementation findings were reported.

Material evaluation writes only render-owned object-frame values. All shadow
caster paths read the evaluated alpha. Audio evaluates immutable automation
at each absolute sample time and retains the static gain arithmetic. Scene
cleanup frees all added scalar/color tracks, including partial XML loads.
The complete XML already participates in the resume fingerprint; no new
external inputs or caches were added.

The named limits of 4096 materials and 4096 audio tracks apply to both
document versions. These are explicit resource-limit diagnostics for inputs
above the supported count; existing repository scenes remain below them.
Tests load both exact limits and reject the next host. Other parser tests
cover the 1.1 child gate, key bounds, wrong hosts, duplicate animation,
normalized audio spans, and 216 seeded valid/mutated/truncated cases.

At review time, focused Release and ASan suites and integration passed.
The three new material golden images were visually reviewed, and the golden
runner checked 1/4-thread identity and warmed frames. Integration verifies
24-frame lossless FFV1/PCM containers are identical at 1/4 threads and contain
96000 samples; all prior integration hashes are unchanged.

Full Release, ASan/UBSan and coverage CTest passed 66/66 each, including all
17 golden scenes through sequential, shuffled, sliced and interrupted/resumed
paths at 1/4 threads. The compatibility oracle against the verified curve
checkpoint `0735d72` matched all 309 previews across 42 old-schema fixtures,
three encodes and two expected rejection diagnostics. Coverage is 91.21%
lines / 73.50% branches; floors rise to 89.20% / 71.40%.

The final test additions compare decoded lossless audio from frames [7,19)
against samples [28000,76000) of the full render, and compare transparent
animated materials against static zero alpha after another rendered time.
The follow-up read-only review reported no findings. These two suites pass
again in Release, ASan/UBSan and coverage.

Verification logs are `/tmp/b1-hosts-{release,asan,coverage}-full.log`,
`/tmp/b1-hosts-oracle.log`, and `/tmp/b1-hosts-coverage-final.log`. Final test
extensions have separate `extra-test.log` files in the same prefix family.
The strict 2% fixed-baseline performance run failed: median total CPU was
23.596 seconds versus 10.437 (+126.1%), with every measured stage marked
noisy. A separate user animatic render was active at roughly eight CPU cores;
our functional test jobs had finished. This run cannot isolate this slice's
cost, and it does not clear the gate. The original baseline is unchanged.
Details: `/tmp/b1-hosts-perf.log`; remeasure/profile under stable conditions.

Performance remains open. This slice does not mark B1-1 complete;
gradient/new-node animation contexts land with B1-4/B1-5.
