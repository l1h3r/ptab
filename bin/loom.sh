#!/usr/bin/env bash

# Runs Loom tests with defaults for Loom's configuration values.
#
# The tests are compiled in release mode to improve performance, but debug
# assertions are enabled.
#
# Any arguments to this script are passed to the `cargo test` invocation.
#
# Usage:
#   ./bin/loom.sh                # Run all loom tests
#   ./bin/loom.sh insert         # Run a specific test
#   ./bin/loom.sh -- --nocapture # See output

set -euo pipefail

RUSTFLAGS="${RUSTFLAGS:-} --cfg loom -C debug-assertions=on" \
    LOOM_MAX_PREEMPTIONS="${LOOM_MAX_PREEMPTIONS:-2}" \
    LOOM_LOG="${LOOM_LOG:-1}" \
    LOOM_LOCATION="${LOOM_LOCATION:-1}" \
    LOOM_CHECKPOINT_INTERVAL="${LOOM_CHECKPOINT_INTERVAL:-1}" \
    cargo test --features loom --release --tests "$@"
