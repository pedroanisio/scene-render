---
disclaimer:
  notice: >-
    No information within this document should be taken for granted.
    Any statement or premise not backed by a real logical definition
    or verifiable reference may be invalid, erroneous, or a hallucination.
  generated_by: "AI-assisted"
  date: "2026-10-19"
---

# A drop falling into a glass of water

## Disclaimer

This work is subject to the methodological caveats and commitments described in [@DISCLAIMER.md](../../DISCLAIMER.md).
> No statement or premise not backed by a real logical definition or verifiable reference should be taken for granted.

Part of [scene-render](../../README.md).

`glass-of-water.scene.xml` drops a 4 mm drop into a square tumbler filled to 8 cm. It uses the engine's own 3D,
physics and water elements only, with no `effect`, `program` or shader. Use it as a starting point for small
transparent liquids. `crates/sr-model/tests/examples.rs` holds it to those elements and validates it.

```sh
scene-render render examples/glass-of-water/glass-of-water.scene.xml --time 0.2 -o splash.png
scene-render encode examples/glass-of-water/glass-of-water.scene.xml -o glass-of-water.mp4
```

`encode` resolves a relative `-o` against the document's directory, so the video lands next to the scene; pass
`--out-dir` or an absolute path to put it elsewhere.

## What each part is

- **Units.** One scene unit is one centimetre (`physics/@pixelsPerMeter="100"`), and y points down. The table top
  is at y = 0.
- **Glass.** A material with `transmission="1"` and `ior="1.5"`. A white `baseColor` keeps it clear, because the path
  tracer multiplies the light through it by `baseColor`. Any tint comes from `attenuationColor` and
  `attenuationDistance`. The tumbler is a base and four walls, each a closed `box`.
- **Water.** A material with `transmission="1"` and `ior="1.333"`. An `ocean` draws only the moving surface, so a
  `box` of the same material fills the glass beneath it.
- **Ripples.** The `ocean` fills the inside of the glass, 8 × 8 cm in 1 mm cells, with `boundary="closed"` so
  the walls reflect the waves.
- **Drop.** A sphere with a dynamic `rigidBody`, falling under the default gravity. The ocean lists it in
  `colliders`, and `<waterImpulse source="drop"/>` turns its entry into a cavity. The impulse names the drop and
  nothing else: the time, place and size come from the drop's motion.
- **Splash.** The ocean's `<whitewater>` launches spray wherever the surface is steeper than `threshold`, which near
  the cavity means just after impact. The spray beads use an opaque, slightly rough material, because at a
  millimetre across their highlight is what shows.
- **Light and camera.** A gradient-sky dome with no image file, a shadow-casting directional key, and a
  `renderer="pathtrace"` camera.

## Choices that keep it rendering cleanly

- **No coincident transmissive faces.** The path tracer holds one medium at a time. The water box therefore stops
  0.05 cm above the glass base, 0.01 cm inside the walls and 0.15 cm below the rest level of the surface.
- **The drop ends as it merges.** It is drawn until 0.135 s, under a centimetre into the water. Its rigid body
  still enters the water, and that entry is what the impulse reads.
- **Budgets fit the run.** At this cell size the solver takes many substeps, so `maxWork` on the ocean and
  `maxWork` and `maxMemoryMiB` on the whitewater are raised. Going past `maxParticles` is an error, not a cap.

## Limits

- The spray rises at one launch speed, carrying the surface's horizontal velocity. It reads as a burst of droplets
  over the cavity, not as a crown or a Worthington jet.
- The water's sides are flat faces of a box, so there is no meniscus where the water meets the glass.
- A square tumbler keeps every wall a `box`. A round glass needs a closed mesh asset, while the ocean's grid stays
  rectangular.
