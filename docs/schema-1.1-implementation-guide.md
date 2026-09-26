# Scene format 1.1 — implementation guide

This guide explains **how** to implement the three batches. The batch proposals
say **what** to build and why:

- `docs/schema-1.1-batch1-proposal.md` (B1): loader, animation core, lengths,
  compositing, shapes, timeline, outputs.
- `docs/schema-1.1-batch2-proposal.md` (B2): expressions, reuse, constraints,
  responsive media.
- `docs/schema-1.1-batch3-proposal.md` (B3): everything else; closes the
  contract.

**If this guide and a proposal disagree, the proposal wins.** Update this guide
in the same change.

The guide applies to every contributor: people, Claude sessions and Codex
lanes. The only exception is the rule for Codex (§8).

---

## 0. Before the first line of batch code

Do these once, in this order.

1. **Commit the schema errata.**
   - `schema/scene-render-1.1.xsd` is already committed as it was drafted (in
     `8f4dcfc`, whose message is unrelated to the change). It does not yet
     contain errata E1–E9 (B3 §7).
   - Apply all nine in **one** commit:
     `fix(schema): apply 1.1 errata E1-E9`.
   - Run xmllint over every scene in the repository. All 42 scenes that pass
     1.0 validation must also pass 1.1 validation (today two fail because of
     E1).
2. **Check in the equivalence oracle.**
   - The byte-identity oracle from the performance work (120 preview frames
     across 17 scenes at `--threads 1/3/22`, plus 3 full encodes, compared by
     SHA-256 against a reference binary) exists only in session scratch
     space.
   - Add it as `tools/equivalence-oracle.sh`. It takes `REF_BINARY`,
     `NEW_BINARY` and an optional scene list, and exits non-zero on the first
     mismatch.
   - Every refactor in these batches is verified with it.
3. **Add the frame-order check.**
   - B2 §7 describes it; build it now, as part of B1-0, so every batch uses it
     from the start.
   - Add it as `tools/frame-order-check.sh` and as the CTest `frame_order`.
   - It renders each golden scene four ways and compares frame hashes:
     sequentially, as shuffled `--preview-frame` calls, as `--range` slices,
     and with `--resume` where segments are killed and resumed.
4. **Record the base.**
   - `perf-port` is merged (`339c366`): a persistent thread pool and
     pipelined encoding.
   - The B2 §2 note "pthreads created per call" is now out of date. Treat
     `src/parallel.c` as the current source of truth for threading.
   - Re-record `benchmarks/baseline.json` on the owner machine
     (`scripts/perf-check.py --update`) before any lane starts measuring
     against it.

## 1. Build and test environment

Build and test only inside the Flatpak SDK. The bare host has no toolchain
headers. Binaries built inside the SDK run only inside it.

```sh
R=/home/admin/codebases/scene-render      # or your worktree path
sdk() { flatpak run --filesystem=/tmp --filesystem="$R" --command=sh \
          org.freedesktop.Sdk//25.08 -c "cd '$R' && $*"; }

sdk 'cmake -S . -B build -DCMAKE_BUILD_TYPE=Release && cmake --build build --parallel'
sdk 'ctest --test-dir build --output-on-failure'
sdk 'cmake -S . -B build/asan -DCMAKE_BUILD_TYPE=Debug -DSR_SANITIZE=ON && cmake --build build/asan --parallel'
sdk 'ASAN_OPTIONS=detect_leaks=0 ctest --test-dir build/asan --output-on-failure'
sdk 'tools/coverage-gate.sh'
sdk 'make perf-check'
```

- LeakSanitizer cannot run in the sandbox. Leak checking comes from the OOM
  suite (`unit.oom`), which wraps `free`.
- Pass `CPPFLAGS` and `LDFLAGS` to `make` through the **environment**, never
  on the command line. On the command line they override the Makefile's `+=`
  flags.
- Keep one build directory per configuration inside the worktree:
  `build/`, `build/asan/`, `build/coverage/`.

## 2. Non-negotiable engineering rules

Every item in every batch follows these. A review finding that breaks one of
them blocks the merge.

