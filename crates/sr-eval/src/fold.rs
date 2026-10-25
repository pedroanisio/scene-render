//! Folds of the reactive-values draft (sr-core `srep/srep-0000-reactive-values.md`): a `value` asset with `update`
//! and `step` advances in fixed steps from `start` (a time fold, Semantics 2 and 3), one with `update` and `count`
//! runs its update `count` times at every frame (an index fold, Semantics 4). Time folds of one `step` and `start`
//! that read each other form a system and step together in gather form (Semantics 5).
//!
//! The state after k steps is a function of k alone, so the evaluator keeps checkpoints of each system and steps
//! forward from the nearest one at or before the step it needs: a frame rendered alone and the frame played in
//! sequence give identical values (Semantics 3, the seek check of SREP 14 rule 5).

use crate::eval::{ExprHost, FoldCx, Memo};
use crate::expr::vm;
use crate::program::Program;
use crate::value::Value;
use std::sync::{Arc, Mutex, PoisonError};

/// The most steps a time fold takes up to the composition's end, and the most iterations of an index fold
/// (Semantics 8).
pub const MAX_STEPS: u64 = 1_000_000;

/// Steps between two checkpoints of a system.
const CHECKPOINT: u64 = 1024;

/// A value asset with an `update`.
#[derive(Debug)]
pub struct Fold {
    /// The id the value's failures name.
    pub id: Arc<str>,
    /// The slot of its `value`.
    pub slot: u32,
    /// The compiled `update`, an index into [`Program::exprs`].
    pub update: u32,
    /// Time or index fold.
    pub kind: FoldKind,
}

/// How a fold advances.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FoldKind {
    /// Fixed steps of its system's `step` from its `start`; `linear` interpolates between steps.
    Time {
        /// Index into [`Program::systems`].
        system: u32,
        /// `between="linear"`.
        linear: bool,
    },
    /// `count` iterations at every frame.
    Index {
        /// Iterations.
        count: u64,
    },
}

/// Time folds of one `step` and `start` that read each other: they advance together.
#[derive(Debug)]
pub struct System {
    /// Seconds of composition time of the first step's start.
    pub start: f64,
    /// Seconds between steps.
    pub step: f64,
    /// The folds, indices into [`Program::folds`].
    pub folds: Vec<u32>,
    /// Their `value` slots, in the same order: the reads an update resolves to the state before the step.
    pub slots: Vec<u32>,
    cache: Mutex<Cache>,
}

#[derive(Debug, Default)]
struct Cache {
    /// The states after j · [`CHECKPOINT`] steps, for j = 0, 1, … as far as stepping has reached.
    checkpoints: Vec<Arc<[Value]>>,
    /// The states after the last step computed.
    last: Option<(u64, Arc<[Value]>)>,
}

impl System {
    /// A system of `folds` with their `slots`.
    pub fn new(start: f64, step: f64, folds: Vec<u32>, slots: Vec<u32>) -> System {
        System { start, step, folds, slots, cache: Mutex::default() }
    }
}

/// The number of steps at or before composition time `t` (Semantics 2): 0 before the start, else
/// ⌊(t − start) / step + 10⁻⁹⌋, the tolerance D25 gives `frame`.
pub fn steps_at(start: f64, step: f64, t: f64) -> u64 {
    if t < start {
        return 0;
    }
    libm::floor((t - start) / step + 1e-9) as u64
}

/// `draw(channel, i)` at fold index `index` (Semantics 6): U(seed, channel, pack(index, i, 0)), NaN unless `channel`
/// is a non-negative integer and `i` an integer in [−2²⁰, 2²⁰).
pub fn draw(seed: u64, index: u64, channel: f64, i: f64) -> f64 {
    const LATTICE: f64 = (1u64 << 20) as f64;
    let integer = |x: f64| x.is_finite() && x == libm::trunc(x);
    if !(integer(channel) && channel >= 0.0 && integer(i) && (-LATTICE..LATTICE).contains(&i)) {
        return f64::NAN;
    }
    sr_vector::d24::d24_unit(seed, channel as u64, sr_vector::d24::pack(index as i64, i as i64, 0))
}

/// `random()` in an update at fold index `index` (Semantics 6): D25's draw with the index in place of the frame.
pub fn random(seed: u64, index: u64, site: u32, channel: u64, component: u32) -> f64 {
    sr_vector::d24::d24_unit(seed, index, site as u64 + (channel << 32) + ((component as u64) << 48))
}

/// A state as an update reads it: a list of one number is that number.
pub(crate) fn shown(v: &Value) -> vm::V {
    match v {
        Value::List(l) if l.len() == 1 => vm::V::Num(l[0]),
        v => v.to_v(),
    }
}

fn lerp(a: &Value, b: &Value, u: f64) -> Value {
    let (mut x, mut y) = (Vec::new(), Vec::new());
    if !(a.components(&mut x) && b.components(&mut y)) {
        return a.clone();
    }
    let c: Vec<f64> = x.iter().zip(&y).map(|(x, y)| x + (y - x) * u).collect();
    a.with_components(&c)
}

