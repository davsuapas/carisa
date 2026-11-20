# Before writing code

- If the plan is ambiguous or violates a rule here, ask the user. Do not resolve it yourself.
- Ask questions and write user-facing messages in the prompt language.
- Remove unnecessary complexity; encapsulate necessary complexity.

# Before marking done

- Update comments/docs if code changed.
- Verify tests, doc tests, formatting, lints, and documentation.

# Navigation

- Read root module header comments first to rule out irrelevant submodules.
- Drill into submodule headers only when unsure or when that is the target.
- If language-server tools exist, use usages `go-to-definition`, `document-symbols` for types and symbols instead of text search.

# Project structure

- `agent/`: config, builders, workflow loop, activity.
- `memory/session/`: Redis storage, conversation messages, streaming.
- `skill/`: builders, manager.
- `telemetry/`: traces by `agentid` and `messageid` (sink, storage).
- `identity`: shared identifiers.

# Performance and memory

This section outranks style and brevity.

## Clones

- Never add `.clone()` just to silence the borrow checker.
- Try in this order:
  1. Reorder operations so the borrow ends earlier.
  2. Extract only the needed scalar (`usize`, `bool`, `f64`) before mutating.
  3. Use `mem::take`, `mem::replace`, or `mem::swap`.
  4. Delimit the borrow with `{}`.
  5. Split the struct into independent fields.
  6. Use `.clone()` last, only if cost is acceptable and no reasonable alternative exists.
- If you write `.clone()`, justify why options 1–5 do not apply.

## `Cow<B>`

- Use `Cow<str>` / `Cow<[T]>` when a function may return borrowed or owned data depending on a condition.
- Signal: one `if` branch calls `.to_string()` / `.to_owned()` and the other returns data as-is.

## Iterators

- Prefer adapter chains (`map`, `filter`, `fold`, `zip`, `take`, `collect`, etc.) over index loops or intermediate `Vec`.
- Exception: the loop better expresses intent or needs complex control flow (`break` with value, multiple exits).
- When building a return value from an iterated argument, recover the value from the iterator (`.next()`, `into_iter()`) instead of   indexing and cloning.
- Do not hand-implement `Iterator` if `iter()` / `iter_mut()` / `into_iter()` plus standard adapters suffice.

## Signatures and types

- Parameters: most generic borrowed type possible: `&str` over `&String`, `&[T]` over `&Vec<T>`, `&T` over `&Box<T>`.
- Structs: use owned types (`String`, `Vec<T>`).
- Return `&str` / `&[T]` only if the slice belongs to already borrowed data; return owned data for new values.
- If a function takes `&mut T` but is conceptually `T → T` or `&T → NewT`, return the result instead of mutating in place.

## Data structures and allocation

- Choose by access pattern: frequent key lookup → `HashMap` / `BTreeMap`, not linear search over `Vec`.
- Avoid reallocations when final size is known: `Vec::with_capacity`, `String::with_capacity`.
- Do not optimize without an identified bottleneck.
- Clone / `Cow` / iterator / signature rules are basic hygiene, not speculative optimization.

# Builders (`derive_builder`)

- Use `derive_builder` for structs with optional or validated fields when possible.
- Optional fields: `#[builder(default, setter(into, strip_option))]`. Drop `strip_option` if the field is not `Option<T>`.
- Required fields: pass them to `FooBuilder::new(...)`, not `build()`.
- Private fields with public getters.
- Validation: `#[builder(build_fn(validate = "Self::validate"))]`.

# Conversions

- Before manual conversion, check for existing `From` / `TryFrom` or `into_*` / `as_*` / `to_*` / `From` methods on source or target. Always use them if present.
- If a conversion repeats and no method exists, create `impl From<X> for Y` or `fn as_x(&self) -> X` before duplicating.
- Prefix semantics: `into_*` consumes and returns owned; `as_*` is cheap and preserves reference/representation; `to_*` is expensive  (allocates/copies) and may keep the original.

# Invalid states

- Do not leave `if !self.initialized`, repeated defensive checks, or two `bool`s that create impossible states.
- Use a private constructor or `TryFrom` to enforce invariants at  construction, or an `enum` that makes invalid states
  unrepresentable. Validate at construction, not usage.
