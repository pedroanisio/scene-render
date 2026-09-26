# Loader dispatcher refactor review

Base: `5fc5e41` (pre-batch renderer source `5b7dca1`).

The dispatcher is now a context table. Emitters push `E_PARTICLES`; their
color tracks still route to the same fields. Existing
`anim_color.xml_color_keys` reproduced the stack-kind color regression,
which was fixed before verification.

Read-only Codex review found no remaining dispatcher issues. Review of the
verification tools identified suppressed recovering sanitizer reports,
missing alpha in PPM comparisons, only one frame-order thread count, and
acceptance of any interrupted exit. All four are fixed. The verification
regressions fail on the original scripts (9 failures) and pass on the fixed
scripts. Frame-order now compares 1 and 4 threads to a shared reference and
requires SIGKILL from the existing test hook.

SDK 25.08 verification:

- Release: 56/56 CTests pass.
- ASan/UBSan (`detect_leaks=0`): 56/56 CTests pass.
- Equivalence: 309/309 RGBA PNG previews across the 42 XSD-valid fixtures,
  at 1/3/22 threads; 3/3 full encodes match the preserved pre-batch binary.
  Two of those fixtures deliberately cannot render (missing asset and
  forbidden DOCTYPE); their exit codes and diagnostics match instead.
- Existing 14 goldens and integration hashes remain unchanged.
- Coverage: lines 90.70%, branches 62.75%. Branch coverage fails the 69.29%
  gate and remains an open batch gate; the floor has not been lowered.
  Most uncovered outcomes are in the newly inherited band compositor.

No feature behavior is enabled by this refactor. Capability implementation
and further batch work remain in subsequent changes.
