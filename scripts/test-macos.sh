#!/usr/bin/env bash
set -euo pipefail

export RUN_SLOW_TESTS=1

cargo test --workspace "$@"
