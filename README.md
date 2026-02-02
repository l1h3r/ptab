# ptab

[![github][shield-img-github]][shield-url-github]
[![crates.io][shield-img-crates]][shield-url-crates]
[![docs.rs][shield-img-docs]][shield-url-docs]
[![build status][shield-img-ci]][shield-url-ci]

Lock-free concurrent table optimized for read-heavy workloads.

Inspired by the Erlang/OTP BEAM process table, `ptab` provides a fixed-capacity table where lookup operations perform no shared memory writes - not even reference counts. This enables linear read scalability with CPU count.

## Usage

Add the following to your `Cargo.toml`:

```toml
[dependencies]
ptab = "0.1"
```

## Example

```rust
let table: ptab::PTab<&str> = ptab::PTab::new();
let index: ptab::Detached = table.insert("hello").unwrap();

assert_eq!(table.read(index), Some("hello"));
assert!(table.remove(index));
assert_eq!(table.read(index), None);
```

## Performance

Under contention (16 threads reading a single hot key):

|            | ptab    | sharded-slab |
|------------|--------:|-------------:|
| Throughput | 212 M/s |      155 K/s |

Use `ptab` when reads dominate. Use [`sharded-slab`] for write-heavy workloads or dynamic capacity.

## Design

See [`IMPLEMENTATION.md`] for details.

- **Zero-contention reads**: Lookups use only thread-local state and atomic loads
- **Cache-line aware**: Consecutive allocations distributed across cache lines
- **Generational indices**: Slot reuse produces different indices (ABA prevention)
- **Epoch-based reclamation**: Safe memory management via [`sdd`]

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
[`sharded-slab`]: https://crates.io/crates/sharded-slab
[`sdd`]: https://crates.io/crates/sdd

[//]: # (badges)

[shield-url-github]: https://github.com/l1h3r/ptab
[shield-img-github]: https://img.shields.io/badge/github-l1h3r/ptab-main?style=flat-square&logo=github

[shield-url-crates]: https://crates.io/crates/ptab
[shield-img-crates]: https://img.shields.io/crates/v/ptab?style=flat-square&logo=rust

[shield-url-docs]: https://docs.rs/ptab
[shield-img-docs]: https://img.shields.io/docsrs/ptab?style=flat-square&logo=docs.rs

[shield-url-ci]: https://github.com/l1h3r/ptab/actions/workflows/ci.yml?query=branch:main
[shield-img-ci]: https://img.shields.io/github/actions/workflow/status/l1h3r/ptab/ci.yml?style=flat-square
