# Feature matrix

Each feature maps to the module that implements it and to the tests that
verify it. Test names are `suite.case` of `sr-unit-tests` (CTest entry
`unit.<suite>`), `golden.<case>` (reference images in
`tests/golden/expected/`, see `tests/golden/README.md`), `oom.<case>`
(allocation-failure injection), `cli.<name>` CTest entries, or
`integration` (`tests/run-integration.sh`, whose `golden NAME` checks use
`tests/golden.sha256`). Module names are files under `src/`.

## Scene description and validation

| Feature | Implementation | Verified by |
|---|---|---|
| XSD validation with file:line/element/attribute diagnostics | `xml_schema` (libxml2, embedded `schema/scene-v1.xsd`) | `cli.validate_error`, `cli.invalid_scene_exit`, `cli.print_schema`, `xml.invalid_reports_line`, `integration` (xmllint cross-check of every example) |
| Expat loader, semantic checks, unique ids, reference resolution | `xml`, `xml_elements`, `xml_nodes`, `xml_visual`, `xml_camera`, `xml_audio`, `xml_physics`, `xml_resolve` | `xml.load_basic_multilayer`, `xml.duplicate_id_rejected`, `xml.doctype_rejected`, `xml.deep_group_nesting`, `fx.unknown_effect_id`, `text.xml_bounds`, `deform.point_validation`, `physics.soft_body_substeps_validated`, `cli.validate_ok` |
| Scene graph, `(z, XML order)` sorting, dynamic nesting | `scene` | `scene.sort_orders_by_z_then_order`, `scene.sort_empty_group`, `xml.deep_group_nesting` |
| Keyframes: step, linear, ease-in/out, ease-in-out, cubic Bézier | `timeline` | `timeline.curves`, `timeline.duplicate_key_rejected`, `golden.composite_blend_modes` (cubic-bezier key), `integration` (`golden keyframe`) |
| Color keyframes in linear light | `color`, `timeline` | `anim_color.*` (5 cases), `golden.effects_stack`, `golden.composite_blend_modes` |
| Integer frame time (`N × fps_den / fps_num`) | `renderer` | `particles.frame_independent`, `physics.softbody_deterministic`, `cli.hash_threads` |

## Assets and media

| Feature | Implementation | Verified by |
|---|---|---|
| Still images (PNG, JPEG, PPM, ... via libav; exact-size PPM/PNM, Lanczos otherwise) | `video` (`sr_image_decode_rgba8`), `assets`, `color` | `video.still_images`, `image.*` (3 cases), `golden.image_magnification` |
| Image sampling and magnified edges | `compositor`, `raster` | `image.magnified_edge`, `image.half_pixel_translation`, `image.identity_reproduces_pixels`, `golden.image_magnification`, `golden.deformers_mesh_warp` |
| Video decode: persistent decoder, keyframe seeking, 256 MiB per-asset frame LRU, stream matrices | `video`, `assets` | `video.sequential_matches_seeking`, `video.asset_frames_through_scene`, `video.declared_dimensions_must_match`, `video.vfr_never_shows_future_frames`, `video.input_matrices`, `encode_faults.video_open_faults`, `encode_faults.video_frame_faults` |
| Clip reuse, trim, loop, reverse, speed, stretch, `source.time` remap | `compositor` (media time), `assets` | `golden.video_time_remap`, `video.video_and_audio_share_origin`, `integration` (`examples/video-audio-remap.xml` validation) |
| Text: Fontconfig/`fontFile`, FriBidi, HarfBuzz shaping, wrap, justify, align, letter spacing, vertical alignment, FreeType raster | `text` | `text.*` (17 cases), `golden.text_layout_inter` (vendored Inter), `golden.text_scripts_fontconfig` (Hebrew/Arabic, skipped unless Fontconfig `sans` is the reference font) |
| Vector assets: rect, ellipse, SVG-subset paths, fill rules, strokes | `vector_path`, `procedural` | `path.*` (6 cases), `vector.triangle_coverage`, `vector.invalid_path_reports_error`, `golden.vector_paths` |
| Wavefront OBJ meshes | `mesh` | `mesh.load_octahedron_obj`, `golden.scene3d_shadows_mesh` |
| Audio decode (libav + swresample, in memory), timestamps, gaps | `audio` | `audio.mp4_priming_trimmed_once`, `audio.timestamp_gap_is_silence`, `encode_faults.audio_decoder_faults` |
| Audio mix: start, trim, loop, volume, equal-power pan, fades, speed, reverse; sample-exact frame blocks | `audio` | `audio.frame_to_sample_is_exact`, `audio.two_tracks_with_pan_and_fades`, `audio.reverse_and_speed_positions`, `audio.range_render_sample_count`, `audio.seconds_to_samples_saturates`, `integration` (A/V duration check) |

