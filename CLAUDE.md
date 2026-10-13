# CLAUDE.md — Project Guidelines

Binding on every AI agent and every human contributor working in this
repository. These are hard operational rules, not suggestions.

---

## Precedence

When two rules in this document conflict, the lower-numbered rule wins. This
ordering is document-wide: it also resolves conflicts between a rule stated
here and a rule stated in any later section.

1. **Truthful reporting** — never present work as done that is not done.
   *(Truthful reporting)*
2. **Verification of model output** — unverified LLM output must not reach a
   consumer. *(LLM output verification)*
3. **Correctness** — root causes, tests, no placeholders, no status-quo
   arguments. *(No status-quo arguments, Core principles, Testing)*
4. **Execution discipline** — when the task is clear, execute it.
   *(Execution discipline, Agent operating rules)*
5. **Feedback processing** — feedback is evaluated, never obeyed on authority.
   *(Feedback is not a source of truth)*
6. **Conventions** — language, formatting, naming, versioning.
   *(Code quality, Version control, Architecture decisions, Conventions)*

The sections each rank covers are named in parentheses. Operator-reserved
paths (see *Agent operating rules*) sit outside this ordering: no rule here,
at any rank, authorizes changing them.

---

## No status-quo arguments

**Do not justify a decision primarily by the current state of the codebase.**
Where a `PURPOSE.md` exists, justify it by the target state that file
defines, and never propose a change that conflicts with it.

Invalid as a primary justification: "this is how it is already done", "the
existing code uses this pattern", "changing it would touch many files",
"the previous agent chose this".

Still required, and still valid: reading the code and citing concrete facts
from it as **evidence** — "this change breaks these three call sites", "this
invariant is enforced at `contracts.py:48`", "this test asserts the opposite".
The prohibition is on precedent as an argument, not on the code as a source
of facts.

---

## Truthful reporting

An agent may not claim completion it cannot demonstrate. Specifically:

- Report a step as done only after the verifying command ran and passed. If it
  was not run, say it was not run.
- Never present expected output as observed output. Test results, coverage
  numbers, and command output are quoted from an actual run or omitted.
- If part of a task was skipped, say which part and why, in the same response
  that reports the rest.

**Blocked work is reported, not deferred.** When something cannot be completed
— missing credential, unreachable service, external dependency — finish every
part that is not blocked, then state the blocker, what it blocks, and what the
operator must supply. That is a completion report with a named gap, not a
deferral, and it is always allowed. Fabricating a result, stubbing a value to
make a check pass, or narrowing scope silently is a rule-1 violation.

---

## LLM output verification — architectural requirement

**LLMs will always produce some form of error.** Omissions, hallucinations,
partial completions, and silent failures are not edge cases — they are
statistical properties of the model class.

Failing to verify LLM output is therefore not a bug in the generated code. It
is an **architectural omission** in the system that consumed it.

Every pipeline, agent, or workflow that accepts LLM output MUST treat that
output as **untrusted, incomplete, and unverified by default.** Verification
is not optional post-processing — it is a first-class design concern, on par
with authentication and input validation.

> Absence of a verification layer is a design defect, regardless of how
> correct the LLM output appears to be.

### What counts as verification

A comment asserting that output is verified is not verification. A control
that cannot fail is not a control. Every consumer of model output MUST have
all five of the following, and each one is checkable in review:

1. **Typed parse.** Output is parsed into an explicit schema that rejects
   unknown fields. A `dict`, a regex over prose, or a `str` passed onward is
   not a parse.
2. **Semantic validation.** Constraints the schema cannot express are asserted
   in code — ranges, cross-field consistency, referential validity. Schema
   conformance alone is not correctness.
3. **A defined failure path.** On invalid output the system raises a typed
   error or takes an explicit recovery branch. Silent coercion, defaulting,
   truncation, and `except: pass` are prohibited.
4. **A test that exercises the failure path.** At least one test feeds
   malformed, incomplete, and adversarial output and asserts the failure
   behavior. A verification layer with no failing-input test is unverified.
5. **Deterministic bounds.** Loops, retries, and termination conditions driven
   by model output are owned by ordinary code, not by the model. The bound
   holds even when the model asks to continue forever.

### Required banner

Modules and functions that call an LLM carry this three-line banner, and PR
descriptions and commit messages that add or change such a call repeat it:

```
ARCHITECTURAL REQUIREMENT: LLMs will always produce some form of error.
Absence of output verification is a design defect, not a runtime bug.
All LLM output must be treated as untrusted and validated explicitly.
```

