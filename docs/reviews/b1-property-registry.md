# B1-1 property registry refactor

The registry replaces the scene.c scalar/colour lookup chains and the
xml_elements.c host/property/bounds mappings without adding properties.
It stores host masks, property/static-attribute names, value types, offsets,
legacy key bounds, eligibility and activation flags.

Read-only Codex diff review found no issues. Offsets, host masks and bounds
match the original mappings. It explicitly checked camera zoom eligibility,
particle colorEnd activation, depth-card activation and the legacy modifier
fallback to its owning node's scalar properties. The registry introduces no
allocation, mutable global state, rendering state or new input file.

`unit.property` covers all host categories, cross-host/type rejection, lookup
without activation, explicit activation, aliases, bounds, registry ambiguity,
and an XML modifier-owner regression. Focused property/XML/profile/scene
tests pass.

Final verification of `0dd2068`, before any feature build:

- SDK Release CTest 65/65.
- SDK ASan/UBSan CTest 65/65 with `ASAN_OPTIONS=detect_leaks=0`.
- Coverage CTest 65/65, 90.81% lines / 72.39% branches; property.c 100%/100%.
- Byte oracle against the preserved loader binary: 309/309 previews at
  threads 1/3/22, 3/3 full encodes and both expected rejections match.
- All original golden images and integration hashes unchanged.

Registry lookup is confined to scene construction, so it adds no per-frame
work. The original fixed-baseline performance gate remains open as recorded
in the batch status; no noisy measurement has been used to replace it.