## Compositing and color

| Feature | Implementation | Verified by |
|---|---|---|
| Blend modes normal/add/multiply/screen/overlay/difference (W3C, premultiplied) | `raster`, `compositor` | `blend.*` (11 cases), `golden.composite_blend_modes`, `golden.groups_and_masks` |
| Shapes with anti-aliased fill and centred stroke | `raster`, `compositor` | `raster.*` (4 cases), `golden.composite_blend_modes`, `golden.vector_paths` |
| Isolated and pass-through groups, buffer pool | `compositor` | `group.*` (5 cases), `golden.groups_and_masks` |
| Masks: rect, ellipse, rounded-rect, inverted, several per node, animated | `compositor`, `raster` | `mask.*` (5 cases), `golden.groups_and_masks` |
| Working spaces sRGB/Rec.709/Display-P3/Rec.2020, linear light, output conversion (8/16-bit) | `color` | `color.*` (7 cases), `encode.yuv420p10le_path`, `encode.color_tags_follow_output`, `integration` (color tags, `golden production`) |
| Deterministic row-parallel work | `parallel`, `compositor`, `color`, `camera`, `effects` | `compositor.thread_invariant`, `color.convert_frame_thread_invariant`, `particles.thread_invariant`, `text.render_thread_invariant`, every `golden.*` case (1 and 4 threads), `cli.hash_threads`, `integration` (1 vs 4 thread `cmp`) |

## Effects, particles, 3D, physics, deformation

| Feature | Implementation | Verified by |
|---|---|---|
| Whole-frame effects: glow, bloom, box blur, color grade, vignette, lens flare | `effects` | `fx.unreferenced_effect_whole_frame`, `fx.blur_dirty_rect_expansion`, `fx.animated_effect_param`, `golden.effects_stack` |
| Group effects, drop shadow, masks after effects | `effects`, `compositor` | `fx.group_effect_only_inside_group`, `fx.drop_shadow_offset_colour`, `fx.group_mask_after_effects`, `fx.huge_shadow_offset_defined`, `golden.effects_stack` |
| 2D lighting effect: falloffs, spot cone, relief | `effects` | `fx.falloff_curves`, `fx.point_light_falloff_values`, `fx.relief_brightens_lit_side`, `fx.huge_light_stays_finite`, `fx.light_and_effect_bounds`, `golden.effects_stack` |
| Parametric particles, presets, animated emission, cap | `particles`, `compositor` | `particles.*` (8 cases), `golden.particle_emitters`, `golden.equirect_canvas` |
| 3D: sphere/box/plane sprites, OBJ triangles, depth buffer, materials, lights | `lighting`, `mesh` | `golden.scene3d_shadows_mesh`, `integration` (`golden production`, `golden advanced`) |
| Shadow maps (directional, spot), PCF, supersampling `antialias3d` | `lighting` | `shadow.*` (5 cases), `golden.scene3d_shadows_mesh` |
| Rigid bodies: OBB/circle contacts, friction, restitution, damping | `physics` | `physics.circle_box_contact`, `physics.heavy_damping_never_flips_sign`, `physics.divergence_is_an_error`, `golden.physics_rigid_soft` |
| Force fields (directional, radial, vortex), springs, distance and pin constraints | `physics` | `physics.vortex_tangential`, `physics.pin_holds_anchor`, `golden.physics_rigid_soft` |
| Soft bodies (mass-spring grid, pressure, pins, substeps) | `physics`, `deform` | `physics.softbody_pinned_top_sags_and_rests`, `physics.softbody_deterministic`, `physics.soft_body_substeps_validated`, `golden.physics_rigid_soft` |
| Physics cache (versioned, signature-checked, atomic) and `--physics-cache` | `physics` | `physics.softbody_cache_round_trip`, `physics.cache_payload_validated`, `cli.physics_cache`, `integration` (`golden advanced`) |
| Deformers: bend, twist, wave, squash, stretch, mesh-warp | `compositor`, `deform` | `deform.*` (7 cases), `golden.deformers_mesh_warp` |

