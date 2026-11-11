---
disclaimer: >
  No information within this document should be taken for granted.
  Any statement or premise not backed by a real logical definition or
  verifiable reference may be invalid, erroneous, or a hallucination.
  This analysis represents one model's assessment of leverage and
  novelty — it may be wrong. Verify independently.
title: "Step-Change Contribution: rs-scene-render"
date: 2026-11-11
method: step-change-contributor/v1
source_document: "pedroanisio/scene-render at 503e9e4 (README.md, crates/, tools/, .github/workflows/ci.yml)"
generated-by: AI-assisted
tool: Claude Code
---

# Step-Change Contribution: rs-scene-render

rs-scene-render states that evaluation is bit-identical across platforms because every
transcendental function comes from the `libm` crate. The source does not satisfy that
condition: the evaluator and the simulation crate call the operating system's math library
in 170 places, and no CI job compares evaluator or simulation output across operating
systems. The highest-leverage improvement is a determinism contract that names which
stages are bit-exact, forbids platform math in those stages with a source gate, and proves
the result with digests compared on Linux, macOS and Windows.

## 1. Gap Analysis

### 1.1 Structural map

The repository is an 18-crate Rust workspace at version 0.9.0. A scene-render 1.1 XML
document passes through five stages:

| Stage | Crates | Output |
|---|---|---|
| Load | `sr-model` | a typed, validated model or a complete list of coded diagnostics |
| Evaluate | `sr-eval`, `sr-sim`, `sr-wasm` | a FrameGraph per frame, simulation state per step |
| Prepare geometry | `sr-vector`, `sr-text`, `sr-geo`, `sr-space`, `sr-3d`, `sr-volume` | CPU-side paths, glyphs, meshes, projections |
| Composite | `sr-gpu` | pixels in linear light through wgpu |
| Deliver | `sr-media`, `sr-audio`, `sr-deliver` | encoded video, audio, stills |

The claims of the project rest on one another in this order: validation guarantees that
later stages read a resolved model; determinism guarantees that a frame is a function of
the document and the time; the test, evidence, oracle, fuzz and performance jobs in
`.github/workflows/ci.yml` supply the evidence. `DISCLAIMER.md` sets the standard for all
of them: a claim without a test, a reference value or a derivation is unverified.

### 1.2 Strengths

- **Validation is total.** A document loads fully or fails with every problem it
  contains, each with a stable code, position and path (README, opening section).
- **Evidence is classified.** `tools/evidence.py` renders 24 probe scenes and classifies
  each as native, degraded, wrong, error or gap, and any change to an accepted outcome
  fails the run.
- **Tolerances are stated.** Golden frames require PSNR ≥ 50 dB and ΔE2000 ≤ 1; the
  OpenGL and native backends must agree to PSNR ≥ 40 dB (README, *Performance and
  verified platforms*).
- **Unverified territory is named.** The README states that the checks do not establish
  visual equivalence on Metal, DirectX 12, discrete GPUs or hardware encoders.
- **Temporal determinism is tested.** Simulations checkpoint every simulated second and
  tests require bit-exact replay in any frame order on one machine.
- **External inputs are pinned.** Generated media, tile archives and physics caches are
  refused unless their SHA-256 equals `cacheSha256`.
- **Policy is enforced by source scans.** `tools/tests/test_panic_patterns.py`,
  `test_unsafe_policy.py` and `test_crate_layers.py` turn conventions into failing tests.

### 1.3 Gap inventory

| G-ID | Dimension | Description | Severity | Evidence |
|---|---|---|---|---|
| G1 | Mechanism | The stated mechanism for cross-platform bit-identity (all transcendentals from `libm`) is absent from most of the code it describes. | critical | `README.md` line 123; `crates/sr-eval/src/curve.rs` line 10 repeats the claim; `sr-eval` has 67 and `sr-sim` 103 calls to platform math methods (section 3.2) |
| G2 | Falsifiability | No test can observe a cross-platform difference in evaluation or simulation. | critical | `.github/workflows/ci.yml` line 369 runs only `cargo test -p sr-model -p sr-eval -p sr-resolve --lib` on macOS and Windows; the digest assertions in `crates/sr-sim/src/fluid.rs` line 667 and `slime.rs` line 252 run on Linux only |
| G3 | Formalization | "Deterministic" names three different guarantees (bit-exact evaluation, bit-exact replay, tolerance-bounded pixels) under one word. | moderate | The README's first sentence says "deterministic"; the golden thresholds in the same file are tolerances |
| G4 | Boundary | The crates between evaluation and the GPU (`sr-geo`, `sr-space`, `sr-3d`, `sr-vector`) have no stated determinism level. | moderate | 84, 76, 143 and 16 platform math calls respectively, no claim in either direction |
| G5 | Boundary | Visual equivalence on Metal, DirectX 12 and discrete GPUs is unverified. | moderate | README, *Verified platforms* |
| G6 | Temporal | The README is one 300 KB file whose prose is mostly unchecked. | minor | `DISCLAIMER.md`: "most prose is not" checked |
| — | Abstraction, Cross-domain, Asymmetry | No significant gap found. | — | — |

