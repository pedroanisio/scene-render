# B1-2 style tokens and metadata

Scope: the root-section portion of B1-2, based on `0f5be24`. Relative
lengths remain a required B1-2 component with a separate design/implementation
pass covering compositor, masks, depth cards and physics. This note does not
reduce the batch objective. Text styles and accessibility remain deferred.

## Style tokens

The scene owns a document-ordered array of `{name, value, source_line,
resolved_index, resolved_hops}` tokens. Names are case-sensitive and follow
the XSD pattern.
`var(--brand)` addresses the token whose name is `brand`; the two prefix
hyphens are syntax, not part of that name. Values remain strings, so an unused
numeric token is legal. A color consumer must resolve to a valid color.

Aliases may refer forward to another token. Resolve every alias at load time
using visiting/complete states, with a bounded recursion depth. A missing
reference, cycle, duplicate name or malformed token reference is a load error
at the declaration. Store the terminal token's index instead of copying its
value through every alias, and its full alias-chain length. A literal has zero
hops; an alias has one plus its target's hops, including an already-resolved
target. Reject chains longer than 64 regardless of declaration order. No hash
iteration affects output.

The project precedes styles in the XSD, so its background can reference a
later declaration. Extend the existing libxml schema/capability pass with an
optional preparation hook that gathers and resolves the root's styles before
Expat constructs scene values. The original bytes remain unchanged for line
numbers, Expat parsing and the resume fingerprint. The standalone schema
checker keeps its current behavior by supplying no preparation destination.
The Expat styles/token handlers then enforce the supported attributes and
nesting without allocating a second token table.

Use one XML color helper at all existing color consumers: project background,
text/vector asset colors, node fills/strokes and particle colors, materials,
lights, effects, and animated color keys. It resolves a token once and invokes
the existing color parser. Rendering sees the same numeric color as a literal
document and performs no token lookup. Deferred paint/gradient consumers use
the helper when their parent feature lands.

Before this feature, add a status-returning color parser in an isolated,
output-neutral refactor. Its boolean wrapper retains existing behavior and
arithmetic, while the new XML helper can distinguish malformed colors from
allocation failure in the decimal-color parser. This avoids reporting OOM as
an invalid token value. Verify that refactor with the byte oracle separately.

Limits: `SR_MAX_STYLE_TOKENS=4096`, `SR_MAX_TOKEN_NAME=128` UTF-8 bytes,
`SR_MAX_TOKEN_VALUE=4096` bytes and `SR_MAX_TOKEN_DEPTH=64` alias hops. Check
counts before growth, cap capacities and preserve source positions. The
scene frees names, values and the array on success and every failure path.

## Metadata and outputs

The scene owns an ordered array of `{name, value, source_line}` entries.
The metadata element's ten schema attributes keep their literal names;
`meta` children add their explicit name/value. Names must be nonempty.
Duplicate names are rejected case-insensitively for ASCII, matching the
container dictionary's lookup behavior; values and Unicode remain unchanged.
Do not synthesize timestamps, a generator name or other environment data.

`SrOutput.embed_metadata` defaults to true and accepts `embedMetadata`.
The flag is per output, ready for B1-6's output array. False keeps entries in
the scene while omitting them from the container. A scene without authored
metadata takes the exact existing container setup path.

Write metadata into the encoder's `AVFormatContext` before its header. Check
every dictionary and muxer-option allocation result. MP4/MOV need
`use_metadata_tags` in addition to the existing `faststart` flag to retain
arbitrary user keys; enable it only when embedding authored tags. Matroska
uses its native tag dictionary. Verify actual round trips in the SDK,
including known keys, custom keys, empty values, Unicode and multiline text;
documented container key canonicalization is acceptable; silent value loss
is not. If a container-reserved key cannot round-trip, reject it with a precise
diagnostic instead of silently claiming it was embedded.

The SDK 25.08 / FFmpeg 7.1.3 C-API probe establishes this output policy:

- Every container rejects `encoder`, the `encoder-` prefix, and
  `creation_time`, using ASCII case-insensitive matching. The first two are
  removed by the muxer; the last invokes timestamp parsing that can read the
  clock or timezone and changes its representation. The schema's `created`
  and `modified` attributes remain ordinary strings and do not invoke it.
- MOV/MP4 also reject `location` (numeric conversion and a second atom replace
  the authored string) and `com.apple.quicktime.artwork` (string data is
  consumed as cover art and disappears). With authored metadata, set
  `write_tmcd=0`, so a `timecode` tag stays a string and creates no extra
  stream. These flags are absent on the old no-metadata path.
- Matroska also rejects `duration`, `encoding_tool`, `stereo_mode` and
  `alpha_mode`: they are discarded or alter container/track fields.
