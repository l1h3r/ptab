#!/usr/bin/env bash

# Runs Shuttle tests with defaults for Shuttle's configuration values.
#
# The tests are compiled in release mode to improve performance, but debug
# assertions are enabled.
#
# Any arguments to this script are passed to the `cargo test` invocation.
#
# Usage:
#   ./bin/shuttle.sh                # Run all shuttle tests
#   ./bin/shuttle.sh insert         # Run a specific test
#   ./bin/shuttle.sh -- --nocapture # See output

set -euo pipefail

RUSTFLAGS="${RUSTFLAGS:-} --cfg shuttle -C debug-assertions=on" \
    cargo test --features shuttle --release --tests "$@"
