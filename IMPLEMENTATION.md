# Implementation Notes

This document describes the design and implementation of `ptab`, a lock-free
concurrent table optimized for read-heavy workloads.

## Background

The design is inspired by the Erlang/OTP process table implementation,
documented in the BEAM source code. The original problem it solves is
mapping process identifiers to process structures in a highly concurrent
environment where:

- Lookups vastly outnumber insertions and deletions
- Multiple threads perform lookups simultaneously
- Lookups should not modify any shared state (no reference counting)
- The data structure should scale with the number of CPUs

Traditional approaches using locks or even lock-free structures with
reference counting suffer from cache-line contention. Even a simple
atomic increment of a reference counter requires the cache line to
bounce between all CPUs performing lookups, creating a severe scalability
bottleneck.

## Design Goals

1. **Zero-contention reads**: Lookups must not write to any shared memory.
   This is the single most important property for scalability.

2. **Cache-line aware layout**: Adjacent slots should reside in different
   cache lines to avoid false sharing when multiple threads insert
   concurrently.

3. **Generational indices**: Reused slots should produce different indices
   to mitigate ABA problems in concurrent algorithms.

4. **Globally sequential allocation**: New indices should be assigned in
   a consistent order across all threads, preserving the property that
   comparing two indices reveals their relative creation order.

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

The table is split into two sections:

- **Volatile**: Counters that change during operation. These are grouped
  together and cache-padded to isolate them from the read-only data.

- **ReadOnly**: The actual storage arrays. Despite the name, individual
  slots are modified atomically, but the arrays themselves are allocated
  once and never resized.

## Index Structure

Indices in `ptab` come in three forms that encode the same information
differently:

### Detached Index

The public-facing index type. It encodes both a slot identifier and a
generational component in a single `usize`. The bit layout interleaves
these components to enable efficient conversion to concrete indices.

### Abstract Index

A sequential counter representing allocation order. Abstract index 0 is
the first allocation, 1 is the second, and so on. When a slot is freed
and reallocated, it receives a new abstract index (the old index plus
the table capacity), incrementing its generation.

### Concrete Index

The actual offset into the storage arrays. Multiple abstract indices map
to the same concrete index (they differ only in generation). The mapping
from abstract to concrete is designed to distribute consecutive
allocations across cache lines.

### Index Conversions

```text
Abstract Index <------> Detached Index
      |                       |
      |                       |
      v                       v
Concrete Index <--------------+
```

The conversions use bit manipulation with precomputed masks and shifts
derived from the table's capacity at compile time.

## Cache-Line Distribution

A naive table would place slots 0, 1, 2, ... in consecutive memory
locations. When multiple threads allocate simultaneously, they would
write to adjacent slots, likely in the same cache line, causing
contention.

Instead, `ptab` distributes consecutive allocations across different
cache lines:

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

Where `MASK_BLOCK` selects which cache line, and `MASK_INDEX` selects
the position within that cache line. The constants are derived from
`CACHE_LINE_SLOTS` (typically 8 on 64-bit systems with 64-byte cache
lines and 8-byte pointers).

This approach ensures that only true conflicts trigger communication
between involved processors, avoiding false sharing.

## Allocation

Inserting a new entry involves four steps:

### 1. Reserve Capacity

```rust,ignore
let prev = entries.fetch_add(1, Relaxed);
if prev >= capacity {
    // Table full, undo and fail
    entries.fetch_sub(1, Relaxed);
    return None;
}
```

This atomic increment reserves space before we find a slot. If the table
is full, we undo the increment and return `None`.

### 2. Find a Free Slot

```rust,ignore
loop {
    let abstract_idx = next_id.fetch_add(1, Relaxed);
    let concrete_idx = abstract_to_concrete(abstract_idx);
    let result = slot[concrete_idx].swap(RESERVED, AcqRel);
    if result != RESERVED {
        return Abstract::new(result);
    }
}
```

We increment `next_id` to get a candidate slot, then attempt to reserve
it by swapping in a marker value. If the slot was already reserved by
another thread, we try the next one.

The value we swap out becomes the new abstract index for this entry.
This value encodes both the concrete slot and its generation. When the
slot is later freed and reallocated, it will receive a higher abstract
index.

### 3. Initialize the Entry

The caller's initialization function runs, writing the value into a
heap-allocated box. This happens after we have the index but before
the entry is visible in the table, allowing the stored value to
contain its own index.

### 4. Publish

```rust,ignore
data[concrete_idx].store(entry, Release);
```

The entry becomes visible to other threads. The `Release` ordering
ensures all initialization writes are visible before the pointer.

## Deallocation

Removing an entry:

### 1. Swap Out the Pointer

```rust,ignore
let value: Option<Owned<T>> = entry.swap((None, Tag::None), AcqRel).0;
if value.is_none() {
    return false; // Already removed
}
```

The atomic swap ensures exactly one thread "wins" if multiple threads
try to remove the same entry concurrently.

### 2. Defer Destruction

When the `Owned<T>` returned from the swap is dropped, the epoch-based
reclamation system (`sdd`) schedules deferred destruction. The actual
deallocation happens later, once all threads that might have been
reading the entry have moved past the current epoch.

### 3. Release the Slot

```rust,ignore
let next_abstract = current_abstract + capacity;
loop {
    let free_idx = free_id.fetch_add(1, Relaxed);
    let concrete_idx = abstract_to_concrete(free_idx);
    if slot[concrete_idx].compare_exchange(
        RESERVED, next_abstract, Relaxed, Relaxed
    ).is_ok() {
        break;
    }
}
```

We compute the next abstract index for this slot (current + capacity,
which increments the generation) and find a reserved slot to write it
to. This makes the slot available for future allocations.

### 4. Update Entry Count

```rust,ignore
entries.fetch_sub(1, Release);
```

## Memory Reclamation

`ptab` uses epoch-based reclamation via `sdd`. This is what
enables zero-contention reads.

### The Problem

When thread A removes an entry, thread B might be in the middle of
reading it. We cannot free the memory until B is done. Traditional
solutions use reference counting, but incrementing a counter on every
read causes the exact cache-line contention we are trying to avoid.

### The Solution

Epoch-based reclamation divides time into epochs. Each thread "pins"
itself to the current epoch before accessing shared data. Memory is
not freed until all threads have unpinned from the epoch in which the
memory was retired.

```rust,ignore
// Reader (in `with`)
let guard = Guard::new();                       // Pin to current epoch
let ptr = data[idx].load(Acquire, &guard);
let value = ptr.as_ref();                       // Safe: guard keeps memory alive
// ... use value ...
// guard dropped: unpin from epoch

// Writer (in `remove`)
let value: Option<Owned<T>> = entry.swap((None, Tag::None), AcqRel).0;
// Dropping `Owned<T>` defers destruction until epoch advances
```

The key insight is that pinning only writes to thread-local storage,
not shared memory. Readers never contend with each other.

## Comparison with sharded-slab

| Aspect | ptab | sharded-slab |
|--------|------|--------------|
| Read contention | None | None |
| Write contention | Global counters | Thread-local (usually none) |
| Index ordering | Globally sequential | Per-shard ranges |
| Memory overhead | Single array | Per-thread shards |
| Capacity | Fixed at compile time | Grows dynamically |

Use `ptab` for read-dominated workloads (BEAM-style process tables).
Use `sharded-slab` for write-heavy or high-churn workloads.

## References

- [Erlang/OTP](https://github.com/erlang/otp)
- [sdd](https://docs.rs/sdd)
- [sharded-slab](https://docs.rs/sharded-slab)
