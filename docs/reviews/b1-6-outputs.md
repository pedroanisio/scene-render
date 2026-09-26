# B1-6 outputs: implementation evidence

Branch `b1-6-outputs`, worktree `/tmp/scene-render-outputs`, base `main`
`4106b84`. Spec: `docs/schema-1.1-batch1-proposal.md` §4 B1-6 (plus the
"multiple `output`" part of B1-0). Design: `docs/design/b1-6-outputs.md`.
All builds and tests ran in Flatpak `org.freedesktop.Sdk//25.08` (FFmpeg
7.1.3, SVT-AV1 3.1.2).

## Proposal bullets → commits

| Bullet | State | Commits |
|---|---|---|
| Several `output` elements, each with an `id` | Implemented; ids required when there is more than one; a 1.0 document keeps one output | `133a5b5` (loader), `875a1a6` (capabilities) |
| Outputs at project size/rate share one pass; each encoder consumes the same frames | Implemented: one composite per frame, per-sink conversion and encode in sink order on the writer thread | `5cb9daf`, `19c6302` |
| Different `width`/`height`/`fps` render in their own pass, no rescaling | Implemented; assets and physics are prepared per pass; the project is changed only between passes | `5cb9daf` |
| Per-output `start`/`end` | Implemented; frames between disjoint ranges of one pass are not rendered | `133a5b5`, `5cb9daf`, `19c6302` |
| prores (prores_ks + `proresProfile`), vp9 (libvpx-vp9), av1 (libsvtav1) | Implemented | `5cb9daf` |
| png-, tiff-, exr-sequence; gif; apng | Implemented (per-frame files written atomically; deterministic per-frame GIF palette; linear float EXR) | `5cb9daf` |
| Bit-exact check at 1 and 4 threads per codec; encoder threads pinned | Every new codec pins codec workers to 1 (SVT `lp=1`) in every document version; x264's 1.0 policy untouched. Checked by `outputs.codecs_thread_identity`, `outputs.sequences_and_slices` and the integration section (52 files `cmp` at 1 vs 4 threads) | `5cb9daf`, `875a1a6` |
| `container`, `keyframeInterval`, `bFrames`, `faststart`, `loopCount` | Implemented | `5cb9daf` |
| `poster`/`thumbnail` as PNG and JPEG; webp/avif unsupported | Implemented; `format="webp|avif"` stays "unsupported in this build" | `133a5b5`, `5cb9daf`, `875a1a6` |
| CLI selection, per-output paths, `--resume`, `--frame-range`, `--preview-frame` | `--output-id`, rules in `docs/xml-reference.md` "Outputs (1.1)" → "Selection and paths" | `5cb9daf`, `19c6302` |
| Resume fingerprint | `output=` and `libav=` manifest lines for 1.1 outputs; per-pass project size/rate in the existing line; 1.0 manifests unchanged | `5cb9daf` |
| Golden: one still per lossless codec | `tests/golden/outputs.xml` (3 references) + `outputs-f012.exr`; png/tiff/apng frames decode exactly to `outputs-f012.png`, the EXR equals its reference bit for bit | `875a1a6` |

Capability entries flipped (`schema/capabilities.json`, regenerated
`src/xml_capabilities_data.inc`, `docs/feature-matrix.md`):
`outputType/@id,container,width,height,fps,start,end,keyframeInterval,bFrames,faststart,loopCount,proresProfile`;
`outputType/@codec` = prores, vp9, av1, png-sequence, tiff-sequence,
exr-sequence, gif, apng; `outputType/@container` = mp4, mov, mkv, webm;
`outputType/@proresProfile` (all six); elements `outputType/poster`,
`outputType/thumbnail`; `stillType/@time,path,format,width,quality`;
`stillType/@format` = jpeg, png. The `scene/output` occurrence limit is
removed (the loader enforces `SR_MAX_OUTPUTS` and the 1.0 single output).

## Verification

- Release CTest: 89/89 (new: `unit.outputs`, 6 `cli.output_*`; `frame_order`
  now covers 22 goldens including `outputs.xml`, at 1 and 4 threads).
- ASan/UBSan CTest (`ASAN_OPTIONS=detect_leaks=0`): 89/89, final run at
  `19c6302`.
