# AGENTS.md — Coding Rules (Rust)

> Philosophy: control complexity, not just make it compile.
> The compiler (borrow checker, type system) is an ally: if it forces
> a design, listen to it before adding indirection or `.clone()`.

---

## 0. Agent Workflow

Before writing code:

1. If anything in the plan is ambiguous or violates a rule in this
   document, raise it with the user before continuing. Do not resolve
   it on your own.
2. Refactor **before** adding new functionality, never at the same
   time. If external behavior changes, it is not refactoring.
3. Create generic, reusable structures and functions.
4. Eliminate unnecessary complexity.
5. Encapsulate complexity.

Before considering the task done:

- [ ] External behavior has not changed (unless that was the goal).
- [ ] Tests, formatting, lints, and documentation verified.
- [ ] Comments and docs updated if the code changed.
- [ ] No new technical debt introduced without justification.
- [ ] **Structural** refactoring (hierarchies, architectural pattern)
      → raise it with the user, don't decide alone.

---

## 1. Performance and Memory — apply always, no exceptions

This section takes priority over style or code brevity.

### Clones

- Never add `.clone()` to "silence" the borrow checker. Order of
  solutions, in this order of preference:
  1. Reorder operations so the borrow ends earlier.
  2. Shorten the lifetime of the borrow: extract only the scalar
     (`usize`, `bool`, `f64`) you need before mutating, instead of
     holding a reference to the composite result (`&String`, `&Vec<T>`).
  3. `mem::take()` / `mem::replace()` / `mem::swap()` to extract or
     swap a value without cloning or moving the whole container.
  4. Delimit the borrow with an explicit `{}` block to release it
     before reusing the original data.
  5. Split the struct into independent fields if the borrow checker
     rejects borrows of unrelated fields within the same scope.
  6. `.clone()` is the **last** option, and only if the cost is
     acceptable and no reasonable alternative exists.
- If you write `.clone()`, mentally justify why none of the above
  options apply; if you can't justify it, don't do it.

### `Cow<B>`

- Use it whenever a function may return either borrowed or owned data
  depending on a condition, to avoid cloning in the common case
  (`Cow<str>`, `Cow<[T]>`). Signal: an `if` where one branch does
  `.to_string()`/`.to_owned()` and the other returns the data as-is.

### Iterators

- Always prefer the adapter chain (`map`, `filter`, `fold`, `zip`,
  `take`, `collect`, etc.) over imperative loops with indices or
  intermediate `Vec`s, unless the loop better expresses intent or
  needs complex control flow (`break` with a value, multiple exits).
- When building a return value from an argument you're iterating
  over, recover the value from the iterator (`.next()`, `into_iter()`)
  instead of indexing and cloning.
- Don't hand-implement `Iterator` if `iter()`/`iter_mut()`/
  `into_iter()` plus standard adapters already solve the case.

### Function Signatures and Types

- In **parameters**: always use the most generic borrowed type
  possible — `&str` over `&String`, `&[T]` over `&Vec<T>`, `&T` over
  `&Box<T>`.
- In **structs**: use the owned type (`String`, `Vec<T>`).
- Return `&str`/`&[T]` only if the slice is part of something already
  borrowed; return the owned type if it's new data.
- If a function takes `&mut T` but is conceptually a `T → T` or
  `&T → NewT` transformation, that's a red flag: change the signature
  to return the result instead of mutating in place.

### Data Structures and Allocation

- Choose the structure based on the access pattern: frequent lookups
  by key → `HashMap`/`BTreeMap`, not a `Vec` with linear search.
- Avoid avoidable reallocations: if you know the final size, use
  `Vec::with_capacity`, `String::with_capacity`, etc.
- Don't optimize without an identified bottleneck (measure before
  optimizing), but the rules above (clone, Cow, iterators, signatures)
  always apply — they are not "speculative optimization": they are
  basic Rust design hygiene.
- Use [note: incomplete in source]

---

## 2. Conversions