- Before submitting Matroska keys, uppercase ASCII and change spaces to
  underscores, matching native Matroska tag spelling. Pre-uppercase also
  prevents FFmpeg from interpreting a `-en`/`-eng` suffix as a language tag;
  both remain distinct literal keys. The SDK probe verifies this, and a
  regression test pins it. Canonicalize `PERFORMER` to `LEAD_PERFORMER` and
  `TRACK` to `PART_NUMBER`, FFmpeg's two native aliases. Detect collisions
  after this canonicalization before creating a dictionary; never overwrite
  one authored entry with another. Check reserved keys again after
  canonicalization, so `creation time` cannot bypass the timestamp rule.
  Demuxing may return the generic `performer` and `track` names. All other
  non-ASCII bytes and all values remain unchanged.

Container-specific validation occurs against the actual selected output path,
including CLI overrides, before opening the output or starting resume segment
work. Preview rendering and `embedMetadata="false"` retain all entries without
applying embedding restrictions. Diagnostics name the entry, source line,
container and reason. Dictionary construction and option setting check every
allocation result. Extend the OOM harness to intercept the project's direct
`av_dict_set` calls, since its existing malloc wrappers do not intercept
allocations inside the shared FFmpeg library.

Matroska's internal metadata conversion rebuilds its dictionary and ignores
allocation failures. After a successful `avformat_write_header`, verify that
every expected canonical key/value still exists in the format context's
dictionary. A missing or changed entry fails encoder setup and closes the
partly opened output; it must not report a successful embed. Exercise this
upstream failure path with a test-only `avformat_write_header` wrapper that
removes an authored dictionary entry while returning success. The validation
is bounded by the metadata limits and also protects against unexpected muxer
rewrites after SDK changes.

Apply authored metadata to full encoding and the final resumed/remuxed output.
Internal resume segments omit authored metadata: their container may differ
from the final output and they are not deliverables. The final copy path
constructs metadata from the scene instead of copying segment tags. Resume
already fingerprints the exact scene XML. Verify that changing a metadata
value invalidates reuse and that an unchanged resumed output is thread-invariant
and retains the tags. No external file or new cache is added.

The C-API probe round-tripped all ten schema attributes, arbitrary custom
strings, empty title/custom values, Unicode and multiline values in MP4, MOV
and Matroska, with exactly one video stream. It reproduced value loss for
Matroska aliases, space collisions and lowercase language-suffix collisions,
and for MOV location/artwork. Uppercase Matroska keys preserved distinct
`X-EN`, `X-ENG` and `X` entries. Production tests must repeat these cases,
including reserved names in mixed case, canonicalization bypasses, and the
MOV timecode option. Compare repeated encodes across UTC and a non-UTC timezone.

Limits: `SR_MAX_METADATA_ENTRIES=256`, `SR_MAX_METADATA_NAME=128` UTF-8
bytes and `SR_MAX_METADATA_VALUE=4096` bytes. These bound allocation and
iteration without a second aggregate budget. Limits and duplicate failures
name metadata/meta and the offending attribute. All strings are scene-owned
and released by `sr_scene_free`, including a partly constructed entry.

References: [FFmpeg's format metadata and MOV options](https://ffmpeg.org/ffmpeg-formats.html)
and [its public metadata API](https://www.ffmpeg.org/doxygen/6.1/group__metadata__api.html),
plus the pinned [MOV muxer](https://github.com/FFmpeg/FFmpeg/blob/n7.1.3/libavformat/movenc.c),
[Matroska muxer](https://github.com/FFmpeg/FFmpeg/blob/n7.1.3/libavformat/matroskaenc.c)
and [language conversion](https://github.com/FFmpeg/FFmpeg/blob/n7.1.3/libavformat/avlanguage.c)
implementations.
These describe the APIs; tests against the installed SDK establish the actual
muxer behavior and supported key policy.

## Capabilities, tests and cost

Enable root metadata/styles and their implemented children/attributes only
with complete parser, evaluator/output and documentation. Token forms are
enabled only on implemented color consumers; `textStyle`, accessibility and
paint references remain gated. New elements require 1.1. The new output
attribute follows the existing rule allowing new attributes in 1.0 scenes.

Tests cover forward project references, aliases, cycles, unknown and duplicate
tokens, all implemented color hosts, animated color keys, non-color terminal
values, exact resource limits, malformed/truncated documents and fixed-seed
mutations. Alias-depth boundary tests use 64 and 65 edges in forward, reverse
and fixed shuffled declaration orders. OOM replay includes token
collection/resolution, decimal color parsing, metadata parsing and encoder setup. A token golden is byte-identical
to a literal-color reference and checks warm/shuffled frames and 1/4 threads.

Metadata tests inspect actual MP4, MOV and Matroska tags via libavformat and
extend `sr-probe` for integration inspection. Cover default/true/false
embedding, every metadata attribute, custom names, values and duplicates,
full versus resumed output, and 1/4-thread byte identity. Existing golden and
integration hashes remain unchanged; add only new fixtures/hashes.

Run the full Release/ASan/coverage matrix, frame-order suite, byte oracle and
strict performance check before the item merge. Parsing adds bounded work at
load time (linear lookup is at most 4096 squared string comparisons), and
metadata adds at most 256 dictionary entries once per encoder. Neither adds
per-pixel or per-sample work. The unchanged-scene CPU budget remains under 2%.
