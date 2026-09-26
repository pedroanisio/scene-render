# B1-2 style tokens

Base: `160b8c1`, after the reviewed root-section design (`7a0adb6`) and
checked-color-parser prerequisite. This slice enables styles/token and token
forms on the 13 existing color-consumer contexts. Metadata, relative lengths,
text styles and deferred paint consumers remain separate work.

The schema/capability pass collects and resolves the scene-owned table before
Expat parses colors, allowing project backgrounds to refer forward. Aliases
resolve to terminal indices without changing the original XML. Exact source
bytes still determine the resume fingerprint. Render-time colors remain the
existing numeric values; rendering adds no token lookup, allocation or state.

## Tests and review

The new styles suite covers static and animated color hosts, source positions,
malformed/unknown references, cycles, duplicates, unsupported contexts, resource
limits and 216 fixed-seed mutations. Depth tests accept 64 and reject 65 alias
hops in forward, reverse and shuffled declaration orders, including memoized
targets. Render comparisons reuse scenes at frames 20, 0, 12 and 0 and compare
literal/token PNG bytes at 1/4 threads while checking token-table immutability.

The fixture forces table growth past 16 entries. OOM replay covers all 120
intercepted allocations: 65 return `SR_ERR_MEMORY` and 55 report a loader
out-of-memory error, with no leaks or successful fallbacks. Initial replay
exposed misplaced token cleanup in scene initialization; moving it into
`sr_scene_free` fixed partial-load cleanup and the complete replay passed.

The read-only reviewer found no production issues and one verification-tool
issue: synthetic output insertion before styles violated root schema order.
The new Python regression failed in six variants before the fix. Output
insertion now follows project, metadata, parameters, styles and colorManagement.
All eight root-order variants and the three Python verification-tool tests
pass. Follow-up review confirmed the finding is closed with no new findings.

The new `styles` golden uses token aliases for the existing material-animation
scene. Its three expected PNGs are copied byte-for-byte from the literal
references and were visually reviewed. Existing references and integration
hashes are unchanged. There are now 18 golden scenes.

## Verification

Builds and tests run in Flatpak SDK 25.08. Targeted Release schema/styles/golden/
OOM CTests passed 4/4. Full Release passed 67/67, including frame order for all
18 goldens. ASan/UBSan and coverage each passed 67/67 CTests, including all
18 golden frame-order checks. Coverage is 91.50% lines / 74.11% branches;
floors rise to 89.50% / 72.10%. The byte oracle against the preserved
`160b8c1` binary matched 309 previews across 42 old fixtures, three encodes
and two expected rejection diagnostics. Every verification process exited 0.

Performance: no render-path change. The batch's strict performance gate remains
open; the original baseline is unchanged and this slice does not claim the
gate passed. No main-checkout merge or push is part of this commit.

Local logs: `/tmp/b1-styles-targeted.log`,
`/tmp/b1-styles-{release,asan,coverage}-full.log`,
`/tmp/b1-styles-oracle-full.log`, and
`/tmp/b1-styles-frameorder-{repro,fixed}.log`.