- Before writing any manual conversion logic between types, check
  whether `From`/`TryFrom` already exists (or can be implemented), or
  an `into_*` / `as_*` / `to_*` / `From` method exists on the source
  or destination type (your own or a dependency's). If it exists,
  **always use it** instead of rewriting the conversion inline.
- If a conversion is repeated in more than one place and no method
  exists for it, create one (`impl From<X> for Y` or
  `fn as_x(&self) -> X`) before duplicating the logic.
- Respect prefix semantics when naming: `into_*` consumes and returns
  owned; `as_*` is cheap and preserves the reference/representation;
  `to_*` is expensive (allocates/copies) and may keep the original.

---

## 3. Making Invalid States Unrepresentable

- If you see `if !self.initialized`, repeated defensive checks before
  using a value, or two `bool`s in a struct that together create
  impossible states (`is_open: true, is_closed: true`): don't leave it
  that way.
- Action: a private constructor or `TryFrom` that guarantees the
  invariant at construction, or an `enum` that makes the invalid state
  unrepresentable. Validation moves to construction, not usage.
- `Option<bool>` where `None`/`Some(false)`/`Some(true)` mean things
  unrelated to "absence" → an enum with three named variants.

---

## 4. Pattern Selection: Quick Reference Table

| Situation                                                         | Solution                                                          |
| ----------------------------------------------------------------- | ----------------------------------------------------------------- |
| Behavior variants known at compile time                           | `enum` + exhaustive `match`                                       |
| Open-ended variants / plugins / external types                    | `Box<dyn Trait>`                                                  |
| Stateless interchangeable algorithm                               | closure / fn pointer                                              |
| Stateful interchangeable algorithm                                | generic `impl Trait` or `Box<dyn Trait>`                          |
| Resource that must always be released                             | `impl Drop` (RAII)                                                |
| Constructor fields or optional fields with validation             | Builder (`derive_builder`)                                        |
| Type safety over a primitive                                      | Newtype (tuple struct)                                            |
| Notification to multiple consumers                                | Channels (`mpsc`, `broadcast`, `crossbeam`)                       |
| Shared mutable state across tasks, one-way communication          | Channel, not `Arc<Mutex<T>>`                                      |
| Unavoidable `unsafe` code                                         | Encapsulate in the smallest possible module, 100% safe public API |
| Trait with a single permanent implementation, no DIP/testing need | Remove it, use the concrete type (YAGNI)                          |

- Don't mix without reason: `Box<dyn Trait>` where a generic solves
  the case adds unnecessary heap allocation.
- `Deref` is only for smart pointers. Never use it to emulate
  inheritance between unrelated structs; use explicit composition.
- `#![deny(warnings)]` is forbidden in library code (it breaks with
  new compiler lints). The CI equivalent is `RUSTFLAGS="-D warnings"`,
  or selective `#[deny(...)]`.

---

## 5. Errors as Values

- A function that fails predictably (parsing, I/O, input validation)
  returns `Result<T, E>`, never `panic!`/`unwrap()`/`expect()` on that
  path.
- `expect()` with a message explaining the invariant is acceptable
  only when the panic is genuinely impossible and that invariant is
  documented.
- If a function's possible errors are known and finite, consolidate
  them into your own error `enum` (with `thiserror`) instead of
  `Box<dyn Error>`, and implement `From` so `?` works.
- Validate at the entry point with guard clauses and early return; the
  main logic should stay at the lowest indentation level (avoid the
  "pyramid of doom").

---

## 6. Formatting and Organization

- Maximum 80 characters per line, including docs and comments.
- 2-space indentation, never tabs.
- Order within the file: public functions first, private ones near
  their caller (proximity), helpers at the end.
- Constants always at the top of the module; never bare magic values
  scattered through the logic.
- One blank line between logical sections of a function and before a
  comment that introduces a new block. Never a blank line right at
  the start of an `if`/`for`/`match`.
- Declare variables as close as possible to where they're used, unless
  that conflicts with reducing nesting (nesting wins).
- `let mut` only when truly mutated; if the data stops being mutated
  past a point, rebind to immutable (`let data = data;`).

---

## 7. Documentation (rustdoc)

- Always in English. Document before implementing when possible; in
  any case, every public item is documented without exception before
  closing the task.
- Private items: document only if the logic isn't obvious.
- Any code change implies reviewing and updating its doc comment and,
  if the module's responsibility changes, its header.
- Code comments (`//`) explain the **why**, never the what; if you
  need to explain the what, the name is poorly chosen.
- `// SAFETY:` notes and panic-invariant notes are never removed for
  seeming "redundant."
- The general format would be:
  - First line: concise description of what this crate is for.
  - Subsequent lines, with one blank line in between, a full
    explanation covering functionality, special cases, and examples
    if needed (especially for public libs). Don't list types unless
    strictly necessary as an aid to understanding something — types
    are already shown via `cargo doc`.

---

## 8. Testing

- All code requires tests, except pure CI/CD infrastructure (and even
  then, document the reason for the exception in the code).
- Target coverage 80-90%, measured by behaviors, not lines.
- Priority: critical business logic > errors and edge cases > secondary
  paths.
- Mandatory edge cases: empty collections, nulls/absent values, exact
  range boundaries, empty vs. whitespace-only strings, overflow if
  applicable, concurrency if there's shared state.
- Arrange/Act/Assert structure without mixing; independent tests, not
  dependent on execution order.
- Two or more cases that only differ in input data → use
  **DDT with `rstest`**, don't copy/paste the test. Identify each case
  with the id format: `"What is being tested.What is expected"`.
- Mock only what crosses system boundaries (network, disk, time,
  external services). Don't mock the system's own internal logic — if
  you need to, the logic and side effects are mixed and must be
  separated first.
- Don't test trivial getters/setters, generated code, or third-party
  libraries.

---

## 9. Code Analysis

- Run `cargo clippy --all-targets -- -D warnings` to analyze the code.
- Run `cargo doc --no-deps` to analyze the documentation.

---

## 10. Preferred Crates and Tools

- `derive_builder` for the Builder pattern, instead of implementing it
  by hand.
- `thiserror` for error types in libraries.
- `crossbeam::channel` instead of `std::sync::mpsc` when you need
  `select!` over multiple channels, timeouts, or bidirectional
  channels.
- `include_str!()` / `include_bytes!()` to embed assets, instead of
  reading them at runtime.
- `dbg!()` during development instead of `println!("{:?}", ...)`.
- `split_at_mut()` to split a slice into two independent mutable
  parts without cloning, when you need parallelism over the same
  buffer.