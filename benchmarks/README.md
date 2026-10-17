# ptab benchmarks

This crate is not published. It holds two kinds of benchmark:

- `ab`: the A/B driver for judging a change. It builds a baseline and a candidate `ptab` and
  runs them in alternating processes.
- `compare` and `workload`: divan benches comparing `ptab` with other concurrent tables.

## A/B

Run from this directory, so `.cargo/config.toml` (`target-cpu=native`) applies.

```sh
cargo run --bin ab -- --base crates            # crates.io release vs the working tree
cargo run --bin ab -- --base git:<rev>         # any revision vs the working tree
cargo run --bin ab -- --base git:$(git merge-base HEAD main)   # the merge base
cargo run --bin ab -- --self-test --quick      # the same revision on both sides
```

- `--quick` runs 3 interleaved rounds with one multi-thread count (about 3 minutes).
- `--full` is the default: 10 rounds, every row (about 15 minutes).
- `--rounds N` overrides the round count.
- `--cand SPEC` picks the candidate (default `tree`).

SPEC is `crates`, `tree` or `git:REV`. A `git:REV` side is a detached worktree under
`target/scratch/`, removed when the run ends. Each side builds as its own workspace under
`target/scratch/ws-*`, so the tree's `Cargo.lock` is never touched.

Each row prints the median ns/op of each side with a 95% interval, and the ratio of candidate to
baseline with its interval. Rows with 1 thread are gated at 2%, multi-thread rows at 5%. Rows with
more than 8 threads (the P-core count here) are reported as noisy and not gated. A change counts
only when its ratio interval clears the band. Raw samples go to `target/scratch/ab-<time>.tsv`.

`--self-test` exits 1 when any gated row's ratio interval leaves its band.

### Quiet machine

Plug in power, close other work, and run the self-test before trusting a comparison.

## Benches

```sh
cargo bench -p ptab-benchmarks --bench compare -- ptab
```

Multi-thread rows of `compare` run at least 1 ms per sample. Threads run at `USER_INTERACTIVE`
QoS on macOS, so they prefer performance cores.