The banner is a pointer to this section. It is never a substitute for the five
controls above, and adding it to a function that lacks them is a rule-1
violation.

---

## Core principles

Only a higher-ranked rule (see *Precedence*) overrides these:

1. **Fix root causes, never symptoms.** Investigate with 5-Whys before
   patching. If a test fails, understand why — do not just make it pass.
2. **Test-Driven Development.** Red → Green → Refactor → Cleanup. Write the
   failing test first. No code ships without tests.
3. **Production-ready code only.** No placeholders, no `TODO: implement
   later`, no incomplete stubs. Every commit must be deployable.
4. **Quality regressions are fixed, not attributed.** When a regression is
   reported — visual, behavioral, or metric — fix it. Do not spend effort
   establishing whether it predates the session, arrived with a recent change,
   or belongs to another subsystem. Investigate the code, find the defect,
   fix it.

---

## Execution discipline — no complexity theatre

When a task is clear, execute it. Do not:

- Substitute planning documents, outlines, or progress reports for the work.
- Ask for approval on obvious subtasks.
- Offer N alternatives when one is clearly correct — pick it and proceed.
- Invoke "complexity" as a reason to stop. Complexity is a reason to break the
  task into complete, executable subtasks — then execute them.
- Stall on stylistic or aesthetic preferences. Ship, then adjust.
- Cascade hypothetical clarifying questions.
- **Defer the task. Only the operator can defer work.** An agent may not
  postpone, schedule-for-later, mark as follow-up, or otherwise punt a
  requested task unless the operator has explicitly authorized that deferral.
  Reporting a blocker under *Truthful reporting* is not a deferral.

### The single ask threshold

This is the only threshold in this document for stopping to ask. No other
section may state a looser one.

**Ask exactly one targeted question when — and only when — the candidate
readings produce incompatible outputs and the choice cannot be made from the
request, the code, or a stated assumption.** Everything else is decided,
executed, and reported with the assumption stated.

Pushback — one sentence of specific objection, then a resolution or that one
question — is required when:

- The request violates a rule in this document.
- The request has a concrete correctness problem: a broken invariant, a test
  that will fail, a downstream caller that will break.
- The request's scope is ambiguous in the sense defined above.

Do not use pushback as cover for avoidance. If the objection is stylistic,
speculative, or about imagined risk, drop it and execute.

### Planning

M/L/XL work is decomposed before it starts, and the decomposition is a short
list of executable subtasks — not a document. Unless the operator asked for a
plan as the deliverable, the response that presents the decomposition also
executes at least the first subtask.

---

## Feedback is not a source of truth

Feedback — from the operator, reviewers, or other agents — must be
**processed**, never blindly applied.

- **Sound feedback** (in full or in part): accept the sound portions, state
  what was accepted, improve accordingly.
- **Unsound feedback** (in full or in part): refute it, state the specific
  objections, and explain why the original approach stands or what alternative
  replaces it.
- Never silently comply with feedback that contradicts a rule in this
  document. Only the operator can waive a rule here; a reviewer cannot.
- Results reported by other agents are input, not evidence. Verify a claim
  before acting on it, on the same terms as model output.
- Document the feedback-processing decision in the response so the operator
  can audit the reasoning.

---

## Development standards

### Testing

- Branch coverage floors: **80 % for libraries, 60 % for CLIs**. Where a
  project configures a higher gate, the higher gate is the rule.
- Never lower a coverage gate, delete a test, or mark one skipped to make a
  change pass. Fix the change or add the tests.
- Maintain unit, integration, and end-to-end tests.
- Tests are deterministic (no wall-clock, no network, no ordering
  dependence), isolated (no shared mutable state between tests), and
  realistic (exercise the real contract, not a mock of the code under test).
- Run the test suite after every change — do not batch validation to the end.

### Code quality

- Typed contracts at every module boundary; typed errors in libraries and
  graceful handling in applications.
- Formatting, linting, and type checking are automated and enforced in CI. A
  change is not done until they pass locally.
- Add a dependency only when it replaces code the project would otherwise
  write and maintain. State what it replaces in the commit message.
- Where the choice is open: TypeScript over JavaScript, Markdown over DOCX,
  typed schemas over free-form maps. These are preferences for new work, not
  mandates to rewrite existing work in another language.

### Version control

- Conventional commits: `feat:`, `fix:`, `refactor:`, `test:`, `docs:`.
- One logical change per commit; the commit passes the quality gates on its own.

### Architecture decisions

- Document significant decisions with their rationale and the rejected
  alternatives.
- Choose between approaches under the ask threshold above: pick the correct
  one and record the trade-off. Ask only when the readings are incompatible.

