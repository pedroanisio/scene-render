# scene-render v0.5.0

This release moves the engine to a new repository, https://github.com/pedroanisio/scene-render, and a new licence. The code and behaviour are those of v0.4.1.

## Unreleased

- Mesh-gradient points now honour animated box-relative `x` and `y` coordinates, defaulting independently to `col/(cols-1)` and `row/(rows-1)`. Geometry and colour share the Catmull–Rom surface; absent coordinates preserve existing pixels.
- Inline vector assets (primitives and paths) now honour `strokePosition` and `paintOrder` through the shape stroke implementation. No existing corpus, example or golden vector asset sets these attributes.
- Curved connectors use the shared SREP 58 32-sample length table and evaluate outline cuts, labels, gaps and trimmed endpoints on the true cubic. Curved output can change away from the symmetric apex, including `tests/corpus/valid/connector-points.scene.xml`; straight and orthogonal routes retain their geometry.
- Trim shape modifiers default to simultaneous, matching `shape/@trimMode`.
- Mesh-warp and soft-body grid dimensions share the schema's defensive 2–16 clamp before allocation.
- Text animator `easeHigh` and `easeLow` independently ease the high and low ends of range selection. `rotationX`, `rotationY`, and `zDepth` project complete glyphs through the layer's 3D camera, or the default 60-degree camera for ordinary 2D text. Authored easing and glyph depth/rotation can intentionally change output.
- Caption emphasis uses the active colour for karaoke/highlight/pop/enlarge and bold weight for other presets. WebVTT cue settings round-trip; percentage `line` and `position` place burned captions, and unsupported settings produce render notes.
- Caption tracks stack by `z` among top-level composition entries (default 1000000), preserving track document order on ties. Groups remain compositing units.
- Nodes, symbols, text animators and puppet pins share human labels in diagnostics, findings and checkpoint companion metadata: nonblank name, else id, else element index. IDs and resume keys remain stable. Inspect lists symbol labels. The JSON envelope is `schema_version` 1.1.0: a diagnostic may carry `label`.
- Lighting implements `falloff`: linear, quadratic, smooth (the schema default) and none. Omitted falloff now uses smoothstep instead of the previous fixed quadratic attenuation; use explicit quadratic to retain that curve.
- Pixel motion blur declares its amount and sample controls and respects the quality-tier sample budget (8 in draft), retaining the existing 64-tap maximum.
- Matte-choke implements position: positive amounts erode inside, dilate outside (the schema default), or average both results at center. Negative amounts reverse inside/outside. Existing sign-only chokes can retain their former erosion/dilation direction by setting position to inside.
- Stroke and outline implement softness with the schema default 0.1, including when omitted. At ordinary scale that default stays below the one-pixel blur threshold; scaled nodes can now have softer edges. Their declaration lists and matte-choke’s list advertise only controls the kernels use.
- De-esser gain supplies post-processing make-up gain. Limiter gain drives the input before limiting so the documented threshold remains the processed signal’s ceiling. Zero gain retains both filters’ previous processing.
- Ocean gravity is consistently m/s²: solver and default wave speed convert through `pixelsPerMeter` and the ocean's world scale; impact cavities use physical gravity. Ocean motion changes when world units differ from metres.
- Particle and flock sprites accept image sequences (looped by particle age), videos and generators (layer time), with sequence missing-frame policies and particle blend order preserved.
- `scene-render params FILE` prints parameter metadata and variant labels as JSON. Parameter diagnostics include labels alongside stable ids.
- Unknown chart kinds produce render errors instead of column charts. Stepping programs require explicit dimensions. Both fracture paths share the documented 0.3 energy default; the existing 2D mass floor remains `1e-6` kg.
- Render-report asset records carry the licence and credit of every asset, generated media included. Video layer records include source in/out timecodes at the asset frame rate, with drop-frame labels preserved.
- Image, video, audio and mesh proxies are selected through the representation resolver. Explicit names win; audio and image/video representations can also be selected by language tag, then primary language subtag.
- Representation dimensions and aspect ratios are checked (A08), as are declared audio duration, sample rate and channel count (A09). Layout retains the original asset dimensions; video display width includes its pixel aspect ratio.
- An audio track without a language takes its asset's, or the selected representation's, when tagging the encoded stream; audible video layers contribute theirs. Several languages remain `mul` and none `und`.
- Tracking-kind/data mismatches, and camera tracks used by 2D constraints or stabilisation, are rejected with E17. Planar tracks continue to supply their centre, rotation and scale to 2D constraints.

## Licence

- **From 0.5.0, scene-render is licensed under the PolyForm Noncommercial License 1.0.0** (`LICENSE`, unmodified from https://polyformproject.org/licenses/noncommercial/1.0.0, with its required notice in `NOTICE`). Noncommercial use is free. Commercial use needs a separate licence from Pedro Anisio Silva (`COMMERCIAL.md`).
- **Versions up to 0.4.1 were published under the MIT License.** Copies of those versions keep the MIT terms they were released with.
- **Contributions** are accepted under the contributor licence agreement in `CONTRIBUTING.md`, so they can be distributed under the noncommercial and the commercial licence.
- The Cargo `license` field is the SPDX identifier `PolyForm-Noncommercial-1.0.0`. `THIRD-PARTY-NOTICES.md` is unchanged except for its opening licence statement: the dependencies keep their own permissive licences.

## Repository

- The repository starts from a clean tree at v0.4.1 (commit `1306b0e6` of the previous repository). The earlier history is kept privately as the record of authorship and dates.
- `CHANGELOG.md` keeps every earlier release's notes. `tools/changelog.py` reads them from `tools/changelog-history.md`, because their tags belong to the previous repository.
- The map-tile requests' User-Agent names the new repository.
- Commits carry their author only, with no co-author trailers (`CONTRIBUTING.md`).

## Changes that can alter output or verdicts from v0.4.1

- None. Rendered frames, validation verdicts and the vendored schema (sr-core 1.7.0) are those of v0.4.1.

The Linux archive contains the executable, source commit identifier, capability manifest, README, licence, notice, commercial-licence page and third-party notices. Its executable comes from the successful CI run for the release commit and is checksum-verified and smoke-tested before packaging.

Verify the download with `sha256sum -c scene-render-v0.5.0-linux-x86_64.tar.gz.sha256`, extract it, then run `./scene-render --help`.
