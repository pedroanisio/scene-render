#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
set -eu
exec python3 "$(dirname "$0")/equivalence-oracle.py" "$@"
