# B1-1 design review

Read-only Codex review of `docs/design/b1-1-animation.md` before feature code.
The separate, output-preserving registry refactor was accepted.

Five findings were incorporated:

- Particle rate caches read raw key times/values and assume hold outside the
  track; lifetime culling originally bounds only old curves and Bezier.
  The design now preserves the old path and specifies a bounded evaluated
  rate grid and conservative extended lifetime bounds, with regressions.
- Physics cache signatures omit source XML and hash only legacy key fields.
  New parameters, track modes and resolved clocks must enter the signature,
  with a cache version change and cached/uncached invalidation tests.
- Exact endpoints conflicted with step-start jumps. Right-continuous segment
  selection and loop/ping-pong boundary rules are now explicit.
- Clock ownership, omitted infinite ends and normalized spring seconds were
  ambiguous. The design now gives finite spans, the affine group equation,
  host ownership and conversion back to seconds for spring evaluation.
- Numeric limits were unspecified. Concrete key/count/step/parameter/clock
  limits, preparation failures and legacy-preserving bounds are documented.

Follow-up review found no remaining blocking contradiction and approved the
design for implementation. These are implementation requirements; review of
the finished code and actual regression evidence remain necessary.
