# Implementation Notes

This document describes the design and implementation of `ptab`, a lock-free concurrent table optimized for read-heavy workloads.

## Background

The design is inspired by the Erlang/OTP process table implementation, documented in the BEAM source code. The problem it solves is mapping process identifiers to process structures in a highly concurrent environment where:

- Lookups vastly outnumber insertions and deletions
- Multiple threads perform lookups simultaneously
- Lookups must not modify any shared state (no reference counting)
- The data structure must scale with CPU count

Traditional approaches using locks or lock-free structures with reference counting suffer from cache-line contention. Even a simple atomic increment of a reference counter requires the cache line to bounce between all CPUs performing lookups, creating a severe scalability bottleneck.

## Design Goals

1. **Zero-contention reads**: Lookups must not write to any shared memory. This is the single most important property for scalability.

2. **Cache-line aware layout**: Adjacent slots reside in different cache lines to avoid false sharing when multiple threads insert concurrently.

3. **Generational indices**: Reused slots produce different indices to mitigate ABA problems in concurrent algorithms.

4. **Globally sequential allocation**: New indices are assigned in a consistent order across all threads, preserving the property that comparing two indices reveals their relative creation order.

## Architecture Overview

```text
┌─────────────────────────────────────────────────────────────────┐
│                            PTab<T, P>                           │
├─────────────────────────────────────────────────────────────────┤
│  ┌───────────────────────┐    ┌───────────────────────────────┐ │
│  │   Volatile (mutable)  │    │     ReadOnly (immutable)      │ │
│  │   [cache-padded]      │    │     [cache-padded]            │ │
│  ├───────────────────────┤    ├───────────────────────────────┤ │
│  │ entries: AtomicU32    │    │ data: Array<AtomicOwned<T>>   │ │
│  │ next_id: AtomicU32    │    │ slot: Array<AtomicUsize>      │ │
│  │ free_id: AtomicU32    │    │                               │ │
│  └───────────────────────┘    └───────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
```

The table splits into two sections:

- **Volatile**: Counters modified during operations. Grouped together and cache-padded to isolate from read-only data.

- **ReadOnly**: The storage arrays. Despite the name, individual slots are modified atomically, but the arrays themselves are allocated once and never resized.

## Index Structure

Indices come in three forms encoding the same information differently:

### `Detached` Index

The public-facing index type. Encodes both a slot identifier and a generational component in a single `usize`. The bit layout interleaves these components to enable efficient conversion to concrete indices.

### `Abstract` Index

A sequential counter representing allocation order. Abstract index 0 is the first allocation, 1 the second, and so on. When a slot is freed and reallocated, it receives a new abstract index (old index plus capacity), incrementing its generation.

### `Concrete` Index

The actual offset into storage arrays. Multiple abstract indices map to the same concrete index (differing only in generation). The mapping from abstract to concrete distributes consecutive allocations across cache lines.

### Index Conversions

```text
Abstract Index <------> Detached Index
      |                       |
      |                       |
      v                       v
Concrete Index <--------------+
```

Conversions use bit manipulation with precomputed masks and shifts derived from `Params::LENGTH` at compile time.

## Cache-Line Distribution

A naive table places slots 0, 1, 2, ... in consecutive memory locations. When multiple threads allocate simultaneously, they write to adjacent slots, likely in the same cache line, causing contention.

Instead, `ptab` distributes consecutive allocations across cache lines:

```text
Cache Line 0:  slot 0,  slot 8,  slot 16, slot 24, ...
Cache Line 1:  slot 1,  slot 9,  slot 17, slot 25, ...
Cache Line 2:  slot 2,  slot 10, slot 18, slot 26, ...
   ...
Cache Line 7:  slot 7,  slot 15, slot 23, slot 31, ...
```

The abstract-to-concrete mapping implements this transformation:

```rust,ignore
concrete = (abstract & MASK_BLOCK) << SHIFT_BLOCK
         + (abstract >> SHIFT_INDEX) & MASK_INDEX
```

Where `MASK_BLOCK` selects which cache line and `MASK_INDEX` selects position within that cache line. Constants derive from `CACHE_LINE_SLOTS`.

This ensures only true conflicts trigger communication between processors, avoiding false sharing.

## References

- [Erlang/OTP](https://github.com/erlang/otp)