### 1.4 Honesty check

- G5 is **[not addressable from the repository]**: it requires rendering on the target hardware,
  and the README already says so.
- G6 is **[possibly intentional]**: a single README keeps `tools/doc_facts.py` and
  `tools/doc_links.py` simple.
- G4 is **[possibly intentional]** for the GPU path, whose output is bounded by a
  tolerance in any case. It is unexamined for geometry that feeds discrete decisions.
- G1 and G2 are neither: the claim is explicit, and the code and CI contradict it.

## 2. Leverage Ranking

### 2.1 Candidate contributions

| C-ID | Contribution | Addresses | Form | Source domain | Novelty |
|---|---|---|---|---|---|
| C1 | A determinism contract: stage-level guarantees, a source gate against platform math, and digests compared on three operating systems | G1, G2, G3, G4 | formal model with an enforcement design | reproducible builds; IEEE 754 numerical reproducibility | The claim reads as already satisfied from inside the project, so no reviewer looks for its absence |
| C2 | Cross-platform digests only, with no source gate | G2 | CI design | — | An expected next step once G2 is seen |
| C3 | A definition of the three determinism levels in the README | G3 | documentation | — | Clarifies wording, enforces nothing |
| C4 | A Metal and DirectX 12 golden-frame harness | G5 | test design | — | Already named as missing by the project |
| C5 | The README split by subsystem with fact checks per file | G6 | restructuring | — | Routine |

### 2.2 Falsifiability gate

- **C1 — [Falsifiable].** If digests of FrameGraphs and simulation state computed from
  the unmodified source on Linux x86-64, macOS arm64 and Windows x86-64 are all equal,
  the contract's premise fails.
- **C2 — [Falsifiable].** The same observation.
- **C3 — [Reformulable].** A definition alone predicts nothing. Reformulated as "each
  stage's guarantee is asserted by a named test", it merges into C1.
- **C4 — [Falsifiable].** Golden frames on Metal within PSNR ≥ 50 dB would show the
  harness finds nothing.
- **C5 — [Unfalsifiable].** Eliminated: no observation refutes a reorganisation.

### 2.3 Scoring and selection

| C-ID | Impact | Uniqueness | Realizability | Leverage |
|---|---|---|---|---|
| C1 | 4 | 3 | 4 | 48 |
| C2 | 3 | 2 | 4 | 24 |
| C3 (merged into C1) | 2 | 2 | 5 | 20 |
| C4 | 4 | 2 | 1 | 8 |

C1 is selected. Its realizability is 4 because the design and the Linux measurements are
complete here, while the macOS and Windows digests require CI runners.

## 3. The Contribution

### The determinism contract

#### 3.1 Definitions

A **platform** π is a triple (target triple, system math library, GPU adapter and driver).
For a document *d* and a time *t*, write E<sub>π</sub>(*d*, *t*) for the FrameGraph,
S<sub>π</sub>(*d*, *k*) for the simulation state after step *k*, and
R<sub>π</sub>(*d*, *t*) for the composited pixels.

- **Portable — bit-exact across platforms.** For all platforms π₁, π₂:
  bytes(E<sub>π₁</sub>(*d*, *t*)) = bytes(E<sub>π₂</sub>(*d*, *t*)), and the same for S.
- **Repeatable — bit-exact on one platform.** For a fixed π, the result is independent of frame
  order, thread count and run. The simulation replay tests already assert repeatability.
- **Bounded — within a tolerance across platforms.** R<sub>π₁</sub> and R<sub>π₂</sub> agree within a
  stated tolerance. The golden and backend tests already assert boundedness for lavapipe and
  OpenGL at PSNR ≥ 50 dB and ≥ 40 dB.

