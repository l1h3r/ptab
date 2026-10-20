macro_rules! each_capacity {
  ($expr:expr) => {
    $crate::utils::each_capacity!(
      @impl $expr,
      $crate::params::Capacity::DEF.log2(),
      $crate::params::Capacity::MAX.log2(),
      $crate::params::Capacity::MIN.log2(),
    );
  };
  (@impl $expr:expr, $($bits:expr),+ $(,)?) => {
    $(
      $crate::utils::each_capacity!(@run $expr, $bits);
    )+
  };
  (@run $expr:expr, $bits:expr) => {{
    const S: usize = 1 << $bits;
    type P = $crate::params::ConstParams::<{ S }>;
    $expr
  }};
}

pub(crate) use each_capacity;
