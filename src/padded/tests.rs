use crate::padded::CachePadded;

#[test]
fn cache_padded_deref_reads_value() {
  let value: CachePadded<u8> = CachePadded::new(123);

  assert_eq!(*value, 123);
}

#[test]
fn cache_padded_deref_mut_writes_value() {
  let mut value: CachePadded<u8> = CachePadded::new(123);

  *value = 45;

  assert_eq!(*value, 45);
}
