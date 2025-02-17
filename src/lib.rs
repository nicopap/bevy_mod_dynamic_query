pub use builder::{AndFilter, AndFilters, DQuery, DynamicQueryBuilder, Fetch, OrFilters};
pub use dynamic_query::{DynamicItem, DynamicQuery};
pub use state::{DynamicState, Ticks};

/// Panic in debug mode, assume `true` in release mode.
macro_rules! assert_invariant {
    ($invariant:expr) => {{
        debug_assert!($invariant);
        if !$invariant {
            std::hint::unreachable_unchecked();
        }
    }};
}

mod archematch;
pub mod builder;
mod debug_unchecked;
mod dynamic_query;
mod fetches;
mod filters;
mod iter;
mod maybe_item;
pub mod pretty_print;
mod state;

#[cfg(test)]
mod tests;
