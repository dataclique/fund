# `fund` — on-chain program

Anchor program deployed under ID `5nNVyzESLk4QNQh7HgxAAwFmHnN37WUz1aCttBLwFo2e`
(see `src/lib.rs` and `../../Anchor.toml` — keep them in sync).

## Layout

- `src/lib.rs` — `declare_id!` and the `#[program]` module. Each public program
  function is a thin delegate to `instructions::<name>::handler`.
- `src/instructions/` — one file per instruction; re-exported via
  `instructions.rs` so `Initialize` / `initialize::handler` are reachable as
  `fund::Initialize` / `fund::initialize::handler`.
- `src/state.rs` — account state structs.
- `src/constants.rs`, `src/error.rs` — shared constants and `#[error_code]`
  enums.
- `tests/` — Rust integration tests driven by
  [`litesvm`](https://docs.rs/litesvm), not `anchor test`.

## Build & test

From the repo root:

```bash
anchor build      # produces target/deploy/fund.so via the cargo-build-sbf shim
cargo test        # litesvm tests; depends on the .so produced above
```

`anchor build` must run before `cargo test` — the integration tests
`include_bytes!` the compiled `.so` at compile time.

## Adding a new instruction

1. Create `src/instructions/<name>.rs` with
   `#[derive(Accounts)] pub struct
   <Name>` and `pub fn handler(...)`.
2. Add `pub mod <name>; pub use <name>::*;` to `src/instructions.rs`.
3. Add a thin wrapper inside `#[program] mod fund` in `src/lib.rs` that calls
   `<name>::handler(ctx, ...)`.
4. Walk the new `#[derive(Accounts)]` through the checklist in
   `../../docs/sealevel-attacks.md` before merging.

For higher-level context (dev shell, toolchain, continuous-integration (CI)
conventions) see the repo root `../../CLAUDE.md`.
