#[cfg(miri)]
macro_rules! each_capacity {
  ($expr:expr) => {{
    each_capacity!(@run, $expr, 1 << 4);
    each_capacity!(@run, $expr, 1 << 10);
    each_capacity!(@run, $expr, 1 << 16);
  }};
  (@run, $expr:expr, $size:expr) => {{
    type P = $crate::params::ConstParams::<{ $size }>;
    $expr
  }};
}

#[cfg(not(miri))]
macro_rules! each_capacity {
  ($expr:expr) => {{
    each_capacity!(@run, $expr, 1 << 4);
    each_capacity!(@run, $expr, 1 << 5);
    each_capacity!(@run, $expr, 1 << 6);
    each_capacity!(@run, $expr, 1 << 7);
    each_capacity!(@run, $expr, 1 << 8);
    each_capacity!(@run, $expr, 1 << 9);
    each_capacity!(@run, $expr, 1 << 10);
    each_capacity!(@run, $expr, 1 << 11);
    each_capacity!(@run, $expr, 1 << 12);
    each_capacity!(@run, $expr, 1 << 13);
    each_capacity!(@run, $expr, 1 << 14);
    each_capacity!(@run, $expr, 1 << 15);
    each_capacity!(@run, $expr, 1 << 16);
    each_capacity!(@run, $expr, 1 << 17);
    each_capacity!(@run, $expr, 1 << 18);
    each_capacity!(@run, $expr, 1 << 19);
    each_capacity!(@run, $expr, 1 << 20);
    each_capacity!(@run, $expr, 1 << 21);
    each_capacity!(@run, $expr, 1 << 22);
    each_capacity!(@run, $expr, 1 << 23);
    each_capacity!(@run, $expr, 1 << 24);
    each_capacity!(@run, $expr, 1 << 25);
    each_capacity!(@run, $expr, 1 << 26);
    each_capacity!(@run, $expr, 1 << 27);
  }};
  (@run, $expr:expr, $size:expr) => {{
    type P = $crate::params::ConstParams::<{ $size }>;
    $expr
  }};
}
