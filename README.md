# bevy_mod_dynamic_query

Fork of <https://github.com/jakobhellermann/bevy_ecs_dynamic>, a prototype
for dynamic queries in bevy.

`bevy_ecs_dynamic` was severly out of date and was missing a few query parameters:

- `Or<(…)>`
- `Option<Component>`
- Some other kind of queries that are a combinations of the previous

In logic, we can always express a logical expression as a [disjunction of
conjunctions][dnf]. So we can use a `Vec<Vec<Filter>>` to express `Or`s.

[dnf]: https://en.wikipedia.org/wiki/Disjunctive_normal_form

We can always provide an API that accepts a arbitrary logic expression and
flatten it if necessary.

## Usage

Check the example at `examples/query_interpreter`. The README gives instructions
on how to run the example and play with it.

### External API draft

Suppose we have a script that wants to query anything.

```javascript
function damage_system(q) {
  for (item in q) {
    let [health, damage, opt_armor] = item;
    var real_damage = damage;
    if (opt_armor != null) {
      if (opt_armor > damage) {
        real_damage -= opt_armor;
      } else {
        real_damage = 1;
      }
    }
    health.set(health.get() - real_damage);
  }
}
// equivalent to:
// Query<
//     (&mut Health, &Damage, Option<&Armor>),
//     Or<(
//         (With<Player>, Without<Invicible>),
//         (With<Enemy>, Without<FirstBossYouAreMeantToDieFrom>),
//     )>
// >,
var my_query = {
  fetch: [
    mut("Health"),
    read("Damage"),
    read_nullable("Armor"),
  ],
  filter: or(
    and("Player", not("Invicible")),
    and("Enemy", not("FirstBossYouAreMeantToDieFrom")),
  ),
};
runtime.register_system(my_query, damage_system);
```

### Rust API

The `builder` module contains various ways to build a dynamic query.

#### `Query` constructor API

One of them is just a trait over bevy `Query`s. You can't create a dynamic
pendant of a pre-existing bevy query using that builder pattern:

```rust
fn make_query(world: &mut World) -> DynamicQuery {
    DynamicQuery::from_query::<
        Query<
            (&SetRegTag, Option<&mut TableRegFancy>),
            Or<(
                Changed<Transform>,
                (Without<Transform>, Added<SetRegSimple>),
            )>,
        >,
    >(world)
}
```

#### Method-based API

The other reflects the builder syntax defined by james-j-obrian in their own
dynamic query implementation:

```rust
fn make_query(world: &mut World) -> DynamicQuery {
  DynamicQueryBuilder::new(world)
    .component::<SetRegTag>()
    .optional_mut::<TableRegFancy>()
    .or(|b| b.changed::<Transform>())
    .or(|b| b.without::<Transform>().added::<SetRegSimple>())
    .build()
}
```

#### Name-based API

Since everything is fully dynamic, it's also possible to use plain-text
component names. The API reflects that of the james-j-obrian one, but with
strings instead of types.

Note that unlike the other APIs, it is necessary for the components to be first
initialized in the world.

If the components are already used in different queries, this is a non-issue,
but if the components are typically only used in dymaic context, `init_component`
is needed.

```rust
fn make_query(world: &mut World) -> DynamicQuery {
  world.init_component::<SetRegTag>();
  world.init_component::<TableRegFancy>();
  world.init_component::<SetRegSimple>();

  NamedDynamicBuilder::new(world)
    .component("SetRegTag")
    .optional_mut("TableRegFancy")
    .or(|b| b.changed("Transform"))
    .or(|b| b.without("Transform").added("SetRegSimple"))
    .build()
}
```


## Future works

### Avoid `DynamicItem` heap allocation

Currently this requires keeping an interal "scratch buffer" allocated on the
heap to accumulate items and serve them.
In the general case, it's impossible to get rid of this, since you need to
know at compile time the size of stack-allocated variables.
- This induces one major limitation: Can't concurrently have several items
  live without heap allocation. You should be able to keep around a previous
  set of item by using `Clone`, but this requires heap allocation
- `SmallVec` seems like an easy win, since not many components are fetched usually
- `ArrayVec` also works and avoids bimodal performance, But it requires a compile-time
  max query fetch size.
- For a compile-time use-case such as relation, it should be possible to
  use `typenum` to get fixed-size array rather than a stack-allocate slice

### Flexibility wrapper

`DynamicQuery` is more limited than bevy's `Query`. It notably can't handle
duplicate fetch/filter items and nested `Or`.

We can do as much transformation on the filter part before creating the
`DynamicQuery` as we want, since it will always be opaque to the end-user.

This means we can:

- Resolve the `Filter` logical statement through a SAT solver. We can even
  eliminate redundant clauses, fuse the `Fetches` in as well.
- Eliminate redundant components (including with `Fetches`). Some operations
  are time-proportional to the number of components, and validity depends on
  abscence of redundant elements, so we should get rid of them automatically.

When it comes to the `Fetches` part of the query. It becomes a bit more tricky.
We need to:

- Verify the items of the query respect exclusive mutability rules
- Remap the `DynamicItem` slice when accessing it
- Re-nest the items potentially if we want to let users have `[[foo, bar], [baz]]`
  This may be useful to support query aliases or dictionaries.

We can also introduce additional combinators like `AnyOf` in the wrapper.

### Late component ID binding

Currently we extract from `TypeRegistry` the `ReflectFromPtr`s as soon as we
construct `DynamicQuery`. This means we need to update `ReflectFromPtr` when
it changes, and we can't refer to components that will be added after the
`DynamicQuery` is created.