# scene-render v0.5.0

This release moves the engine to a new repository, https://github.com/pedroanisio/scene-render, and a new licence. The code and behaviour are those of v0.4.1.

## Licence

- **From 0.5.0, scene-render is licensed under the PolyForm Noncommercial License 1.0.0** (`LICENSE`, unmodified from https://polyformproject.org/licenses/noncommercial/1.0.0, with its required notice in `NOTICE`). Noncommercial use is free. Commercial use needs a separate licence from Pedro Anisio Silva (`COMMERCIAL.md`).
- **Versions up to 0.4.1 were published under the MIT License.** Copies of those versions keep the MIT terms they were released with.
- **Contributions** are accepted under the contributor licence agreement in `CONTRIBUTING.md`, so they can be distributed under the noncommercial and the commercial licence.
- The Cargo `license` field is the SPDX identifier `PolyForm-Noncommercial-1.0.0`. `THIRD-PARTY-NOTICES.md` is unchanged except for its first line: the dependencies keep their own permissive licences.

## Repository

- The repository starts from a clean tree at v0.4.1 (commit `1306b0e6` of the previous repository). The earlier history is kept privately as the record of authorship and dates.
- `CHANGELOG.md` keeps every earlier release's notes. `tools/changelog.py` reads them from `tools/changelog-history.md`, because their tags belong to the previous repository.
- The map-tile requests' User-Agent names the new repository.
- Commits carry their author only, with no co-author trailers (`CONTRIBUTING.md`).

## Changes that can alter output or verdicts from v0.4.1

- None. Rendered frames, validation verdicts and the vendored schema (sr-core 1.7.0) are those of v0.4.1.

The Linux archive contains the executable, source commit identifier, capability manifest, README, licence, notice, commercial-licence page and third-party notices. Its executable comes from the successful CI run for the release commit and is checksum-verified and smoke-tested before packaging.

Verify the download with `sha256sum -c scene-render-v0.5.0-linux-x86_64.tar.gz.sha256`, extract it, then run `./scene-render --help`.