`README.md` line 123 claims portability for evaluation. The headline word "deterministic" is then the
conjunction: portable load, evaluation and simulation; repeatable everywhere; bounded pixels.

#### 3.2 Why the code is not portable

**A stage is portable when every floating-point primitive in the stage is a platform-independent
function.** IEEE 754 requires correct rounding for addition, subtraction, multiplication,
division, square root and fused multiply-add, so those return the same bits everywhere,
and Rust neither reorders nor contracts them. IEEE 754-2019 clause 9.2 only recommends
correct rounding for transcendental functions, and system math libraries implement them
differently. Rust's `f64::sin`, `cos`, `exp`, `ln`, `powf`, `hypot`, `atan2`, `cbrt` and
their relatives call the system library; the standard library documentation for each of
these methods states that their precision is non-deterministic and varies by platform and
Rust version. A portable stage therefore needs every such call routed to one software
implementation, which is what the `libm` crate provides.

**The stages claimed portable call the system library.** This command counts calls to those methods in
each crate's `src` tree (comment lines excluded, `#[cfg(test)]` modules included):

```sh
P='\.(sin|cos|tan|exp|exp2|exp_m1|ln|ln_1p|log|log2|log10|powf|atan|atan2|asin|acos|sinh|cosh|tanh|asinh|acosh|atanh|cbrt|hypot|sin_cos)\('
for c in sr-eval sr-sim sr-vector sr-text sr-geo sr-space sr-3d; do
  echo "$c $(grep -rhE "$P" crates/$c/src --include=*.rs | grep -vcE '^\s*//')"
done
```

At commit 503e9e4 it prints:

| Crate | Platform math calls | `libm::` calls | `powi` calls |
|---|---|---|---|
| `sr-eval` | 67 | 98 | 45 |
| `sr-sim` | 103 | 0 | 30 |
| `sr-vector` | 16 | 41 | 5 |
| `sr-text` | 2 | 18 | 3 |
| `sr-geo` | 84 | 0 | 0 |
| `sr-space` | 76 | 0 | 10 |
| `sr-3d` | 143 | 0 | 25 |

Shipped call sites include keyframe interpolation in rapidity
(`crates/sr-eval/src/channel.rs` lines 263–264, `atanh` and `tanh`), boid initial velocities and
the neighbour test (`crates/sr-sim/src/flock.rs` lines 68–69 and 287), and fluid dissipation
(`crates/sr-sim/src/fluid.rs` lines 521–522). `sr-sim` does not depend on `libm` at all.

**Two math implementations disagree on a measurable share of inputs.** The program in the
appendix compares Rust's standard methods with `libm` 0.2.16 bit for bit on one million
pseudo-random arguments per function. On `x86_64-unknown-linux-gnu` with glibc 2.39 and
cargo 1.97.0 it prints:

| Function | Results that differ, of 1,000,000 |
|---|---|
| `sin` | 31,190 |
| `cos` | 31,241 |
| `exp` | 97,413 |
| `tanh` | 77,386 |
| `ln` | 34,229 |
| `powf(x, 2.4)` | 99,989 |
| `hypot` | 141,248 |
| `atan2` | 170,040 |
| `cbrt` | 0 |
| `sqrt` | 0 |

`sqrt` agrees on every input because IEEE 754 fixes its result; the transcendental
functions disagree on 3 % to 17 % of inputs. The measurement shows that glibc and the
`libm` crate differ. It does not show that glibc, Apple's libm and the Windows UCRT differ
from one another on the inputs the corpus produces; section 3.5 tests that directly.

#### 3.3 Why a one-ulp difference reaches the picture

A difference in the last bit of one result changes output through three mechanisms.

1. **Discrete decisions.** A comparison, a `floor` or a sort turns a one-ulp difference
   into a different branch. `flock.rs` line 287 counts a neighbour when
   `hypot(Δx, Δy) < 5.0`; the permutation rule in the README swaps index *i* with
   ⌊u · (i + 1)⌋.
2. **Iteration.** A fixed-step simulation applies the same map thousands of times. For a
   system with largest Lyapunov exponent λ > 0, an initial difference ε₀ grows as
   ε₀ · e^(λt). **[Assumption introduced by contributor]** The flock, rigid-body and
   fluid solvers are chaotic in this sense for typical parameters. With
   ε₀ = 2⁻⁵³ ≈ 1.1 × 10⁻¹⁶, the difference reaches order one at t ≈ 36.7 / λ, so a system
   with λ = 2 s⁻¹ diverges visibly after about 18 simulated seconds.