- `Option<bool>` whose variants mean things unrelated to absence → three-variant `enum`.

# Pattern selection

- Compile-time variants → `enum` + exhaustive `match`.
- Open-ended variants / plugins / external types → `Box<dyn Trait>`.
- Stateless interchangeable algorithm → closure / fn pointer.
- Stateful interchangeable algorithm → generic `impl Trait` or `Box<dyn Trait>`.
- Resource that must always be released → `impl Drop` (RAII).
- Type safety over a primitive → newtype (tuple struct).
- Notify multiple consumers → channels (`mpsc`, `broadcast`, `crossbeam`).
- Shared mutable state across tasks with one-way communication → channel, not `Arc<Mutex<T>>`.
- Unavoidable `unsafe` → smallest possible module, 100% safe public API.
- Trait with one permanent implementation and no DIP/testing need → remove it; use the concrete type (YAGNI).
- Do not use `Box<dyn Trait>` where a generic solves the case; it adds unnecessary heap allocation.
- `Deref` is only for smart pointers. Never use it to emulate inheritance between unrelated structs; use explicit composition.

# Errors as values

- Predictable failure (parsing, I/O, input validation) returns `Result<T, E>`, never `panic!` / `unwrap()` / `expect()`.
- `expect()` is acceptable only when the panic is genuinely impossible and the invariant is documented.
- If possible errors are known and finite, consolidate them into a  project error `enum` with `thiserror`, not `Box<dyn Error>`, and implement `From` so `?` works.
- Validate at the entry point with guard clauses and early returns; keep main logic at the lowest indentation level.

# Formatting and organization

- Maximum 80 characters per line, including docs and comments.
- 2-space indentation; never tabs.
- File order: public functions first, private ones near their caller, helpers at the end.
- Constants at the top of the module; no scattered magic values.
- One blank line between logical sections inside a function and before a comment introducing a new block.
- Never put a blank line right at the start of `if` / `for` / `match`.
- Declare variables close to use unless it conflicts with reducing nesting; nesting wins.
- Use `let mut` only when truly mutated; rebind to immutable after mutation stops (`let data = data;`).

# Documentation (rustdoc)

- Always in English.
- Every public item documented without exception before closing.
- Private items: document only when logic is not obvious.
- Any code change requires reviewing/updating its doc comment and, if module responsibility changed, its header.
- `//` comments explain **why**, never **what**; if the what needs explanation, rename.
- Document so future engineers and users understand the system without reading the code.
- Module files need comprehensive documentation, preferably with examples.
- General format:
  - First line: concise description of the crate purpose.
  - Subsequent lines, separated by blank lines: full explanation of functionality, special cases, and examples for public libs.
  - Do not list types in module headers unless strictly necessary to understand something; `cargo doc` shows them.
  - Reference types anywhere in the comment when useful.

# Testing

- All code requires tests.
- Priority: critical business logic > errors/edge cases > secondary paths.
- Mandatory edge cases: empty collections, nulls/absent values, exact range boundaries, empty vs whitespace-only strings, overflow
  if applicable, concurrency if shared state exists.
- Arrange/Act/Assert without mixing; tests independent of execution order.
- Multiple cases of the same function/behavior → a single parameterized test with `rstest` (`#[case]`); don't write a separate `# [test]` per case.
- Mock only system boundaries (network, disk, time, external services). Do not mock internal logic; separate logic and side effects first if needed.
- Do not test trivial getters/setters, generated code, or third-party libraries.

# Code analysis

- `cargo clippy --all-targets -- -D warnings`
- `cargo doc --no-deps`
- `cargo check` (decide whether parameters are needed)
- `cargo test --doc`

# Preferred crates and tools

- `derive_builder` for the Builder pattern; do not hand-implement.
- `thiserror` for library error types.
- `crossbeam::channel` instead of `std::sync::mpsc` for `select!`, timeouts, or bidirectional channels.
- `include_str!()` / `include_bytes!()` to embed assets, not runtime reads.
- `dbg!()` during development instead of `println!("{:?}", ...)`.
- `split_at_mut()` to split a slice into independent mutable parts without cloning when parallelism over the same buffer is needed.