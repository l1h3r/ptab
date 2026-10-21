# ptab

[![github][shield-img-github]][shield-url-github]
[![crates.io][shield-img-crates]][shield-url-crates]
[![docs.rs][shield-img-docs]][shield-url-docs]
[![build status][shield-img-ci]][shield-url-ci]

A lock-free, fixed-capacity concurrent table for read-heavy workloads.

Ported from the Erlang/OTP process table. Lookups use atomic loads and write nothing to the table. Consecutive inserts land in different cache lines. Capacity and the reclamation strategy are set at compile time.

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

## Features

- `sdd` (default): the `Sdd` collector. Turns on `std`.
- `std`: builds against the standard library.
- `alloc`: builds without `std`. Without `sdd`, the default collector is `Leak`.

To build without default features:

```sh
cargo build --no-default-features --features alloc
```

The alloc-only build inserts and reads. Its default collector is [`Leak`], which has no `remove` and frees entries when the table drops.

## Collectors

| | [`Sdd`] | [`Leak`] |
|-|-|-|
| Feature | `sdd` (default) | Without `sdd` |
| Needs `std` | Yes | No |
| Frees entries | Eventually, via epochs | On table drop |
| Supports `remove` | Yes | No |
| Guard type | `sdd::Guard` | `()` |
| Read cost | Epoch pin | Plain atomic load |

## Design

See [`IMPLEMENTATION.md`] for design notes.

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
[`Leak`]: https://docs.rs/ptab/latest/ptab/memory/collector/enum.Leak.html
[`Sdd`]: https://docs.rs/ptab/latest/ptab/memory/collector/enum.Sdd.html

[//]: # (badges)

[shield-url-github]: https://github.com/l1h3r/ptab
[shield-img-github]: https://img.shields.io/badge/github-l1h3r/ptab-main?style=flat-square&logo=github

[shield-url-crates]: https://crates.io/crates/ptab
[shield-img-crates]: https://img.shields.io/crates/v/ptab?style=flat-square&logo=rust

[shield-url-docs]: https://docs.rs/ptab
[shield-img-docs]: https://img.shields.io/docsrs/ptab?style=flat-square&logo=docs.rs

[shield-url-ci]: https://github.com/l1h3r/ptab/actions/workflows/ci.yml?query=branch:main
[shield-img-ci]: https://img.shields.io/github/actions/workflow/status/l1h3r/ptab/ci.yml?style=flat-square
