# B1-2 metadata and deterministic codec workers

Base: `c0df479`. This implements the reviewed root-section design in
`docs/design/b1-2-styles-metadata.md`. Relative lengths and the rest of the
batch remain required work.

The scene owns the ten metadata attributes and ordered custom entries, bounded
to 256 entries, 128 name bytes and 4096 value bytes. Empty values are valid;
ASCII case-insensitive duplicate names fail the load. Source lines and exact
XML fingerprints are retained. The per-output embedding flag defaults to true.

MP4/MOV use explicit metadata tags without a timecode stream. Matroska keys
are canonicalized before collision checks and insertion. Reserved keys are
rejected against the selected final path before render setup or resume work.
Preview/hash operations and disabled embedding keep entries without applying
container restrictions. Full encoding and final resume assembly submit scene
tags; internal segments omit them. Every dictionary/option allocation is checked,
and post-header verification detects missing or changed values.

## Findings reproduced and resolved

Initial parser tests exposed the shared required-attribute helper's rejection
of empty values. Metadata now distinguishes an absent attribute from a present
empty string. The fixture's animation property was also corrected from `x` to
the existing registry name `position.x`. The initial build needed an explicit
diagnostics-header dependency; the corrected build emits no warnings.

Cross-thread metadata tests reproduced an existing encoder-policy gap: x264's
SEI recorded `threads=1` versus `threads=4`, and x265 also produced different
bytes. The old bit-exact test repeated the same thread count. Version 1.1 now
pins video codec workers and x265 pools to one, leaving render/scaler threads
independent and preserving the exact legacy policy for version 1.0/zero.

The read-only reviewer approved the version-scoped policy and identified two
necessary safeguards. A regression first showed old-policy cache segments being
reused (two reused instead of two rendered). The 1.1 manifest now includes
`codec_policy=1 threads=1`; removing it invalidates every segment. Another test
injected x265 option allocation failure and first observed a successful encoder
open. The option result is now checked and cleanly returns `SR_ERR_MEMORY`.

The reviewer also found that the integration shell's empty-value assertion
accepted a missing tag because `test` masked the probe's exit status. A negative
test using the actual `sr-probe` reproduced this. Standalone assignments under
`set -e` now preserve exit 3 for missing tags before comparing empty values.
The same negative test passes. The C round-trip tests also require key presence.

Final read-only review found no remaining production/design issues. Its last
draft-completion note was the missing new integration hash; that entry has been
added with an independent reference comparison. No previous hash was changed.

## Verification

All C builds and tests run in Flatpak SDK 25.08.

- Focused schema, metadata, encode, encode_faults, OOM and resume CTests: 6/6.
- Metadata parser: all attributes, custom/empty/Unicode/multiline values,
  exact limits, duplicates, source positions, unsupported contexts and 216
  fixed-seed mutations.
- Actual MP4/MOV/Matroska round trips, 1/4-thread and UTC/non-UTC byte identity,
  full/resumed output, final-path preflight, disabled embedding and segment
  omission. Tag edits invalidate reuse through the XML fingerprint.
- H.264/H.265/FFV1: 256x256, 24 varying frames at 1/4/automatic threads have
  identical container bytes, covering codec and scaler worker separation.
- Metadata loader OOM: 56 allocations; 50 `SR_ERR_MEMORY`, six loader OOM
  diagnostics, no leaks or silent successful fallbacks. Direct dictionary
  failures, muxer-success-with-tag-loss and x265 option OOM have fault tests.
- Integration preview `metadata`: SHA-256
  `0e6454a24e722ca6a63f5ad3d6e4b0190fa6ad81a8ef5c63e2081901da37e348`.
  It exactly matches the preserved `c0df479` binary rendering the same scene
  with metadata removed and version set to 1.0, at a different thread count.
  The 32x16 PNG was visually inspected. Existing references remain unchanged.
- Full Release, ASan/UBSan and coverage: 68/68 CTests each, including integration
  and frame order for all 18 goldens. Coverage is 91.56% lines / 74.44% branches;
  floors rise to 89.55% / 72.40%.
- Byte oracle against preserved `c0df479`: all 309 previews across 42 old
  fixtures, three full encodes and two expected rejections match.
- Strict performance check at 2% tolerance: failed, total 13.034 CPU seconds
  versus baseline 10.437 (+24.9%); all material stage deltas were marked noisy.
  A separate user render was still using roughly eight CPU cores. This does
  not clear the gate or establish its cause. Frame hashes matched, and the
  original baseline is unchanged. Stable remeasurement remains required.

Local logs: `/tmp/b1-metadata-targeted-2.log`,
`/tmp/b1-metadata-policy-{repro,fixed}.log`,
`/tmp/b1-metadata-empty-probe-repro.log`, `/tmp/b1-metadata-preview.log`, and
`/tmp/b1-metadata-{release,asan,coverage,oracle}-full.log`,
`/tmp/b1-metadata-coverage-floors.log` and `/tmp/b1-metadata-perf.log`.