- Coverage build: 89/89; 92.81% lines / 78.32% branches; floors raised from
  90.30/75.25 to 90.80/76.30 (`83dfb26`). New modules: outputs.c 98.0/84.4,
  outputs_resolve.c 97.7/91.7, output_plan.c 95.3/88.3, gif_palette.c
  100/93.3, output_media.c 92.9/64.1, xml_outputs.c 88.9/74.3.
  (The gate's steps were run with `-j 6` build / `ctest -j4`, because the
  script uses every core on this shared machine.)
- Byte oracle, `tools/equivalence-oracle.sh /tmp/scene-render-b1-reference`:
  309/309 previews at threads 1/3/22 across 42 scenes, 3/3 encodes, 2
  expected rejections — after the encoder refactor, after the renderer
  refactor and on the final binary (`19c6302`).
- Existing goldens and existing `tests/golden.sha256` entries unchanged;
  six entries added (`outputs-png`, `-tiff`, `-exr`, `-apng`, `-gif`,
  `-poster`).
- Unit tests (`tests/unit/test_outputs.c`, 19 cases): sequence patterns and
  collisions; `F(t)` against exact integer arithmetic over 6 rates × 715
  times; container derivation; loader acceptance, 35 rejection cases, the 1.0 single-output rule and
  both limits; seeded parser fuzz (216 documents incl. truncations, all
  accepted ones planned); pass grouping and per-output `--frame-range`;
  CLI rules; median cut against an independent qsort reference and the
  exact ≤256-colour path; EXR conversion against `sr_color_decode`
  and the gamut matrix; 10 codec/container cases at 1 vs 4 threads;
  sequences sliced in reverse order equal full renders; every output of a
  shared pass equals the same output rendered alone; `--resume` for prores
  (same pictures), vp9, av1 (rerun reuses segments, identical bytes);
  stills (PNG poster equals the preview, JPEG size, slice rules); lossless
  codecs vs golden; GIF lossless at few colours; write failures (I/O error
  names the frame file; failing second sink on the writer thread); review
  regressions.
