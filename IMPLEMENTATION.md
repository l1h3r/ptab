# Implementation Notes

Design notes for `ptab`, a lock-free concurrent table for read-heavy workloads.

## Background

The design follows the Erlang/OTP process table. That table maps process identifiers to process structures under these conditions:

- Lookups far outnumber insertions and removals.
- Many threads look up entries at once.
- A lookup must not modify shared state, including reference counts.

A reference count makes every reader write to the same cache line. The line bounces between CPUs, and lookups stop scaling with the thread count. Readers here only load.

## Goals

1. Lookups write no table memory.
2. Consecutive insertions touch different cache lines, so concurrent inserters do not share a line.
3. Capacity and layout are fixed at compile time. The arrays are allocated once and never resized.

## Layout

```text
┌─────────────────────────────────────────────────────────────────┐
│                            PTab<T, P>                           │
├─────────────────────────────────────────────────────────────────┤
│  ┌───────────────────────┐    ┌───────────────────────────────┐ │
│  │   Volatile            │    │   ReadOnly                    │ │
│  │   [cache-padded]      │    │   [cache-padded]              │ │
│  ├───────────────────────┤    ├───────────────────────────────┤ │
│  │ entries: AtomicU32    │    │ data: Array<Atomic<T>>        │ │
│  │ next_id: AtomicU32    │    │ slot: Array<AtomicUsize>      │ │
│  │ free_id: AtomicU32    │    │                               │ │
│  └───────────────────────┘    └───────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
```

`Volatile` holds the counters that every insertion and removal updates. Padding keeps them off the lines that readers load.

`ReadOnly` holds two arrays of `Params::LENGTH` elements, each allocated aligned to a cache line:

- `data` holds the entries.
- `slot` is a ring of free `Abstract` indices (see Allocation). The index an insert takes from it names the `data` position the new entry uses.

The name is loose. Elements of both arrays change atomically, but the arrays themselves never move.

## Index Forms

An entry is named by three forms of one number.

`Detached` is the public form, a `usize`. Its low bits are the `Concrete` index and the remaining high bits are the serial. `Detached::decompose` returns the `(number, serial)` pair.

`Abstract` is the allocation counter. The first allocation is 0, the second is 1, and so on. When an entry is removed, its index plus `Params::LENGTH` goes back into the ring, so the low bits stay the same and the serial grows by one.

`Concrete` is the offset into `data` and `slot`. Every `Abstract` index that differs only in its serial maps to the same `Concrete` index.

```text
Abstract <------> Detached
    |                 |
    v                 |
Concrete <------------+
```

The conversions use masks and shifts that `ParamsExt` derives from `Params::LENGTH`:

- `ID_MASK_ENTRY` is `LENGTH - 1`. It selects the entry bits of an index.
- `ID_MASK_BLOCK` is the block count minus one, and `ID_SHIFT_INDEX` is the number of bits in it.
- `ID_MASK_INDEX` is the slots each block holds minus one, `(LENGTH >> ID_SHIFT_INDEX) - 1`, and `ID_SHIFT_BLOCK` is the number of bits in it. It is the slots per cache line minus one unless the table is smaller than one cache line.

A block is one cache line of slots. Lookup and removal take the `Concrete` index from a `Detached` index by masking with `ID_MASK_ENTRY`. They ignore the serial.

## Cache-Line Distribution

Mapping the allocation counter straight to an array offset would place consecutive insertions in adjacent slots. Threads inserting together would write to one cache line.

The `Abstract` to `Concrete` mapping puts the low bits of the counter in the block position:

```text
concrete = ((abstract & ID_MASK_BLOCK) << ID_SHIFT_BLOCK)
         + ((abstract >> ID_SHIFT_INDEX) & ID_MASK_INDEX)
```

Consecutive allocations visit every block before any block gets a second slot. With a 128-byte line, 8-byte slots and 1024 entries there are 16 slots per line and 64 blocks:

```text
abstract  0  1  2 ... 63  64  65
concrete  0 16 32 ... 1008 1  17
```

Slots per line depend on the target. `CACHE_LINE_SLOTS` holds the value.

## Capacity

`Capacity` is a power of two from `Capacity::MIN` (2^4) to `Capacity::MAX` (2^27). `Capacity::new` rounds up and clamps. The block count is derived at compile time, and a capacity whose layout does not fit in `isize::MAX` bytes fails to compile.

The table is full when `entries` reaches `LENGTH`. `insert` increments `entries` first and returns `None` after undoing the increment when the table is full.

At `Capacity::MAX` one slot is reserved. `entries` starts at 1, so the table holds `LENGTH - 1` entries and `PTab::capacity` reports that.

## Allocation

`next_id` and `free_id` are counters that walk the `slot` array as a ring.

- Insert takes the next value of `next_id`, maps it to a `Concrete` index and swaps the reserved marker (`usize::MAX`) into that slot. The old value is the `Abstract` index of the new entry. If the slot already held the marker, insert tries the next value.
- Remove evicts the entry, then takes the next value of `free_id`. It stores the freed index plus `LENGTH` into that slot, replacing the marker, and decrements `entries`.

## Reclamation

Remove only unlinks the entry from `data`. The memory is handed to the collector chosen by `Params::Collector`, which defers freeing it while a reader may still hold a reference. Readers take a guard from the same collector before they load.

## References

- [Erlang/OTP](https://github.com/erlang/otp)
