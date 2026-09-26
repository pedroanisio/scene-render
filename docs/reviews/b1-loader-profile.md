# B1-0 embedded schema and capability review

Item: B1-0, following the separately verified table dispatcher refactor.
Root sections that require B1-2/B1-4/B1-5/B1-6 remain explicitly gated until
those dependent implementations land. This is infrastructure evidence, not
completion of the full batch.

Read-only Codex review checked determinism, frame-order state, thread safety,
input bounds, OOM, fingerprints and behavior of 1.0 documents.

## Findings and regressions

1. XSD fixed values were omitted from enum comparisons. Reproduced with a
   1.0 scene360 layout="cubemap" before fixing inventory expansion. The
   baseline's fixed equirectangular value now participates in the version
   gate. Root version itself has an explicit compatibility exception.
2. Unsupported URI attributes could disclose authentication information.
   Reproduced with a credential-bearing relative media URI. Unsupported
   xs:anyURI values are now wholly redacted in capability diagnostics.
3. String-backed semantic vocabularies escaped capability checks. Reproduced
   with rigidBody shape="capsule". The profile now includes known implemented
   and future rigid-body shapes, plus modifier axes. Unknown vocabulary
   members remain invalid, distinct from known unsupported members.

All three regressions are in `unit.profile`. The follow-up review found no
remaining issue in these fixes or the generated capability rows.

## Independent defects exposed during verification

The inherited compositor forced 33 specializations even at -O0, retaining
unreachable constant branches in every copy. Guarding `always_inline` with
`__OPTIMIZE__` preserves the Release path and restores useful shared-function
coverage in unoptimized builds. Review accepted the correction; coverage
floors stay at 88.30% lines / 69.29% branches.

The SDK's x265 4.2 leaves its reorder-delay time uninitialized when flushed
before a third input frame. The larger embedded schema changed heap layout
and exposed this in the existing two-frame H.265 integration fixture.
Packet tracing showed reference DTS=PTS and current DTS as an invalid large
negative number. The [upstream encoder implementation](https://github.com/videolan/x265/blob/master/source/encoder/encoder.cpp)
initializes the delay only when the delayed input POC arrives.

`unit.encode_faults.short_hevc_timestamps` deterministically poisons the
returned DTS for one- and two-frame HEVC streams. Both cases failed before
the correction. At flush, these streams now use DTS=PTS, because they have
no reordered pictures. Longer streams keep encoder timestamps. The focused
fault suite and integration test pass. Follow-up review found no issue in
scope, allocations, shared state or behavior of longer streams.

## Verification

Final full-suite, oracle, performance and coverage results are recorded in
`docs/schema-1.1-batch1-status.md` when each command completes. Existing
golden images and integration hash references have not been changed.
