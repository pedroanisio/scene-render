#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Coverage gate, independent of CTest's pass/fail: configures an
# instrumented build (-DSR_COVERAGE=ON), runs the whole test suite in it,
# then fails when line or branch coverage of src/ drops below the floors.
#
# usage: tools/coverage-gate.sh [BUILD_DIR]     (default: build/coverage)
#
# The floors are the coverage measured when the gate was introduced minus
# 2 percentage points (measured: lines 90.30%, branches 71.29%, GCC 15.2 in
# the Freedesktop SDK 25.08). Raise them when coverage improves; lowering
# them needs a reason in the commit message.
set -eu
LINE_FLOOR=88.30
BRANCH_FLOOR=69.29

root=$(cd "$(dirname "$0")/.." && pwd)
build=${1:-$root/build/coverage}
jobs=$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)

cmake -S "$root" -B "$build" -DSR_COVERAGE=ON
cmake --build "$build" -j "$jobs"
# Counters accumulate across runs; start from zero.
find "$build" -name '*.gcda' -delete
tests=0
(cd "$build" && ctest -j "$jobs" --output-on-failure) || tests=1
cov=0
python3 "$root/tools/coverage.py" "$build" \
    --fail-under-lines "$LINE_FLOOR" --fail-under-branches "$BRANCH_FLOOR" || cov=1
if [ "$tests" -ne 0 ]; then
    echo "coverage-gate: tests failed (coverage above is still reported)" >&2
fi
exit $((tests | cov))
