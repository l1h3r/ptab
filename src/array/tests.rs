use crate::array::Array;
use crate::index::Concrete;
use crate::params::CACHE_LINE;
use crate::params::Params as _;
use crate::utils::each_capacity;

#[cfg_attr(loom, ignore = "needs `loom::model`")]
#[test]
fn alignment() {
  each_capacity!({
    let array: Array<usize, P> = Array::new(|_, slot| {
      slot.write(0);
    });

    // TODO: ptr::is_aligned_to once stable
    assert_eq!(array.as_ptr().addr() & (CACHE_LINE - 1), 0);
  });
}

#[cfg_attr(loom, ignore = "needs `loom::model`")]
#[test]
fn get() {
  each_capacity!({
    let array: Array<usize, P> = Array::new(|index, slot| {
      slot.write(index);
    });

    for index in 0..P::LENGTH.as_usize() {
      assert_eq!(array.get(Concrete::<P>::new(index)), &index);
    }
  });
}

#[cfg_attr(loom, ignore = "needs `loom::model`")]
#[test]
fn as_slice() {
  each_capacity!({
    let array: Array<usize, P> = Array::new(|index, slot| {
      slot.write(index);
    });

    assert_eq!(array.as_slice().len(), P::LENGTH.as_usize());

    for (index, value) in array.as_slice().iter().enumerate() {
      assert_eq!(*value, index);
    }
  });
}

#[cfg_attr(loom, ignore = "needs `loom::model`")]
#[test]
fn as_mut_slice() {
  each_capacity!({
    let mut array: Array<usize, P> = Array::new(|index, slot| {
      slot.write(index);
    });

    assert_eq!(array.as_mut_slice().len(), P::LENGTH.as_usize());

    for (index, value) in array.as_mut_slice().iter_mut().enumerate() {
      assert_eq!(*value, index);
      *value += 1;
    }

    for (index, value) in array.as_slice().iter().enumerate() {
      assert_eq!(*value, index + 1);
    }
  });
}