3. **Digests.** SHA-256 of a cache or a FrameGraph changes when any bit changes.
   `scene-render simulate` prints `cacheSha256`; a second operator who runs it on another
   platform for the same document obtains a different digest whenever one stored value
   differs.

A physics cache pinned by `cacheSha256` already carries one platform's result to every
other platform. Documents that simulate at render time have no such protection, and
parallel segment rendering across unlike machines joins frames from different
trajectories.

#### 3.4 The contract

| Stage | Crates | Level | Enforced by |
|---|---|---|---|
| Load | `sr-model` | portable | existing corpus tests, run on three operating systems |
| Evaluate, simulate | `sr-eval`, `sr-sim`, `sr-wasm` | portable | source gate (3.5 A) and cross-platform digests (3.5 B) |
| Prepare geometry | `sr-vector`, `sr-text`, `sr-geo`, `sr-space`, `sr-3d`, `sr-volume` | portable, reached crate by crate under the ratchet | source gate with a ceiling per crate |
| Composite | `sr-gpu` | repeatable on one adapter, bounded across adapters | existing golden and backend tests |
| Deliver | `sr-media`, `sr-audio`, `sr-deliver` | repeatable with a fixed FFmpeg build | existing round-trip tests |

Geometry preparation belongs in the portable level because its results feed discrete decisions: tile
selection, tessellation counts, sort order and culling.

#### 3.5 Enforcement

**A. Source gate — S.** A new test, `test_float_portability`, placed in `tools/tests/` in the pattern of
`test_panic_patterns.py`, scans each portable crate's `src` tree for the methods in 3.2 and
compares the count with a ceiling in a new file `float_portability.json` under `tools/`. The ceilings start at
the counts in the table above. A count above its ceiling fails; a count below it fails
until the ceiling is lowered to match, so the number only falls. `sr-eval`, `sr-sim` and
`sr-model` start with the target ceiling of zero as soon as step C lands for them.

**B. Cross-platform witness — M.** A test computes SHA-256 of the serialized FrameGraph of
every document in `tests/corpus/valid` at its start and its middle, and of the state of
each `sr-sim` solver after a fixed number of steps, and compares them with a checked-in
digest file. The `portability` job in `.github/workflows/ci.yml` runs this test and
`cargo test -p sr-sim --lib` on `macos-14` and `windows-2022`; the Linux `tests` job runs
it as well. One digest file serves all three, so any disagreement fails a named document
on a named platform.

**C. Migration — L, in three subtasks.** (1) `sr-eval`: replace the 67 calls with their
`libm` equivalents, as `curve.rs` and `rng.rs` already do. (2) `sr-sim`: add the `libm`
dependency and replace the 103 calls. (3) Geometry crates, one per change, each lowering
its ceiling to zero. `powi` stays until the witness decides: it is repeated multiplication
supplied by the compiler runtime, and the witness shows whether it varies.

**D. Wording — XS.** `README.md` line 123 and `curve.rs` line 10 state the guarantee the gate
enforces, with the crate list, and the README's first paragraph defines "deterministic" as
the conjunction in 3.1.

Steps A and B land before C. B run on the unmodified source is the experiment that decides
how urgent C is.

**[Falsification criterion]:** If the witness of step B, run on the source at commit
503e9e4 with no migration, produces identical FrameGraph digests for every valid corpus
document and identical `sr-sim` state digests on `ubuntu-24.04` (x86-64), `macos-14`
(arm64) and `windows-2022` (x86-64), then the claim that platform math breaks portability in
practice is refuted, and this contribution reduces to guarding a property that already
holds.

## 4. Justification

### 4.1 Step-change argument

The contract converts the project's headline property from an assertion into a tested
invariant. Today the portability claim rests on a sentence, and a contributor who writes
`angle.sin()` in `sr-eval` passes formatting, clippy, the test suite and review. With the
gate, the same line fails CI with the name of the rule; with the witness, a platform
difference fails with the name of the document.

The improvement is non-linear because portability is a universally quantified property: a single
platform-dependent call on a reachable path falsifies it for every document that reaches
the path. Removing half of the calls leaves the property false. Only a gate at zero and a
witness across platforms make it true, and from that point every later change preserves
it at no further cost.

