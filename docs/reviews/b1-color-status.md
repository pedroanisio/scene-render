# B1-2 prerequisite: checked color parsing

Base: `0f5be24`. This is the isolated, output-neutral refactor required by
the [styles design](../design/b1-2-styles-metadata.md). It does not enable
tokens, metadata or relative lengths.

`sr_parse_color_status` preserves the existing hex/decimal grammar, arithmetic
and assignment-on-success behavior. Invalid arguments/colors return
`SR_ERR_ARGUMENT`; failure to allocate the decimal parser's temporary copy
returns `SR_ERR_MEMORY`. The existing boolean API wraps this status API and
retains its behavior. No render-time allocation or evaluation changes.

Tests cover exact hex/decimal values, whitespace/exponent forms already
accepted by the parser, invalid components/counts/ranges, null arguments,
unchanged output on failure, and allocation-failure replay for RGB/RGBA plus
the boolean wrapper. Each decimal parse has one intercepted allocation;
its failure returns the expected status and leaks nothing.

## Review

The read-only reviewer approved the production refactor. One test-harness
finding was reproduced before fixing it: after `replay_until_success` returned,
the OOM debug label still borrowed its stack buffer. The new wrapper replay
exposed it with `SR_OOM_BACKTRACE=1` and
`ASAN_OPTIONS=detect_leaks=0:detect_stack_use_after_return=1`.
ASan reported stack-use-after-return in `oom_should_fail`. The helper now clears
the borrowed label before returning, and the manual replay uses a static label.
The same ASan command then passed all eight OOM tests. Follow-up review found
no remaining issues.

## Verification

All builds and C tests run in Flatpak SDK 25.08. Release, ASan and coverage
builds emit no compiler warnings.

- Release CTest: 66/66, including all 17 goldens' frame-order checks.
- Final Release color/OOM rerun after the debug-label fix: 2/2.
- ASan/UBSan full CTest: 66/66, including frame-order checks.
- Coverage CTest: 66/66. Lines 91.28%, branches 73.82%; existing floors
  89.20% / 71.40% pass unchanged.
- Byte oracle against `/tmp/scene-render-b1-hosts-verified`: 309 previews
  across 42 old fixtures, three encodes and two expected rejections match.
  The first attempt stopped because the new worktree lacked ignored generated
  example assets; copying the existing fixtures allowed the complete rerun.
- Performance: no render-path change. The batch's existing strict performance
  gate remains open; this refactor does not update its baseline or claim it
  passed.

Local logs: `/tmp/b1-color-status-{release,asan,coverage}-build.log`,
`/tmp/b1-color-status-{release,asan,coverage}-test.log`,
`/tmp/b1-color-status-release-final.log`,
`/tmp/b1-color-status-oracle.log`, and the failing/fixed review reproductions
`/tmp/b1-color-status-review-{repro,fixed}.log`.

B1-2 feature implementation and the remaining batch items are still required.
No main-checkout merge or push is part of this prerequisite commit.
