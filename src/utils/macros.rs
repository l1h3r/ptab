macro_rules! each_capacity {
  ($expr:expr) => {
    #[cfg(not(feature = "slow"))]
    {
      $crate::utils::each_capacity!(
        @impl $expr,
        4, 10, 16,
      );
    }

    #[cfg(feature = "slow")]
    {
      $crate::utils::each_capacity!(
        @impl $expr,
        4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
      );
    }
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