Portability enables three uses that are unsafe today. Frame ranges rendered on unlike machines can
be joined, because every machine evaluates the same FrameGraph. A FrameGraph digest
becomes a cache key that is valid across a render farm. A reported defect can be
reproduced from the document and the time alone, on any operating system, up to the GPU
stage.

### 4.2 Leverage audit

- **Impact (4):** delivered. The contribution corrects a claim in the README's evaluation
  section and supplies its enforcement.
- **Uniqueness (3):** delivered at 3. The finding needs no knowledge from another field,
  only a comparison of the claim with the source; its value comes from the claim reading
  as satisfied.
- **Realizability (4):** delivered at 4. The counts and the Linux measurement are
  observed. The cross-platform divergence is predicted, and the gate and witness are
  specified without being implemented.

### 4.3 Falsifiability audit

- **Falsification criterion:** identical digests on all three operating systems from the
  unmodified source refute the practical claim.
- **Test sketch:** implement step B alone on a branch, record the digests on Linux, and
  let the `portability` matrix compare them on `macos-14` and `windows-2022`. A green run
  on both refutes; a red run names the first diverging document or solver.
- **Strength of test: Strong.** One all-green run refutes the core claim. The structural
  claim, that platform math calls exist where the README says they do not, is already
  established by the count in 3.2.

### 4.4 Limitations and caveats

- The contract covers no GPU output. Equivalence on Metal, DirectX 12 and discrete GPUs
  stays unverified.
- The counts include `#[cfg(test)]` modules, so the number of shipped call sites is lower
  than the totals. The gate should scan shipped code only, or tests should use `libm` too.
- A double-precision difference of one ulp usually disappears when the value is cast to
  `f32`, as in `fluid.rs` line 521. Some solvers may therefore produce equal digests across
  platforms while others diverge.
- **[Assumption introduced by contributor]** `libm`'s software implementations are slower
  than glibc's. The 2 ms evaluation budget in `crates/sr-eval/tests/perf.rs` and
  `tools/perf_gate.py` measure the cost; a regression past their thresholds requires
  choosing between speed and portability for the affected path.
- Dependencies are outside the gate. The rigid-body engine and any crate that calls
  platform math internally need the witness to expose them.
- Portability also depends on the compiler: a future Rust or LLVM release that changes `powi` or
  constant folding would move digests on every platform at once, which the witness
  reports as a digest update on a toolchain change.

## Appendix: comparison program

`Cargo.toml` depends on `libm = "=0.2.16"`; build with `cargo run --release`.

```rust
fn main() {
    let n = 1_000_000u64;
    let mut s = 0x9E3779B97F4A7C15u64;
    let mut next = || {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (s >> 11) as f64 / (1u64 << 53) as f64
    };
    let xs: Vec<f64> = (0..n).map(|_| next() * 200.0 - 100.0).collect();
    let ps: Vec<f64> = (0..n).map(|_| next() * 100.0 + 1e-9).collect();
    macro_rules! cmp1 {
        ($name:literal, $std:expr, $lm:expr, $v:expr) => {{
            let d = $v.iter().filter(|&&x| ($std)(x).to_bits() != ($lm)(x).to_bits()).count();
            println!("{:8} {:7} of {} differ", $name, d, n);
        }};
    }
    cmp1!("sin", f64::sin, libm::sin, xs);
    cmp1!("cos", f64::cos, libm::cos, xs);
    cmp1!("exp", |x: f64| (x * 0.1).exp(), |x: f64| libm::exp(x * 0.1), xs);
    cmp1!("tanh", |x: f64| (x * 0.05).tanh(), |x: f64| libm::tanh(x * 0.05), xs);
    cmp1!("ln", f64::ln, libm::log, ps);
    cmp1!("cbrt", f64::cbrt, libm::cbrt, ps);
    cmp1!("powf2.4", |x: f64| x.powf(2.4), |x: f64| libm::pow(x, 2.4), ps);
    cmp1!("sqrt", f64::sqrt, libm::sqrt, ps);
    let d = xs.iter().zip(&ps).filter(|(&a, &b)| a.hypot(b).to_bits() != libm::hypot(a, b).to_bits()).count();
    println!("hypot    {:7} of {} differ", d, n);
    let d = xs.iter().zip(&ps).filter(|(&a, &b)| a.atan2(b).to_bits() != libm::atan2(a, b).to_bits()).count();
    println!("atan2    {:7} of {} differ", d, n);
}
```
