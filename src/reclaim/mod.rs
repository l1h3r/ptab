pub(crate) mod leak;
pub(crate) mod sdd;

// -----------------------------------------------------------------------------
// Sanity Check
// -----------------------------------------------------------------------------

const _: () = assert!(align_of::<leak::Atomic<()>>() == align_of::<usize>());
const _: () = assert!(size_of::<leak::Atomic<()>>() == size_of::<usize>());

const _: () = assert!(align_of::<sdd::Atomic<()>>() == align_of::<usize>());
const _: () = assert!(size_of::<sdd::Atomic<()>>() == size_of::<usize>());
