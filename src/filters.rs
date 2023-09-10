use std::collections::HashSet;

use bevy_ecs::archetype::Archetype;
use bevy_ecs::component::{ComponentId, Tick};
use bevy_ecs::world::unsafe_world_cell::UnsafeEntityCell;
use fixedbitset::FixedBitSet;

use crate::ctor_dsl::{AndFilter, AndFilters, OrFilters};
use crate::debug_unchecked::DebugUnchecked;
use crate::fetches::Fetches;
use crate::jagged_array::{JaggedArray, JaggedArrayBuilder};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filters(JaggedArray<Filter>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
enum FilterKind {
    With = 0,
    Changed = 1,
    Added = 2,
    Without = 3,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Filter {
    /// This is an optimized `enum` where the variant discriminant is stored in
    /// the most significant two bits of `component`.
    component: u32,
}

impl FilterKind {
    const fn from_u32(u32: u32) -> Self {
        match u32 {
            0 => FilterKind::With,
            1 => FilterKind::Changed,
            2 => FilterKind::Added,
            3 => FilterKind::Without,
            _ => unreachable!(),
        }
    }
}
impl Filter {
    const MASK: u32 = 0x7f_ff_ff_ff;
    const KIND_OFFSET: u32 = 30;

    pub const fn id(&self) -> ComponentId {
        let masked = self.component & Self::MASK;
        ComponentId::new(masked as usize)
    }
    #[allow(unused)]
    const fn kind(&self) -> FilterKind {
        let unmasked = self.component >> Self::KIND_OFFSET;
        FilterKind::from_u32(unmasked)
    }
    fn new(kind: FilterKind, id: ComponentId) -> Self {
        let kind_mask = (kind as u32) << Self::KIND_OFFSET;
        let id_mask = id.index() as u32;
        let component = kind_mask | id_mask;
        Filter { component }
    }
}
impl From<AndFilter> for Filter {
    fn from(value: AndFilter) -> Self {
        match value {
            AndFilter::With(id) => Self::new(FilterKind::With, id),
            AndFilter::Without(id) => Self::new(FilterKind::Without, id),
            AndFilter::Changed(id) => Self::new(FilterKind::Changed, id),
            AndFilter::Added(id) => Self::new(FilterKind::Added, id),
        }
    }
}

/// [`Filters`] are a list of "conjunction".
pub struct Conjunction<'a> {
    filters: &'a [Filter],
    fetches: &'a Fetches,
}

struct InclusiveFilter<'a>(&'a [Filter]);
struct ExclusiveFilter<'a>(&'a [Filter]);
struct ChangedFilter<'a>(&'a [Filter]);
struct AddedFilter<'a>(&'a [Filter]);

