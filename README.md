# ptab

[![github][shield-img-github]][shield-url-github]
[![crates.io][shield-img-crates]][shield-url-crates]
[![docs.rs][shield-img-docs]][shield-url-docs]
[![build status][shield-img-ci]][shield-url-ci]

A lock-free concurrent table for Rust.

**ptab** provides a fixed-capacity table optimized for read-heavy concurrent workloads. Inspired by Erlang's BEAM process table, lookups scale linearly with CPU count by avoiding all shared memory writes.

## Features

- **Zero-contention reads** - Atomic loads for lock-free lookups.
- **Cache-line distribution** - Prevents false sharing with no memory overhead.
- **Generational indices** - Slot reuse mitigates ABA problems.

## Usage

Add the following to your `Cargo.toml`:

```toml
[dependencies]
ptab = "0.1"
```

## Example

```rust
use ptab::PTab;
use std::sync::Arc;

let table = Arc::new(PTab::<&str>::new());

let handle = std::thread::spawn({
  let table = Arc::clone(&table);
  move || table.insert("world").unwrap()
});

let index_a = table.insert("hello").unwrap();
let index_b = handle.join().unwrap();

assert_eq!(table.read(index_a).unwrap(), "hello");
assert_eq!(table.read(index_b).unwrap(), "world");
```

## Design

See [`IMPLEMENTATION.md`] for detailed design notes.

<br>

#### License

<sup>
  Licensed under either of <a href="LICENSE-APACHE">Apache License, Version 2.0</a> or <a href="LICENSE-MIT">MIT license</a> at your option.
</sup>

<br>

<sub>
  Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this crate by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
</sub>

[//]: # (links)

[`IMPLEMENTATION.md`]: IMPLEMENTATION.md

[//]: # (badges)

[shield-url-github]: https://github.com/l1h3r/ptab
[shield-img-github]: https://img.shields.io/badge/github-l1h3r/ptab-main?style=flat-square&logo=github

[shield-url-crates]: https://crates.io/crates/ptab
[shield-img-crates]: https://img.shields.io/crates/v/ptab?style=flat-square&logo=rust

[shield-url-docs]: https://docs.rs/ptab
[shield-img-docs]: https://img.shields.io/docsrs/ptab?style=flat-square&logo=docs.rs

[shield-url-ci]: https://github.com/l1h3r/ptab/actions/workflows/ci.yml?query=branch:main
[shield-img-ci]: https://img.shields.io/github/actions/workflow/status/l1h3r/ptab/ci.yml?style=flat-square
