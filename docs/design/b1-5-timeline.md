# B1-5 timeline structure

Scope: batch 1 proposal B1-5 and finding S4, on `4106b84`. Covered: the root
`markers` section (`marker`, `beatGrid` and the generated ids `beat.N` and
`bar.N`), node `startMarker`/`endMarker`, key `marker` snapping, group
`timeOffset`/`timeScale`, the `sequence` node with `timeGap`, and node `name`
and `tags`. `sequence/@transition` and `@transitionDuration` stay
unsupported until transitions land (B3-C), as the proposal says. Marker kinds
with batch 3 behaviour (`chapter`, `comment`, `todo`, `cta`; B3-H) and
`beatGrid/@source` (tempo analysis; no batch assigns it yet) also stay behind
the capability gate. `audioTrack/@startMarker` belongs to B3-G.

Everything in this item is resolved at load time. Rendering reads the result
and never evaluates a marker, a clock or a sequence. The only render-time
change is the particle clock conversion (a multiply/divide by the node's
clock rate, exact at 1.0); no render-time allocation is added, so the
compositor resource ledger is unchanged, and the existing particle
admission limits (rate cells, candidates) bound re-timed emitters.

## Clocks

Every node N has a *parent clock*, the affine map
`P_N(t) = a_N * t + b_N` (`a_N > 0`) from composition seconds `t` to the
coordinate in which N's `start`, `end` and `timeBase="composition"` keys are
written. Children of the composition have `a = 1, b = 0`: every 1.0 document
and every existing 1.1 fixture keeps project seconds exactly.

A group G with `start` s, `timeOffset` o and `timeScale` q gives its children
the clock

    C_G(t) = (P_G(t) - s) * q + s + o

The pivot is the group's own start, so at the instant the group starts its
children's timeline reads `s + o` (a child starting at `s` is `o` seconds
into its content), and `q` changes their speed from that instant on. With the defaults (q = 1, o = 0) C_G = P_G: groups without
the new attributes are unchanged, which keeps 1.0 children in absolute
composition seconds (their existing meaning). The arithmetic is exact for the
defaults: q = 1 composes as `b' = b + o` and q = 1, o = 0 copies the parent
clock without arithmetic. The group's own properties, masks, start and end
stay on the parent clock; only its children are re-timed. Nested groups
compose the maps from the root down.

A `sequence` is a group (it accepts every group attribute, including
`timeOffset` and `timeScale`) whose child nodes, in document order, are
placed one after another. With the sequence's children clock C and start s:

    cursor_0     = s
    item i clock = C - cursor_i
    cursor_{i+1} = cursor_i + end_i + timeGap

Each item's `start`, `end`, keys and subtree are written relative to its slot:
the XSD's "a child's own start is an additional offset". Every item but the
last needs a finite `end` (the next item starts from it). An item's
`endMarker` is converted into its slot (`end = item clock(marker)`), so an
item can run until a marker; `startMarker` on an item is rejected because
the sequence decides where the item starts. Item intervals are unmapped as
`unmap(C, slot + start)`. Composition endpoints are anchored: a node whose
start (end) on its parent clock equals its enclosing group's start (end),
or, for a sequence item, the previous item's end, reuses that composition
instant instead of unmapping again. Marker endpoints and gapless junctions
therefore stay bitwise exact at any nesting depth (a marker end under a
scale-10 clock leaves no blank frame before the next item or its
descendants). A
negative `timeGap` overlaps items. `object3D` and `camera` inside a sequence
keep their scene-global timing, exactly as inside a group; they are not
items. Items are placed before `z` sorting, so `z` changes drawing order only.

`timeBase="composition"` keys use the host's parent clock: the enclosing
composition timeline, which a timed group or a sequence nests like a
precomposition. This refines B1-1 ("composition time is project seconds"),
which is the same thing for every document without timed groups: a sequence
would otherwise be unusable for animated content. `local` is
`P_N(t) - start` and `normalized` divides that by `end - start`, where an
omitted end is the project end mapped into the parent clock. Masks, modifiers
and mesh-warp points share their node's clock. Shared hosts (materials,
cameras, lights, effects, force fields, 3D objects, audio tracks) keep their
B1-1 clocks, because they have no unique owning node.

The resolver writes each node track's affine clock once:

| timeBase | clock_scale | clock_offset | seconds_per_unit |
|---|---|---|---|
| composition | a | b | 1 |
| local | a | b - s | 1 |
| normalized | a / span | (b - s) / span | span |

