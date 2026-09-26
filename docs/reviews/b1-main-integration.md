# Partial Batch 1 integration with main

This merge brings the cumulative B1 branch through `769d415` into main
`2a26923`. It does not complete Batch 1. Remaining requirements are recorded
in `docs/schema-1.1-batch1-status.md` and the compositing review notes.

Git merged without conflicts. Main's additional changes are `.gitignore`,
the asset generator/schema and the standalone scene validator. The renderer,
headers, build inputs, embedded schemas and baseline remain identical to
`769d415`; no engine rebuild or repeat timing run was needed for this merge.

## Integration correction

The standalone validator rejected valid style-token aliases in the new
`tests/data-styles.xml` fixture with twelve TOKEN-REF errors. It now resolves
aliases iteratively, permits forward references, detects cycles and missing
or malformed targets, and enforces the renderer's 64-hop limit. Unused token
graphs are checked; literal values are checked as colors when consumed by a
color/paint attribute. File reads now close their handles explicitly.

Regression tests first reproduced the alias failures, then passed after the
fix. Independent review found one P2 issue: whitespace before a reference
could turn it into a literal that the terminal color check incorrectly
accepted. A failing regression reproduced that issue before the fix; the
validator now rejects its color use while permitting the unused literal.
Follow-up review closes the finding; no remaining review findings.

## Verification

All commands ran in Flatpak SDK 25.08. Final Python verification passes
12 tests. Seven integration fixtures pass XSD and semantic validation:
blend colors, lengths, styles, metadata, animation hosts, skew and the styles
golden. Schema compatibility passes 42/42 legacy fixtures; the generated
feature matrix is current. Both Python tools parse and the asset schema is
valid JSON. No asset-generation network calls were made.

Logs: `/tmp/b1-merge-verification.log`,
`/tmp/b1-merge-validator-before.log`,
`/tmp/b1-merge-validator-review-before.log`, and
`/tmp/b1-merge-validator-final.log`.

Inherited engine evidence is in `b1-compositor-evaluation.md`: initial Release
and ASan/UBSan 82/82, affected suites rerun after final fixes, final coverage
82/82 at 92.25% lines / 77.25% branches (floors 90.30 / 75.25), and the final
oracle's 309 previews, three encodes and two expected rejections all match.
The final strict 2% performance gate passes; timing remains noisy. No golden
references, baseline or render arithmetic changed during integration.

The final renderer binary SHA-256 remains
`4565d99e3b26605ba247f5349d9d18b1cb28c0173a5671227976951d1c10d82d`.
