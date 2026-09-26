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
tests pass. Final full-suite, coverage and oracle evidence will be appended
before any animation feature commit builds on this refactor.
