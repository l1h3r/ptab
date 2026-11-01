use crate::padded::CachePadded;

#[test]
fn works() {
  let mut value: CachePadded<u8> = CachePadded::new(123);

  assert_eq!(&*value, &123);
  assert_eq!(&mut *value, &123);
}