## 360 and cameras

| Feature | Implementation | Verified by |
|---|---|---|
| Equirectangular canvas, horizontal wrap | `renderer`, `compositor` | `golden.equirect_canvas`, `integration` (`tests/data-equirect.xml` dimensions) |
| Perspective viewport from a panorama: yaw, pitch, roll, vertical FOV | `camera` | `camera.pixel_center_is_exact`, `camera.positive_pitch_looks_down`, `golden.viewport_extraction`, `integration` (`golden viewport`) |
| Spherical metadata: MP4 `sv3d`/`st3d` and Spatial Media v1 UUID, Matroska projection | `encoder`, `spatial` | `encode.spherical_side_data`, `encode.spatial_offsets_promote_to_co64`, `integration` (UUID and `sv3d` check, resume keeps metadata) |

## Output, CLI and robustness

| Feature | Implementation | Verified by |
|---|---|---|
| In-process encode: H.264, H.265, FFV1; MP4/MOV/Matroska; AAC audio; bit-exact output | `encoder` | `encode.h264_mp4_round_trip`, `encode.h265_mp4_round_trip`, `encode.ffv1_mkv_round_trip`, `encode.bitexact_output`, `encode.open_rejects_bad_configuration`, `encode.finish_once_and_destroy_unfinished`, `integration` |
| libav failure handling | `encoder`, `video`, `audio` | `encode_faults.*` (every wrapped libav call), `encode_faults.finish_failure_keeps_moov` |
| Allocation-failure handling (every core allocation during load, asset load, one frame, encode) | all modules | `oom.*` (6 cases) |
| Preview PNG/PPM and FNV-1a preview hash | `renderer` | `cli.preview_png`, `cli.preview_default_name`, every `golden.*` case, `integration` (PNG previews) |
| `--hash` per-frame hashes, thread invariant | `renderer` | `cli.hash_threads`, `cli.hash_with_resume` |
| Segmented `--resume` with manifest, reuse and packet-copy mux | `resume`, `renderer`, `encoder` | `cli.resume`, `integration` (resume byte-identity) |
| CLI parsing, exit codes, usage | `cli_args`, `main` | `args.*` (4 cases), `cli.help`, `cli.version`, `cli.unknown_option`, `cli.frame_out_of_range`, `cli.missing_asset_exit` |
| Optional OpenCL color conversion with CPU fallback | `gpu`, `renderer` | `integration` (`--renderer gpu` preview equals the CPU one) |
| Metrics and per-frame JSON Lines trace | `renderer`, `main` | exercised by `docs/benchmark.md` measurements; no automated check |

## Deliberate limits

- The 2D solver and the soft bodies are deterministic visual behavior, not
  verified physical simulation. Spheres, boxes and planes are camera-facing
  sprites; there are no PBR materials or ray-traced shadows; the lens flare
  is not optical.
- Characters missing from the chosen font render as its missing-glyph box;
  there is no font fallback.
- Frames are bit-identical for one compiler, C library, architecture and set
  of library versions; the golden references are regenerated and reviewed
  when those change (`tests/golden/README.md`). The OpenCL conversion is not
  claimed identical across GPU vendors; the CPU path is the reference.
- Allocation-failure injection covers allocations made by the engine's own
  code; allocations inside the shared libraries are not intercepted.