- OOM (`oom.outputs_survive_allocation_failures`): data-outputs load (92
  allocations), multi-output render serial (70) and with the writer thread
  (80; 10 slot-buffer failures fall back to byte-identical serial output),
  plan (15), palette (7); no leak, every injected failure SR_ERR_MEMORY
  (or the loader's out-of-memory SR_ERR_XML).
- Integration (`tests/run-integration.sh`): `tests/data-outputs.xml`
  (every construct) at 1 and 4 threads — 52 files identical; sr-probe of
  codec/size/frame count for all 11 kinds of file, audio codecs (opus in
  WebM, AAC in MOV), pixel formats; `--output-id` alone equals the shared
  pass; preview at the selected output's size; exit 2 for selection
  errors; `--resume` of the ProRes output with segment reuse.

### Visual review of the new goldens

`outputs-f000/012/023.png`: three blue-teal bands, the yellow sun moving
right with ease-in-out, the translucent red card rotating from 0° to about
86° (it overlaps the sun at frame 12, the overlap is orange), the white ring
and the green bar that grows to the right. `outputs-f012.exr` converted back
to sRGB with FFmpeg (`-apply_trc iec61966_2_1`) shows the same picture as
`outputs-f012.png`. The `data-outputs.xml` poster (frame 6) and the EXR frame
4 were also looked at (EXR displayed without the transfer looks darker, as
linear data should).

### Performance

`scripts/perf-check.py` against the unchanged `benchmarks/baseline.json`
failed at total +94.4% with every stage (including ones this lane does not
touch: clear, lighting, viewport) +55–110% — the machine was shared with
other lanes. Five alternating pairs of `perf-scene.xml` 0:30 encodes against
`/tmp/scene-render-b1-reference` gave CPU seconds ref/new 34.3/35.4,
30.0/25.4, 29.0/23.1, 19.5/24.1, 23.9/27.1 (median paired difference
+3.0%, spread ±20%): inconclusive, not a pass of the 2% gate. By design a
scene with one output runs one pass with one sink; the added per-frame work
is a loop over one sink and one range check. Baseline not updated.

## Codex reviews

- Design review (11 findings), folded into the design note before code
  (`6365c93`).
- Diff review of `4106b84..875a1a6`: 1 blocker, 7 major, 1 minor. All were
  reproduced in `outputs.review_regressions` (or unit/oom additions) and
  fixed in `19c6302`: out-of-bounds scan of a malformed `--output` pattern;
  sequence overlap through digit-extended prefixes; GPU conversion depending
  on the selection; skipped outputs aborting renders; disjoint ranges
  rendering the gap; `--output` changing a 1.1 container or voiding
  `faststart`; inherited sizes/rates bypassing limits; `crf="+18"` rejected
  (1.0 lexical forms restored); `SVT_LOG` set during encoder open (moved to
  process start). No second review round was run.

## Deviations and findings

- **`keyframeInterval` default.** The schema default (2 s) applies to vp9
  and av1 only. h264/h265 without the attribute keep the encoder default so
  existing encodes (1.0 and existing 1.1 fixtures) stay byte-identical.
- **Sequence patterns vs `xs:anyURI` (schema erratum candidate).** libxml2
  rejects `%d` and `%%` in `xs:anyURI` (invalid percent escapes); only
  `%0Nd` with N = 1–9 validates. The loader would accept `%d`, but documents
  must use `%0Nd`. Recorded in `docs/xml-reference.md`; a B3 §7 erratum
  (e.g. a dedicated pattern type) is for the orchestrator/owner.
- **Attributes that do not apply are errors**, including an authored
  `faststart` on Matroska/WebM (the schema default is `true` everywhere).
- **Stills:** PNG stills are 8-bit RGBA (the preview format); still time must
  lie inside the output's range; `marker` stays unsupported until B1-5
  markers land (flip `stillType/@marker` then).
- **EXR** is premultiplied linear light in the output gamut, not clamped.
- **SVT-AV1 `bitrate`** fails for frames below roughly 128×72 (SVT disables
  adaptive quantization there and then refuses rate control); documented,
  `crf` works at every size.
- **`--resume`** renders each output in its own pass (no sharing); gif, apng
  and sequences cannot resume.
- **Fingerprint:** libav version integers are recorded for 1.1 outputs;
  external libvpx/SVT-AV1 versions are not fingerprinted separately (the
  SDK pins them). 1.0 outputs get no library line (would invalidate existing
  parts directories); left to a batch-wide fingerprint policy.
- **GPU path untested:** the SDK has no OpenCL. By construction OpenCL
  conversion is only used for scenes with one 1.0 output.
- Still unsupported (by the proposal's scope or other batches): `layout`,
  `variant`, `destination`, `alpha`, `twoPass`, `maxFileSize`, `profile`,
  `level`, `maxBitrate`, `bufferSize`, `transfer`, HDR metadata, `captions`,
  `burnCaptions`, `audioLanguages`, `audio`, `representation`; codecs
  `dnxhr`, `webp`, `jpeg-sequence`, `audio-only`; containers `mxf`, `wav`,
  `m4a`, `mp3`.

## Files outside the lane's own modules

Shared files touched (merge attention): `include/scene_render/scene.h`
(SrCodec values appended, SrOutput fields, SrStill, `extra_outputs`),
`src/scene.c` (output init/free), `src/xml.c` (dispatch rows for output,
poster, thumbnail), `src/xml_internal.h` (E_STILL, ParseFrame.output),
`src/xml_resolve.c` (one call), `src/xml_elements.c` (output handler moved
out), `src/renderer.c` (passes, sinks, writer, stills), `src/encoder.c`,
`include/scene_render/encoder.h`, `src/resume.c`,
`include/scene_render/resume.h`, `include/scene_render/renderer.h`
(`output_ids`), `src/metadata.c/.h`, `src/cli_args.c`, `src/main.c`,
`schema/capabilities.json`, `src/xml_capabilities_data.inc`,
`docs/feature-matrix.md`, `CMakeLists.txt`, `Makefile`,
`tests/unit/test_main.c`, `tests/unit/harness.h`, `tests/unit/test_oom.c`,
`tests/unit/test_golden.c`, `tests/unit/test_args.c`,
`tests/unit/test_resume.c`, `tests/run-integration.sh`,
`tests/golden.sha256`, `tests/golden/README.md`, `docs/xml-reference.md`,
`docs/feature-tests.md`, `tools/coverage-gate.sh`.
