# B1-3 compositing design review

The independent read-only reviewer audited the proposal, current renderer and
`docs/design/b1-3-compositing.md` before feature implementation. The primary
agent performed the second design pass. The neutral randomness prerequisite
received independent phase approval before its code was written; the complete
design is now approved for implementation. B1-3 capabilities remain gated.

The initial source audit identified the whole-parent extent of stencil
operations, particle flattening, private matte depth, card coordinate mapping,
adjustment backdrop ordering, coverage lifetime and bounded dependency work.
Those requirements are incorporated in the design. A separate discussion of
cross-parent mattes resolved placement in favor of authored transforms,
relative scopes, clocks and physics, with an explicit ancestor capture policy.

Two findings against the draft were corrected:

1. Pulling a backdrop into an adjustment-card plane and inverse-resampling it
   does not preserve identity. A half-pixel shift of a one-pixel checkerboard
   reproduced the false identity assertion in
   `/tmp/b1-adjustment-card-identity-repro.log`: the original alternating zero
   and one values became all 0.5. The design now preserves the original
   destination and projects only the weighted effect delta. Required tests
   cover fractional translation, scale, perspective and disabled effects.
2. A frozen star contour cannot represent independently animated inner and
   outer radii. The design now distinguishes immutable parsed path storage
   and polygon/star topology from bounded render-owned evaluated vertices.
   Omitted innerRadius follows half the evaluated outer radius only when no
   attribute or track is authored. Track bases, post-interpolation range
   checks, shuffled-time tests and scene-immutability checks are explicit.

The primary pass also corrected a surface budget that would have excluded
common 4K adjustment copies and specified one-time ancestor-opacity application
for captures. The review covers determinism, frame order, thread safety,
resource limits, OOM ownership, fingerprints and legacy bypass. Follow-up
review closes both draft findings with no remaining issues. This is design
approval, not evidence that any visible B1-3 feature is complete.

## Preparation lifecycle amendment

The follow-up interface audit found that arbitrary direct-C field edits,
complete preflight validation and no extra legacy per-frame traversal cannot
all be guaranteed automatically. Pointer-keyed caches miss edits; draw
counters skip inactive authored subtrees and run after recursive physics
collection. The primary pass and independent reviewer therefore selected an
explicit prepare/invalidate lifecycle for new B1-3 scene features.

The amendment requires XML preflight before recursive resolution and sorting,
and direct-C preparation before evaluation. Prepared authored state is
immutable until invalidation; callers invalidate before editing and prepare
again afterwards. No automatic detection of undeclared direct assignments is
promised. Failed or invalidated plans cannot be reused. Iterative structural
checks cover inactive nodes, aggregate counts, depth, cycles and duplicate
ownership; independent frame budgets cover evaluated work and surfaces.

Read-only follow-up review found no actionable contract gaps. Its wording
clarification, "invalidate before editing", is incorporated. Deliberately
invalid graph tests must restore an acyclic ownership tree before ordinary
scene cleanup. This is an approved implementation contract; the preparation
APIs and shared limits were still pending at that review. The structural
phase now implements the lifecycle and node/mask/depth preflight; its evidence
and remaining resource work are in `b1-compositing-preparation.md`.