An identity composition clock keeps `clock_set` false, so legacy tracks stay
on the legacy path. For a = 1, b = 0 these are the B1-1 expressions bit for
bit. `domain_start/domain_end` become the absolute node interval,
`(s - b) / a` and `(e - b) / a` (the project end for an open end). Node
`start_time`/`end_time` are rewritten to that absolute interval, which the
compositor, depth cards, relative-length walk and particles already compare
with composition time; the authored values are kept for diagnostics.

B1-1 computed local clocks at parse time from the sum of ancestor group
starts. Group starts do not shift children in 1.0 (the compositor compares
a child's own `start` with composition time), so that offset made a local
track disagree with its node's visibility whenever an ancestor group had a
nonzero start: with a group at 2 and a child at 3, the child appears at 3
but its local time there was -2. No fixture or example uses this (checked
over every repository scene), and B1-1 is not batch-merged, so the resolver
replaces it with the clock above instead of freezing it into the contract;
a regression test pins the corrected value. Parse time now only records
the time base for node hosts.

Clock consumers that are not tracks:

- **Video layers.** Source time advances at the node's clock rate: the
  resolver multiplies `speed` by `a` (exact for a = 1). `source.time` tracks
  get the node clock like every other track.
- **Particles.** Emission, ages and the 1/240 s rate grid run on the parent
  clock. With the emitter's absolute start S and clock rate a: local age
  `now = (t - S) * a`; a local instant u (a birth, a grid point `k/240`) is
  evaluated at composition time `S + u / a`, where the tracks apply their
  own clocks; grid lookups use `(t - S) * a / step`; the lifetime bound
  interval stays `[S, t]` in composition seconds. `a` is 1.0 for every
  existing scene, where each conversion is exact. A system in a group with
  timeScale 2 at time t equals the same system at 2t outside it, byte for
  byte; a dyadic offset reproduces exactly as well.
- **Physics.** Simulation runs on the project clock and already ignores
  node `start`; kinematic bodies integrate their configured velocity, not
  animation tracks. Under a pure offset (sequence items, `timeOffset`) this
  is unchanged. A dynamic or kinematic rigid body or a soft body under a
  clock rate other than 1 is rejected ("unsupported in this build"), because
  its simulated motion could not follow the clock. The physics cache
  signature is therefore unaffected.

`timeScale` and `timeOffset` are static attributes. The schema does not make
them animatable (animated time scaling is a time remap, B2), so they get no
property-registry row.

## Markers

`markers` is a 1.1 root section after `scene360`, before `composition`. The
scene owns a timeline object (`markers.h`): explicit markers in document
order `{id, time, duration, kind, label, color, source_line}`, an index sorted
by id for O(log n) lookup, the beat grid, and the id strings of snapped keys.
`id` is optional; a marker without one cannot be referenced. `kind` is stored
(`cue` by default). `color` goes through the shared XML colour helper, so
style tokens work. `startMarker`/`endMarker` and key snapping use the marker's
`time`; `duration` is stored for later consumers (chapters).

A `beatGrid` with `bpm` B, `offset` o and `beatsPerBar` k defines

    time(beat.N) = o + (N - 1) * 60 / B          N >= 1
    time(bar.M)  = time(beat.((M - 1) * k + 1))  M >= 1

N and M are 1-based, the musical convention (bar 1, beat 1 is the
downbeat at `offset`); `beat.1` and `bar.1` are both at `offset`. The grid
generates the beats whose time is at most the project duration, and the bars
whose first beat is generated. Both are computed as
`o + (double)((N - 1) * 60) / B` so bar and beat instants are bitwise equal.
Ids are not materialized: a reference is parsed as `beat.` or `bar.` followed
by a canonical decimal (no sign, no leading zero) and checked against the
generated range. The generated ids join the id table before any reference
resolves (S4): an explicit marker or any other element whose id equals a
generated id is a load error, and at most one `beatGrid` is allowed because a
second would generate the same ids.

References resolve in `xml_resolve.c` after parsing, through the timeline
module, with kind checks: an id that names a node, asset or other element is
reported as "not a marker"; an unknown id or a generated id outside the grid
is reported with the grid's range. All diagnostics carry the referring line,
element and attribute.

## References to markers

- **startMarker / endMarker** set the node's start/end to `P_N(time)` on its
  parent clock, and its composition interval to the marker time itself (no
  map/unmap round trip, which is inexact for scales such as 10). Each is
  exclusive with the literal `start`/`end`. The resolved interval must still
  satisfy `0 <= start < end` on the parent clock and be non-empty in
  composition seconds.
- **key marker.** The key's `time` becomes an offset in the track's own
  coordinate: `key.time = clock(track, marker) + time`, where `clock` is the
  track's resolved affine map (identity without a clock). Evaluating the
  track at the marker's instant therefore lands exactly on the key. The
  offset may be negative; the resolved key time must be non-negative, like a
  literal key time. Snapping happens after clocks are assigned, then the
  track is re-sorted and re-validated with the same rules and messages as a
  literal track (unique times, handle placement, bounds) at the key's line.

`SrKeyframe` gains a `marker` field: 0, or 1 + an index into the timeline's
reference table. Colour keys carry it on all four channel tracks.

## Names and tags

`name` is kept verbatim (at most 1024 bytes). `tags` is split on XML
whitespace into NMTOKENs (at most 64 per node, 128 bytes each; duplicates are
an error). Both live in an optional per-node `SrNodeTimeline`, allocated only
for nodes that use a B1-5 attribute, so other nodes cost one NULL pointer.
Every B1-5 diagnostic names the node as `group 'id' ("name")`. Safe-area
enforcement (B3) will read the tags.

## Limits

| Limit | Value |
|---|---|
| `SR_MAX_MARKERS` explicit markers | 65536 |
| `SR_MAX_MARKER_LABEL` bytes | 4096 |
| `SR_MAX_GENERATED_MARKERS` generated beats | 1048576 |
| `SR_MAX_BEATS_PER_BAR` | 1024 |
| `bpm` | (0, 1e6] |
| marker `time`, `duration`, grid `offset`, `timeOffset`, `timeGap` | magnitude <= 1e6 |
| `SR_MIN_TIME_SCALE` / `SR_MAX_TIME_SCALE` (each group and the product) | 1e-6 / 1e6 |
| resolved clock offsets, sequence cursors | magnitude <= 1e12 |
| `SR_MAX_NODE_NAME` bytes | 1024 |
| `SR_MAX_NODE_TAGS` / `SR_MAX_TAG_BYTES` | 64 / 128 |
| key references | bounded by the existing 1048576 scene keys |

Every limit is a load error naming the element and attribute.

## Determinism and ownership

No randomness, clock or hash iteration is involved; the marker index is
sorted by `strcmp` and ids are unique, so the order is total. Placement uses
document order (`order`), never pointers. The scene owns the timeline object
and each node owns its `SrNodeTimeline`; `sr_scene_free`/`sr_node_free`
release them on every path. No new file is read, so the scene XML hash stays
the complete resume input. Clock fields are part of the physics cache
signature through the existing track hash. Every frame is still a pure
function of the scene and t: 1 and 4 threads, shuffled, sliced and resumed
renders agree (`frame_order`).

## Tests and cost

- Unit (`timeline_markers`): beat/bar times against the closed form, grid
  counts at exact boundaries, canonical id parsing, lookup and kind errors,
  every limit, clock composition for nested groups and sequences against
  hand-computed maps, all three time bases, key snapping (exact hit at the
  marker instant, negative offsets, re-sorting, duplicate-time errors),
  startMarker/endMarker, names/tags, video speed and particle equivalence
  (timeScale 2 at t equals unscaled at 2t, byte for byte).
- OOM replay of a document using every construct; a seeded fuzz generator
  for the markers section, tags and marker references (valid, mutated,
  truncated).
- Golden `sequence-markers.xml` (320x180, 24 frames at 24 fps): a sequence of
  three animated groups with a gap and an overlap, a timeOffset/timeScale
  group, keys snapped to markers and beats, startMarker/endMarker nodes.
- Cost: O(nodes x registry rows + markers log markers + keys) at load; zero
  per frame. Scenes without B1-5 constructs take one extra tree walk at load
  and are byte-identical (equivalence oracle).

## Review

A read-only Codex review of this note (before code) raised eight points;
the design above incorporates them:

1. B1-1 local-clock correction: kept as a deliberate fix of an unreleased
   B1-1 behaviour (see "Clocks"); no repository scene is affected and the
   oracle is unchanged.
2. Kinematic bodies integrate velocity, not tracks: the physics paragraph is
   corrected and kinematic bodies are rejected under scales with the others.
3. Marker endpoints are kept exact rather than unmapped.
4. Tracks with snapped keys skip parse-time finalization entirely and are
   validated once after resolution (negative offsets, duplicate times after
   snapping, handles, bounds), including colour channels and shared hosts.
5. The particle coordinate contract is spelled out above and tested with
   keyed rates under scale 2 and with a dyadic offset.
6. Derived work: particle admission limits still apply at render time;
   media `speed * scale` is checked finite at load; the zero-cost claim is
   narrowed.
7. Sequence items accept `endMarker`; only `startMarker` is rejected.
8. The clock explanation is corrected; the composition-timebase refinement
   is recorded in `docs/xml-reference.md` next to the B1-1 text.
