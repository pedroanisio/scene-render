---
disclaimer:
  notice: >-
    No information within this document should be taken for granted.
    Any statement or premise not backed by a real logical definition
    or verifiable reference may be invalid, erroneous, or a hallucination.
  generated_by: "AI-assisted"
  date: "2026-10-10"
---

# Contributing

## Disclaimer

This work is subject to the methodological caveats and commitments described in [@DISCLAIMER.md](./DISCLAIMER.md).
> No statement or premise not backed by a real logical definition or verifiable reference should be taken for granted.

The checks below are the ones `.github/workflows/ci.yml` runs. A change is ready when they pass locally. Back to the
[README](README.md).

## Contributor licence agreement

scene-render is licensed under PolyForm Noncommercial 1.0.0, and Pedro Anisio Silva also licenses it commercially
([COMMERCIAL.md](COMMERCIAL.md)). A contribution can be accepted only if it can be distributed under both. By
opening a pull request, or by otherwise submitting code, documentation, data or any other material ("your
contribution") to this repository, you agree to the following:

1. **Licence to the maintainer.** You grant Pedro Anisio Silva a perpetual, worldwide, non-exclusive, royalty-free,
   irrevocable licence to use, reproduce, modify, prepare derivative works of, publicly display, publicly perform,
   sublicense and distribute your contribution and such derivative works, under any licence terms, including the
   project's current licence, later licences and commercial licences.
2. **Patents.** You grant Pedro Anisio Silva, and everyone who receives the software from him, a perpetual,
   worldwide, non-exclusive, royalty-free, irrevocable patent licence to make, use, sell, offer to sell, import and
   otherwise transfer your contribution. The licence covers the patent claims you can license that your contribution
   alone, or its combination with the work it was submitted to, would infringe.
3. **Your right to grant it.** You confirm that you created the contribution, or have the right to submit it under
   these terms. If your employer or anyone else has rights in it, you confirm they have allowed you to make this
   grant. Material from other sources must be marked as such, with its licence.
4. **No other obligation.** You keep the copyright in your contribution. Nothing here obliges the maintainer to use
   it, and the contribution is provided as is, without warranty.

Add the line `I agree to the contributor licence agreement in CONTRIBUTING.md.` to your first pull request's
description. Commits are authored under the contributor's own name. This agreement is maintained by the project; it
is not legal advice, and a contributor who needs certainty about its effect should seek it independently.

## Toolchain

- **Rust:** 1.90 or newer (`rust-version` in `Cargo.toml`). CI also checks the whole workspace on exactly 1.90.
- **FFmpeg and ffprobe:** 5.1 or newer, for media tests and encoding. Without them, or without an encoder a test
  needs, media tests skip with a `SKIPPED` line unless `SR_REQUIRE_FFMPEG=1` is set, as in renderer CI.
- **Python 3.11 or newer:** for `tools/`. The tool tests use the standard library (`tomllib` needs 3.11). The
  conformance oracle (`tools/oracle.py`, `tools/build_corpus.py`) also needs `lxml`.

## Before you push

```sh
python3 tools/check_release_hygiene.py --all      # no working notes, private paths or session narration
python3 tools/doc_facts.py --check                # figures quoted in documents equal their sources
python3 tools/doc_links.py                        # links and repository paths in documents exist
cargo fmt --all --check                           # rustfmt.toml: max_width 120
cargo clippy --workspace --all-targets --features scene-render/pdf -- -D warnings
cargo test --workspace --features scene-render/pdf --locked
python3 -m unittest discover -s tools/tests -v
```

- **Test prerequisites:** a test that needs a GPU adapter, FFmpeg or a HEIC decoder asks `sr-testkit` (a
  dev-dependency, never published), which prints `SKIPPED (<test>): <reason>` or, under `SR_REQUIRE_GPU=1`,
  `SR_REQUIRE_FFMPEG=1` or `SR_REQUIRE_HEIC=1`, fails with the reason. Do not read those variables in a test.
- **GPU tests:** they skip without an adapter unless `SR_REQUIRE_GPU=1` is set. CI runs the tests on Vulkan in
  <!-- facts:ci-shards -->20<!-- /facts --> shards (`tools/ci_tests.py`).
- **Corpus:** when you change the schema or the rules, run `python3 tools/build_corpus.py`. Commit the result:
  CI fails if `tests/corpus` differs from what the generator writes.
- **Staged files:** `tools/check_release_hygiene.py --staged` checks only what you are about to commit.

## Commits

- Use conventional subjects: `feat:`, `fix:`, `perf:`, `test:`, `docs:`, `refactor:`, `chore(release):`, with an
  optional scope such as `fix(gpu):`.
- Do not use delivery-phase labels or `WIP` in the subject. `tools/check_release_hygiene.py --commit-msg FILE`
  rejects them.
- Commits carry their author only, with no co-author trailers.

## Schema changes

The schema is vendored from [sr-core](https://github.com/pedroanisio/sr-core), and `schema/UPSTREAM` records the
release, digests and local additions. The schema changes through Scene Render Enhancement Proposals (SREPs), for
which this engine is the reference implementation. A local addition must be listed in `schema/UPSTREAM`.

## Documentation

- **Generated figures:** mark a figure that comes from the schema, the corpus or the codes as
  `<!-- facts:NAME -->VALUE<!-- /facts -->`. `python3 tools/doc_facts.py --list` shows the names, and `--write`
  updates the values.
- **Licence notices:** a dependency that compiles or links native code must be named in `THIRD-PARTY-NOTICES.md`.
  `tools/tests/test_notices.py` checks this against `Cargo.lock`.

## Releases

1. Set the version in `Cargo.toml` and write `RELEASE_NOTES.md`, starting with `# scene-render vX.Y.Z`.
2. Run `python3 tools/changelog.py` and commit `CHANGELOG.md`.
3. Push a tag `vX.Y.Z` that matches the workspace version. `.github/workflows/release.yml` runs the complete CI
   suite, then builds, smoke-tests and publishes the archive.
