---
description: Verify a Rust crate signature/usage against local source instead of memory — only when triggered by a build/test error, a version mismatch, or an unfamiliar crate. Special emphasis on rig-core. Do not use for well-known stable crates with no error.
---

## When to use (this is what saves tokens)

Don't trigger on "I feel unsure" — that's unreliable. Trigger only on objective signals:
- A `cargo build`/`cargo test` error pointing to an API mismatch → search **only** the symbol named in the error.
- A crate you have little/no training exposure to, or `Cargo.lock` version looks uncertain.
- **rig-core**: treat as higher-risk by default (small, fast-moving, likely underrepresented) — verify before writing non-trivial code against it, even without an error.

Skip entirely for common/stable crates or something you know well (`serde`, `tokio`, `std`...) with no error — just write the code.

## How to search (cheap, in order)

1. **Find the path** (don't hardcode the registry hash):
   ```bash
   find ~/.cargo/registry/src -mindepth 2 -maxdepth 2 -type d -iname "<crate>-*"
   ```
2. **Check the real version** in use: `grep -A2 '^name = "<crate>"' Cargo.lock`
3. **Grep the symbol**, narrow pattern, small context — never open whole files:
   ```bash
   rg -n "pub fn new|pub struct Agent|pub trait " <path>/src
   rg -n -A 8 "<exact_symbol>" <path>/src   # only if you need surrounding context
   ```
4. **Read only the matched lines** if more context is needed: `sed -n '120,160p' <file>`
5. **Check `examples/`** for real usage if the signature alone isn't enough: `rg -n "<Type>::new\(" <path>/examples`
6. **Web only as last resort**, and only the exact page: `https://docs.rs/<crate>/<version>/<crate>/`

rig-core path pattern: `rig-core-<version>/src`, main code in `src/agent.rs`, `src/completion/`, `src/providers/`.