impl Program {
    /// The value of fold `f` at composition time `comp_t`. A failure (a non-finite state, an update that gives no
    /// number, a step past [`MAX_STEPS`]) is recorded in `memo` and leaves the fold's keyed value.
    pub(crate) fn fold_value(&self, f: u32, comp_t: f64, depth: u32, memo: &Memo) -> Value {
        let fold = &self.folds[f as usize];
        let r = match fold.kind {
            FoldKind::Index { count } => self.index_fold(fold, count, comp_t, depth, memo),
            FoldKind::Time { system, linear } => self.time_fold(fold, system, linear, comp_t, depth, memo),
        };
        r.unwrap_or_else(|e| {
            memo.fail(format!("{}: {e}", fold.id));
            self.unfolded_at(fold.slot, comp_t, depth, memo)
        })
    }

    fn index_fold(&self, fold: &Fold, count: u64, t: f64, depth: u32, memo: &Memo) -> Result<Value, String> {
        let mut v = self.unfolded_at(fold.slot, t, depth, memo);
        let cx = |k: u64| FoldCx { index: k, count, dt: 0.0, system: None };
        for k in 0..count {
            v = self.update(fold, &v, t, cx(k), depth, memo)?;
        }
        Ok(v)
    }

    fn time_fold(
        &self,
        fold: &Fold,
        system: u32,
        linear: bool,
        t: f64,
        depth: u32,
        memo: &Memo,
    ) -> Result<Value, String> {
        let sys = &self.systems[system as usize];
        let n = steps_at(sys.start, sys.step, t);
        let at = sys.folds.iter().position(|&g| self.folds[g as usize].slot == fold.slot).expect("a member");
        let now = self.states(sys, n, depth, memo)?;
        if !(linear && t > sys.start) {
            return Ok(now[at].clone());
        }
        let next = self.states(sys, n + 1, depth, memo)?;
        let u = ((t - sys.start) / sys.step - n as f64).clamp(0.0, 1.0);
        Ok(lerp(&now[at], &next[at], u))
    }

    /// The states of `sys` after `n` steps, from the nearest checkpoint (or the last state) at or before `n`.
    fn states(&self, sys: &System, n: u64, depth: u32, memo: &Memo) -> Result<Arc<[Value]>, String> {
        if n > MAX_STEPS {
            return Err(format!("a time fold takes at most {MAX_STEPS} steps; {n} at this time"));
        }
        let known = {
            let cache = sys.cache.lock().unwrap_or_else(PoisonError::into_inner);
            let last = cache.last.clone().filter(|(m, _)| *m <= n);
            let checkpoint = (cache.checkpoints.len() as u64).checked_sub(1).map(|top| {
                let j = (n / CHECKPOINT).min(top);
                (j * CHECKPOINT, cache.checkpoints[j as usize].clone())
            });
            match (last, checkpoint) {
                (Some(l), Some(c)) => Some(if l.0 >= c.0 { l } else { c }),
                (l, c) => l.or(c),
            }
        };
        let (mut k, mut state) = match known {
            Some(known) => known,
            None => {
                // s₀: each value's keys, expression or link at the start (Semantics 3)
                let s0: Arc<[Value]> = sys.slots.iter().map(|&s| self.unfolded_at(s, sys.start, depth, memo)).collect();
                let mut cache = sys.cache.lock().unwrap_or_else(PoisonError::into_inner);
                if cache.checkpoints.is_empty() {
                    cache.checkpoints.push(s0.clone());
                }
                (0, s0)
            }
        };
        let mut reached = Vec::new();
        while k < n {
            let time = sys.start + (k + 1) as f64 * sys.step;
            let cx = || FoldCx { index: k, count: k + 1, dt: sys.step, system: Some((&sys.slots, &state)) };
            let next: Arc<[Value]> = sys
                .folds
                .iter()
                .zip(state.iter())
                .map(|(&g, v)| self.update(&self.folds[g as usize], v, time, cx(), depth, memo))
                .collect::<Result<_, _>>()?;
            state = next;
            k += 1;
            if k % CHECKPOINT == 0 {
                reached.push((k / CHECKPOINT, state.clone()));
            }
        }
        let mut cache = sys.cache.lock().unwrap_or_else(PoisonError::into_inner);
        for (j, s) in reached {
            if j as usize == cache.checkpoints.len() {
                cache.checkpoints.push(s);
            }
        }
        cache.last = Some((n, state.clone()));
        Ok(state)
    }

    /// One run of `fold`'s update from state `v` at composition time `time`.
    fn update(&self, fold: &Fold, v: &Value, time: f64, cx: FoldCx, depth: u32, memo: &Memo) -> Result<Value, String> {
        let local = Memo::default();
        let mut regs = Vec::new();
        let mut h = ExprHost {
            p: self,
            frame: None,
            comp_t: time,
            t: time,
            slot: Some(fold.slot),
            expr: fold.update,
            value: match shown(v) {
                vm::V::Num(x) => Value::Num(x),
                _ => v.clone(),
            },
            depth: depth + 1,
            memo: &local,
            sample: [0.0; 2],
            fold: Some(cx),
        };
        let r = vm::run(&self.exprs[fold.update as usize].code, &mut h, &mut regs);
        memo.absorb(local);
        let c = r.components().ok_or("the update gave no number")?;
        if c.iter().any(|x| !x.is_finite()) {
            return Err("the update produced a non-finite state".into());
        }
        self.slots[fold.slot as usize].kind.from_v(&r, v).ok_or_else(|| "the update gave no number".into())
    }
}