impl Filters {
    /// # Safety
    /// - Filters within conjunctions must be sorted
    /// - There is no duplicate inclusive filters within each single
    ///   conjunction.
    pub unsafe fn new_unchecked(OrFilters(dsl_value): OrFilters) -> Self {
        let cell_count = dsl_value.iter().map(|x| x.0.len()).sum();
        let mut builder = JaggedArrayBuilder::new_with_capacity(dsl_value.len(), cell_count);
        for AndFilters(filters) in dsl_value.into_iter() {
            builder.add_row(filters.into_iter().map(Filter::from));
        }
        Filters(builder.build())
    }
    pub fn new(OrFilters(dsl_value): OrFilters) -> Option<Self> {
        let cell_count = dsl_value.iter().map(|x| x.0.len()).sum();
        let mut builder = JaggedArrayBuilder::new_with_capacity(dsl_value.len(), cell_count);
        for AndFilters(filters) in dsl_value.into_iter() {
            let mut filters: Vec<_> = filters.into_iter().map(Filter::from).collect();
            filters.sort_unstable();
            if duplicates_in(&filters) {
                return None;
            }
            builder.add_row(filters);
        }
        Some(Filters(builder.build()))
    }
    pub fn conjunctions<'a>(
        &'a self,
        fetches: &'a Fetches,
    ) -> impl Iterator<Item = Conjunction<'a>> + 'a {
        let conjunction = |filters| Conjunction { filters, fetches };
        self.0.rows_iter().map(conjunction)
    }
    pub fn tick_conjunctions(
        &self,
        last_run: Tick,
        this_run: Tick,
    ) -> impl Iterator<Item = TickConjunction> + '_ {
        let conjunction = move |filters| TickConjunction { filters, last_run, this_run };
        self.0.rows_iter().map(conjunction)
    }
}
impl TryFrom<OrFilters> for Filters {
    type Error = ();
    fn try_from(value: OrFilters) -> Result<Self, Self::Error> {
        Self::new(value).ok_or(())
    }
}
fn duplicates_in(filters: &[Filter]) -> bool {
    let mut encountered = HashSet::with_capacity(filters.len());
    filters.iter().any(|f| !encountered.insert(f.id()))
}
fn tick_filters(filters: &[Filter]) -> (ChangedFilter, AddedFilter) {
    // A Filter value that always fit at the very end of the inclusive filters range.
    let mut last_with = Filter::new(FilterKind::Changed, ComponentId::new(0));
    let mut last_changed = Filter::new(FilterKind::Added, ComponentId::new(0));
    let mut last_added = Filter::new(FilterKind::Without, ComponentId::new(0));
    last_with.component -= 1;
    last_changed.component -= 1;
    last_added.component -= 1;

    let first_changed = filters.binary_search(&last_with).err();
    let first_added = filters.binary_search(&last_changed).err();
    let first_without = filters.binary_search(&last_added).err();

    let first_changed = unsafe { first_changed.prod_unchecked_unwrap() };
    let first_added = unsafe { first_added.prod_unchecked_unwrap() };
    let first_without = unsafe { first_without.prod_unchecked_unwrap() };

    let changed_filter = ChangedFilter(&filters[first_changed..first_added]);
    let added_filter = AddedFilter(&filters[first_added..first_without]);
    (changed_filter, added_filter)
}
fn filters(filters: &[Filter]) -> (InclusiveFilter, ExclusiveFilter) {
    // A Filter value that always fit at the very end of the inclusive filters range.
    let mut last_inclusive = Filter::new(FilterKind::Without, ComponentId::new(0));
    last_inclusive.component -= 1;

    // SAFETY: If we find a componet with an ID equal to 2**30, something fishy is going on
    let first_exclusive = filters.binary_search(&last_inclusive).err();
    let first_exclusive = unsafe { first_exclusive.prod_unchecked_unwrap() };
    let (inclusive, exclusive) = filters.split_at(first_exclusive);
    (InclusiveFilter(inclusive), ExclusiveFilter(exclusive))
}
impl<'a> Conjunction<'a> {
    // O(n²) where n is sizeof archetype
    pub fn includes_archetype(&self, archetype: &Archetype) -> bool {
        // NOTE(perf): We don't skip this on `fetch_archetype == false` because
        // we hope the optimizer can merge `all_included` `for` with this one.
        let (inclusive, exclusive) = filters(self.filters);
        let include_filter = inclusive.all_included(archetype.components());
        let exclude_filter = exclusive.any_excluded(archetype.components());
        let fetch_archetype = self.fetches.all_included(archetype.components());

        fetch_archetype && include_filter && !exclude_filter
    }
}
/// [`Filters`] are a list of "conjunction"
pub struct TickConjunction<'a> {
    filters: &'a [Filter],
    last_run: Tick,
    this_run: Tick,
}
impl<'a> TickConjunction<'a> {
    // NOTE: unlike `fetches::FetchesIter::next`, we can't assume we are on the
    // right table, because we may call this with a table from a different conjunction.
    // TODO(perf): This needs to be cached.
    // O(n²) where n is sizeof archetype
    pub fn within_tick(&self, entity: UnsafeEntityCell) -> bool {
        let archetype = entity.archetype();
        let (inclusive, exclusive) = filters(self.filters);
        let include_filter = inclusive.all_included(archetype.components());
        let exclude_filter = exclusive.any_excluded(archetype.components());

        if !include_filter || exclude_filter {
            return false;
        }
        let (changed, added) = tick_filters(self.filters);
        let changed = changed.within_tick(entity, self.last_run, self.this_run);
        let added = added.within_tick(entity, self.last_run, self.this_run);

        changed && added
    }
}
// TODO(perf): Likely can avoid O(n²). If only `ComponedId`s were
// ordered in `Archetype::components()`…
impl<'a> InclusiveFilter<'a> {
    #[inline]
    pub fn all_included(self, ids: impl Iterator<Item = ComponentId>) -> bool {
        let mut found = FixedBitSet::with_capacity(self.0.len());
        for id in ids {
            if let Some(idx) = self.0.iter().position(|x| x.id() == id) {
                found.set(idx, true);
            }
        }
        found.count_ones(..) == self.0.len()
    }
}
impl<'a> ExclusiveFilter<'a> {
    #[inline]
    pub fn any_excluded(&self, ids: impl Iterator<Item = ComponentId>) -> bool {
        let mut found = false;
        for id in ids {
            found |= self.0.binary_search_by_key(&id, Filter::id).is_ok();
        }
        found
    }
}
impl<'a> ChangedFilter<'a> {
    fn within_tick(&self, entity: UnsafeEntityCell, last_run: Tick, this_run: Tick) -> bool {
        self.0
            .iter()
            .map(|f| unsafe { entity.get_change_ticks_by_id(f.id()) })
            .map(|t| unsafe { t.prod_unchecked_unwrap() })
            .all(|t| t.last_changed_tick().is_newer_than(last_run, this_run))
    }
}
impl<'a> AddedFilter<'a> {
    fn within_tick(&self, entity: UnsafeEntityCell, last_run: Tick, this_run: Tick) -> bool {
        self.0
            .iter()
            .map(|f| unsafe { entity.get_change_ticks_by_id(f.id()) })
            .map(|t| unsafe { t.prod_unchecked_unwrap() })
            .all(|t| t.added_tick().is_newer_than(last_run, this_run))
    }
}