1. **Determinism.**
   - The same inputs, toolchain and library versions give bit-identical
     frames and audio.
   - Keep `-ffp-contract=off -fno-fast-math`.
   - No wall clock, no environment-dependent behaviour, no `rand()`. Every
     random value comes from `src/random.c` (B2-0), seeded by the documented
     rule.
   - Iteration over hash maps must not decide output order. Sort, or iterate
     in document order.
2. **Any frame can be rendered on its own.**
   - Render-time evaluation is a pure function of `(scene, t)`.
   - Anything stateful is computed at prepare time and cached (B3 D1), or
     computed by re-evaluating the scene at other times (B3 D2).
   - The `frame_order` CTest enforces this for every golden scene.
3. **Evaluation is read-only and thread-safe.**
   - Nothing reachable from `const SrScene *` is written during rendering.
   - Lazily built caches are allowed only behind the existing
     once-and-mutex pattern (see the particle rate cache), and must not
     change results.
4. **Frames do not depend on thread count.** 1 thread and 4 threads give
   byte-identical frames, and encoder thread counts are pinned. Work is split
   by fixed rules (rows, tiles, items), never by timing.
5. **Every input is bounded.**
   - Every loop, recursion and allocation whose size comes from the document
     or from media needs a named limit (`SR_MAX_*`) with a diagnostic that
     names the element.
   - Examples: node expansion, include depth, the expression operation
     budget, and the sizes of LUTs, meshes, captions and tracking data.
6. **Nothing fails silently.**
   - A construct that is valid in the schema but not implemented fails with
     an "unsupported in this build" diagnostic.
   - A semantic error fails the load, with line, element and attribute.
   - A runtime error fails the render. `--expression-errors=hold` is the only
     sanctioned relaxation.
7. **Every input is fingerprinted.** Any new file the render reads goes into
   the resume fingerprint (`src/resume.c`). This includes included scenes,
   sequence members, LUTs, fonts, caches and track data. Library versions
   that affect output (ICU, HarfBuzz, librsvg, libav) are fingerprinted too.
8. **Existing scenes do not change.**
   - 1.0 scenes and all goldens stay byte-identical.
   - The only sanctioned change is the single 3D refresh (B3 D4), in its own
     reviewed commit.
   - A change that alters arithmetic on an existing path waits for that
     refresh, or goes behind a 1.1-only construct.
9. **Licences.**
   - Allowed: SDK libraries, or permissive vendored code pinned in
     `third_party/MANIFEST`.
   - No GPL additions.
   - C++ only for TinyUSDZ, behind a C shim (B3 §5).
10. **No secrets in the repository or in logs.** Credentials are named by
    profile (B3 D7). Diagnostics redact tokens and URLs with user information.

## 3. Code conventions

These are inferred from the current source. The repository has no formatter
config; §10 lists adding one as an optional item.

**Names:**
- Public functions `sr_*`, types `Sr*`, constants and macros `SR_*`.
- File-local helpers are `static` and unprefixed.
- `-Wmissing-prototypes` means anything non-static needs a header prototype.

**Files:**
- Public API in `include/scene_render/<module>.h`, guarded as
  `SCENE_RENDER_<MODULE>_H` and included as `"scene_render/x.h"`.
- Internal headers go in `src/*_internal.h`.
- New subsystems get their own module (`expr.c`, `random.c`, `layout.c`,
  `ocio.c`, `dsp_*.c`, …) instead of growing `compositor.c` or
  `effects.c`.
- Effect families live in `effects_<family>.c`.

**Format:**
- 4-space indent, K&R braces with the function brace on the same line.
- Keep lines to 80 columns; 100 is the hard maximum.
- One statement per line. Do not copy the dense style of `xml_physics.c`.
- Unit test files start with `/* SPDX-License-Identifier: Apache-2.0 */`.

**Comments:** sparse `/* … */` comments that explain *why*, plus
trailing-comment units on struct fields (`/* seconds */`, `/* linear, 0..1 */`).
No `//` comments.

**Errors:**
- Public functions return `SrStatus`. Helpers return `bool` or NULL, and the
  caller maps that to a status.
- Loader errors go through `sr_xml_fail` or `SR_XML_FAIL_RETURN`.
- Resolve-time and later errors go through
  `sr_diag_error(diag, line, element, attribute, fmt, …)`.
- Warnings go through `sr_diag_warning`.