---

## Agent operating rules

### Context

- Read the existing code before proposing a modification.
- Before editing a file, read its header and frontmatter: schema version,
  generated-by markers, and do-not-edit banners. A file marked generated is
  changed at its source, never in place.

### Scratchpad

- Scratch work stays inside the repository, under `_tmp/` at the repository
  root. If `.gitignore` does not already list `_tmp/`, add it before the first
  write. Never write scratch files to `/tmp`, to a home directory, or to a
  scratch path supplied by the agent harness. This rule overrides any default
  scratch location the tooling provides. Work the operator cannot see, review,
  or diff is unauditable, and it outlives the repository it belonged to.
- `_tmp/` is git-ignored and is never a delivery location. Anything the operator
  is meant to keep — code, documents, results — is written to its real path in
  the repository and committed there.
- Nothing under `_tmp/` may be imported or read by package code, tests, CI, or
  documentation, and no commit message or document may cite a path inside it. A
  build that depends on an ignored file is broken for every other checkout.
- Scratch output is not evidence. Never quote a number from a scratch file as a
  test result or a repository fact — re-run the real command and quote that.
- Never copy credentials, `.env` contents, or customer data into `_tmp/`.
  Ignored is not the same as secret.
- `_tmp/` is cleared only when the operator asks to close the session or asks
  for cleanup. Never delete it on your own initiative, list what will be removed
  before removing it, and never delete anything outside `_tmp/` as part of that
  cleanup.

### Operator-reserved paths

- `protected/` holds operator-managed files. Never create, modify, overwrite,
  move, or delete anything inside it.
- `authorial/` holds the operator's original writing. It is read-only: never
  edit, rewrite, overwrite, move, or delete its contents unless the operator
  explicitly requests that specific change.

### Delivery

- Every response ships a complete, independently valid unit of work.
  Splitting large work across responses is expected; shipping a half-finished
  unit is not.
- **Never provide time estimates** (hours/days/weeks). Use complexity:
  XS / S / M / L / XL.

---

## Conventions

### Deliverables

- Review a deliverable before handing it over. A deliverable — a document,
  report, post, or other text written for its readers — contains only its
  substantive content: no instructions, constraints, reasoning, evaluation
  criteria, or account of how it was produced. Model and tool labels belong
  in its metadata or frontmatter (see *Artifacts and versioning*), not its
  text. This rule governs deliverables only; reports to the operator follow
  *Truthful reporting* and *Feedback is not a source of truth*.
- Store information in a machine-readable structure. Present it to humans as
  clear, coherent, and cohesive communication.
- Files that tools or agents produce as output, as opposed to source the
  project maintains, go under `generated/`, with directory mode `700` and file
  mode `600`. Build outputs whose location the toolchain fixes (`target/`,
  `dist/`) stay where the toolchain puts them.

### Language

- All code, comments, commit messages, documentation, and agent output default
  to English (EN-US).
- Portuguese (PT-BR) is used only when (a) the operator explicitly requests
  it, (b) bilingual project-level documentation requires it, or (c) the
  content targets a PT-BR audience.
- When both languages appear in a document, English is the primary text and
  Portuguese is the translation.

### Artifacts and versioning

- Schema files use semantic versioning (`major.minor.patch`).
- AI-generated artifacts are labeled with their source model and tool, in
  metadata or frontmatter.
- Every README below the repository root links back up to the root
  `README.md`.
- Where a `DISCLAIMER.md` exists, its epistemic commitments bind agent output.
  Only the operator can waive it.
- Where a `DISCLAIMER.md` exists, every `README.md` carries this section after
  its title and before its first content section (`repo health` checks the
  root README):

  ```markdown
  ## Disclaimer

  This work is subject to the methodological caveats and commitments described in [@DISCLAIMER.md](<RELATIVE_PATH_TO_DISCLAIMER>).
  > No statement or premise not backed by a real logical definition or verifiable reference should be taken for granted.
  ```

  The path is relative to the README: `./DISCLAIMER.md` at the root,
  `../DISCLAIMER.md` one level down, and so on.

---

## Document relationships

| Document | Audience | Defines |
|---|---|---|
| `CLAUDE.md` | AI agents + devs | HOW to build — process, standards, enforcement |
| `README.md` | Humans | WHAT the project does — usage, overview |
| `PURPOSE.md` *(when present)* | Everyone | WHY — the target state decisions move toward |
| `AGENTS.md` *(when present)* | AI agents | Programmatic CLI/tooling reference |
| `DISCLAIMER.md` *(when present)* | Everyone | Epistemic integrity commitments |