**Memory:**
- Use `sr_alloc`, `sr_realloc` and `sr_strdup`, and check every result.
- An allocation failure returns `SR_ERR_MEMORY`, or `"out of memory"` in the
  loader.
- Before multiplying sizes, check for overflow (see `xml.c:68-88`).
- Clean up with an early return and explicit frees, or with a single `fail:`
  label.

**Ownership:**
- The scene owns everything reachable from it.
- State ownership in header comments ("borrowed; must outlive …",
  "owned by the caller").
- Every new heap object has a paired `*_free`, reached from `sr_scene_free`.

**OOM contract:** every new operation is added to `tests/unit/test_oom.c`.
When allocation *n* fails, the operation must return `SR_ERR_MEMORY` (or the
loader's "out of memory") and leak nothing.

## 4. Recipes

The recipes assume B1-0 has landed (table dispatch, capability table,
property registry). Until then, follow the existing recipe in `xml.c`:
`on_start` branch, `allowed[]` array, `sr_xml_attrs_allowed`, then
`resolve_*`.

### R1. A new element, attribute or enumeration value

1. **Schema.** Confirm the construct is in the XSD, which should already be
   true after §0. If the item needs a schema change, it goes in a separate
   `fix(schema)` commit and is recorded in B3 §7.
2. **Dispatch table.** Add an entry with: element, allowed parent kinds,
   handler, minimum version (`1.0` or `1.1`) and capability flag.
3. **Handler.**
   - Parse with the typed helpers (`sr_parse_double`, `sr_parse_u32`,
     `sr_parse_u64`, `sr_parse_bool`, `sr_parse_color`, the length parser
     from B1-2, and the enum tables).
   - Validate ranges that the XSD cannot express.
   - Record `source_line` for later diagnostics.
4. **Resolve.** In `xml_resolve.c`, resolve references through the scene
   index and **check the kind** of the referenced element (B2 S7). Semantic
   checks go here too; list each one in `docs/xml-reference.md`.
5. **Capability.** Flip the entry to implemented only when rendering, tests
   and documentation are all done. Until then, the construct loads with the
   "unsupported" diagnostic, so a partial lane can merge.
6. **Documentation.** Add the section to `docs/xml-reference.md`: semantics,
   units, defaults, limits, the determinism rule it follows, and a minimal
   example.
7. **Fixture.** At least one rendered scene uses the construct. That is the
   requirement `schema.coverage` checks (B3 §1).

### R2. A new animatable property

- Register `(host kind, name, value type, field, bounds)` in the property
  registry.
- Never add a lookup chain by hand (the pattern of `sr_node_property`, now
  replaced).
- The name follows the existing dotted style: `position.x`, `rotation.x`,
  `scale.y`.
- If the property can also be set by an override or bind, the registry entry
  gives the parser for its static attribute form.
- Add a unit test that animates it, sets it by expression (after B2-1) and
  overrides it (after B2-4).

### R3. A new effect or compositing kernel

- It is a pure function from input buffers and parameters evaluated at `t` to
  an output buffer. It works in float premultiplied in the working space, and
  row-parallel through `sr_parallel_for`.
- It handles `mix` and `enabled` through the shared wrapper and does not
  reimplement them.
- A second input (`source`) is rendered through the matte/source path. It
  must never read the frame being written.
- Randomness is seeded per effect id and the frame index.
- Test with a unit test against a closed-form reference on small synthetic
  buffers, plus an entry in the family's golden sheet.

### R4. A new asset kind or media format

- Loading happens in `assets.c` before the render, and unloading in
  `sr_assets_unload`.
- The file path resolves against **that asset's** base directory (B2-5).
- Verify `sha256` when it is present.
- Add every file read to the resume fingerprint.
- Cap every dimension and count (the existing ≤ 16384 px for images,
  frames, samples and entries).
- Decoded media is converted into the float premultiplied working space
  exactly once.

### R5. A precomputed analysis (B3 D1)

This covers audio analysis, the full mix, optical flow, tracking,
stabilisation, particle simulation, flash analysis and HDR statistics.

Use physics as the model (`physics.c:863-985`):
1. **Prepare stage.** It runs once in `renderer.c`, before the first frame,
   on every path including `--preview-frame`.
2. **Output.** A table indexed by frame, sample or step that render-time code
   reads without modifying.
3. **Cache.** An optional disk cache in a versioned format, keyed by a
   signature over every input: parameters, media fingerprints, format
   version and library versions. A signature mismatch recomputes; it never
   reuses a stale result.
4. **CLI.** A `--<name>-cache` path and a `--no-cache` flag.
5. **Tests.**
   - Cached and uncached runs give identical output.
   - The frame-order check passes.
   - A change to one input invalidates the cache.

### R6. A feature that re-evaluates time (B3 D2)

This covers motion blur, echo, posterize-time, frame-mix and
pixel-motion-blur.

- Evaluate the subtree at the other times through the normal draw path, with
  the time in the context replaced.
- Never keep previous frames.
- The sample times are a fixed function of the frame and the `quality` level.
- Document the cost in the construct's reference section, and add the case to
  `perf-check`.

### R7. A parser for an external format

This covers expressions, captions, LUTs, OCIO YAML, PSD, Lottie, TeX, CSV,
JSON, tracking data and IES profiles.

- Hand-written or vendored and permissive, with no network or file access
  beyond the declared path.
- Every size, count and nesting depth is bounded (rule 5).
- Errors carry a position (file, line or byte offset) and are reported through
  `sr_diag_*`.
- Add a seeded input generator to the fuzz suite (`tests/unit/test_fuzz_*.c`)
  that produces valid, mutated and truncated inputs. It runs under ASan and
  UBSan in CI with a fixed seed list; findings become regression seeds.
- Add an OOM entry.

### R8. Vendoring a dependency

1. Check the licence against B3 §5; anything not listed there needs owner
   approval.
2. Add the source to `third_party/<name>/` at a tagged release. Record the
   URL, version and SHA-256 of the archive in `third_party/MANIFEST`.
3. Build it with CMake as a static library. The per-target warning exceptions
   are listed in the manifest.
4. Add its licence to `NOTICE`.
5. Wrap it behind our own module. No third-party headers in
   `include/scene_render/`.
6. Supply our allocator hooks if the library supports them, so that
   `unit.oom` covers it.

### R9. CLI flags and subcommands

- Parse flags in `cli_args.c` with the existing strict parsing (unknown or
  duplicate flags are errors).
- Add a CLI test in `tests/cli/` covering success, bad value and exit code.
- The subcommands `resolve` and `deliver` (B3 D7) run no rendering and never
  run inside a render.

## 5. Testing requirements

| Test | When it is required | How |
|---|---|---|
| Unit tests | every item | a suite in `tests/unit/test_<area>.c`, registered in `harness.h`, the `suites[]` table in `test_main.c`, `SR_UNIT_SUITES` in `CMakeLists.txt` and `UNIT_SUITES` in `Makefile` |
| Golden scenes | every visible feature | at most 320×180 and 24 frames; add a `GOLDEN(...)` line in `test_golden.c`; `SR_UPDATE_GOLDEN=1 build/sr-unit-tests golden`; review the PNGs; commit them with the reason |
| Thread invariance | every golden | built into the golden runner (1 vs 4 threads) |
| Frame order | every golden | `frame_order` CTest (§0.3) |
| OOM | every new allocating operation | an entry in `test_oom.c` |
| Fuzz | every new parser | R7 |
| Integration | new outputs, codecs, audio, containers | `tests/run-integration.sh` plus `sr-probe`; update `tests/golden.sha256` in the same commit |
| CLI | every new flag or subcommand | `sr_cli_exit(...)` or a `tests/cli/*.cmake` script |
| Equivalence oracle | every refactor of an existing path | `tools/equivalence-oracle.sh` against the pre-change binary: 120/120 frames and 3/3 encodes must match |
| Conformance | where B3 §8 lists a reference suite | results checked in as report files; threshold failures fail CI |
| Coverage | every item | `tools/coverage-gate.sh`; floors are raised after each milestone and never lowered |
| Performance | every item that touches the render path | `make perf-check` within the lane's budget (§7) |

A sanitizer report counts as a failure. The CTest regex
`\[FAIL\];runtime error:;ERROR: AddressSanitizer` must match nothing.

## 6. Workflow for one work item

1. **Claim it.**
   - Create a task record in `.agent-tasks/registry.json` with `task_id`,
     `agent_id`, `objective` (the proposal item, for example "B1-3
     compositing"), `declared_files` and `status`.
   - Check `.agent-tasks/locks/` for overlapping files. Several sessions work
     in this repository at the same time.
2. **Isolate it.**
   - Work in a git worktree on the branch `b<batch>-<item>-<slug>`, for
     example `b1-3-compositing`.
   - Never develop in the main checkout. Other sessions commit there, move
     branches and switch `main`.
3. **Design note (L and XL items only).**
   - Write `docs/design/<item>.md` covering the data structures, the
     algorithms with references, the limits, the determinism argument and the
     test plan.
   - Get it reviewed before code: an owner or reviewer pass plus a Codex
     second opinion.
   - B2-1 must start with `docs/expressions.md`.
4. **Refactor first, alone.** Structural changes to existing code (dispatch
   table, property registry, scene index, `random.c`, per-asset base
   directories) land as separate commits that change no output. The
   equivalence oracle verifies them before any feature commit builds on
   them.
5. **Implement in small commits.** Each commit builds, passes the unit tests,
   and leaves unfinished constructs behind the "unsupported" gate.
6. **Verify.**
   - Run the §5 matrix in Release, then in the ASan build.
   - Run the coverage gate and `perf-check`.
   - Paste the results into the merge description (§9).
7. **Review.**
   - Codex diff review (§8).
   - Reproduce each finding as a failing test first, then fix it. Findings
     that are rejected are answered in the review log with the reason.
8. **Flip the capability entries** for the constructs that are now complete.
9. **Update the documentation** (§10).
10. **Merge** (§9). Archive the task record with `actual_files` and
    `result_summary`, and remove the worktree.

## 7. Performance budgets

- Each lane records its budget in its design note, or in the merge
  description for small items. The budget is the allowed change in median
  stage CPU on `benchmarks/perf-scene.xml`, plus a lane-specific scene when
  the feature is not in the perf scene.
- **Scenes that do not use a new feature:** less than 2 % change. New
  features must not slow down documents that do not use them.
- **The feature's own cost** is documented, for example "motion blur
  N samples ≈ N× compositor stage".
- Update `baseline.json` only on the owner machine, in a separate
  `perf: re-record baseline` commit that states the reason.

## 8. Reviews

**Codex diff review, for every merge of a work item:**
- Codex lanes are **read-only** here. They cannot build, because the sandbox
  blocks bubblewrap.
- Codex reviews diffs, designs and audits whole modules. Claude or a person
  builds, runs, verifies and commits.
- The review prompt includes: the proposal item, this guide's §2 rules, the
  diff, and the test evidence.
- Ask Codex explicitly for: determinism leaks, frame-order state, thread
  safety, missing limits, OOM paths, fingerprint gaps and behaviour changes
  to 1.0 scenes.

**Reviewer checklist:**
- [ ] Every rule in §2 holds; for rules 2 and 8, name the evidence.
- [ ] Every new construct has a capability entry, a fixture and a
      `xml-reference.md` section.
- [ ] Every new allocation is OOM-tested; every new parser is fuzzed.
- [ ] Every new input is fingerprinted.
- [ ] The oracle was run for every refactor commit.
- [ ] The performance budget is met.
- [ ] No schema change without a `fix(schema)` commit and a B3 §7 entry.

## 9. Git conventions

**Commits:**
- Conventional form `type(scope): summary`. Types: `feat`, `fix`, `perf`,
  `test`, `docs`, `chore`, `refactor`. Scopes follow modules: `loader`,
  `anim`, `expr`, `render`, `text`, `audio`, `color`, `3d`, `media`, `cli`,
  `schema`, `build`.
- The body is a bullet list of changes per file or area, then a
  **Verification** paragraph: commands run, results, oracle or hash
  comparison against the base commit.
- Every commit message matches its change. `8f4dcfc` is the counter-example.
- Agent-authored commits end with the attribution line the session
  specifies.

**Merges:**
- `Merge <branch>: <summary>`, with a description of any conflict
  resolution.
- Before any merge or commit in the main checkout, run
  `git branch --show-current` and `git status`.
- Never force-push or rewrite shared history.
- Pushing to `origin` needs the owner's approval.

**Merge description template:**
```
Item: B<n>-<id> <title> (docs/schema-1.1-batch<n>-proposal.md §4)
Constructs now implemented: <list; capability entries flipped>
Rules §2: determinism <evidence>; frame-order <ctest result>; 1.0 identity <oracle result>
Tests: unit <n>/<n>, golden <n>, oom, fuzz <seeds>, integration, cli
Coverage: lines <x>% / branches <y>% (floor <a>/<b>)
Perf: <stage deltas vs baseline> (budget <…>)
Review: Codex <n> findings, <n> fixed with tests, <n> rejected (reasons in log)
Docs: xml-reference §…, feature-matrix, NOTICE (if vendored)
```

## 10. Documentation duties

- **`docs/xml-reference.md`:** one section per construct (R1.6). By the end
  of B3 it covers the whole contract.
- **`docs/feature-matrix.md`:** from B1-0 on, generated from the capability
  table (`tools/feature-matrix.py`) so it cannot go stale.
- **`docs/expressions.md`** (B2-1) and `docs/design/*.md` (L and XL items).
- **`NOTICE`** and `third_party/MANIFEST` for every vendored item or data
  set.
- **`docs/migration-1.1.md`** (B3 close-out): every E1 alias, every changed
  behaviour, the 3D refresh.
- **The shared AI-Memory page** (`Workspace/AI-Memory/scene-render`): add
  facts, decisions and gotchas the repository does not record, and open
  threads at the end of each session.
- **Optional:** add `.clang-format` and `.editorconfig` matching §3, applied
  only to new and touched files so history stays readable.

## 11. Batch gates and sequencing

A batch starts only when the previous batch's gate is green.

**Batch 1 gate:**
- B1-0 through B1-6 merged.
- 1.1 is embedded.
- 1.0 → 1.1 validation passes for all fixtures.
- Every 1.0 scene is byte-identical under the oracle.
- The `frame_order` CTest is green for all goldens.
- The feature matrix is generated from the capability table.

**Batch 2 gate:**
- `docs/expressions.md` is approved.
- B2-0 was verified by the oracle before any feature merge.
- B2-1 through B2-8 merged.
- Limit tests are green: instance bomb, include cycle, `SR_MAX_NODES`,
  expression budget, reference cycles.

**Batch 3 gate (definition of done for 1.1):**
- Every B3 milestone is merged.
- `schema.coverage` is required and has no gaps.
- The unsupported path is removed.
- The conformance reports meet their thresholds.
- `migration-1.1.md` is published.
- The 3D golden refresh is reviewed.

**Parallelism:**
- Within a batch, items with no dependency arrow between them (B1 §4
  order, B2 §1 table, B3 §6) run as parallel lanes, one worktree each.
- Two lanes that must touch the same central file (`compositor.c`,
  `renderer.c`, `scene.h`) coordinate through `.agent-tasks` locks. The lane
  that lands second rebases and re-runs the oracle.
- Keep central-file edits small by adding new modules instead (§3).

## 12. Quick reference

| Task | Command (inside `sdk '…'`) |
|---|---|
| Release build and tests | `cmake -S . -B build -DCMAKE_BUILD_TYPE=Release && cmake --build build --parallel && ctest --test-dir build --output-on-failure` |
| Sanitizer run | `ASAN_OPTIONS=detect_leaks=0 ctest --test-dir build/asan --output-on-failure` |
| One unit suite | `build/sr-unit-tests <suite>` |
| Refresh goldens (then review) | `SR_UPDATE_GOLDEN=1 build/sr-unit-tests golden` |
| Integration | `make integration` (or `tests/run-integration.sh ROOT BIN WORKDIR PROBE`) |
| Coverage gate | `tools/coverage-gate.sh` |
| Performance | `make perf-check`; profile with `make profile` |
| Byte identity | `tools/equivalence-oracle.sh REF_BIN NEW_BIN` (§0.2) |
| Frame order | `ctest --test-dir build -R frame_order` (§0.3) |
| Schema check for a scene | `xmllint --noout --nonet --schema schema/scene-render-1.1.xsd scene.xml` or `build/scene-render --validate scene.xml` |
| Unsupported report | `build/scene-render --validate --report-unsupported scene.xml` (after B1-0) |
