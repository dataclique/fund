# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Anchor-based Solana program named `fund`. Single on-chain program, Cargo workspace, with a TypeScript app/migrations side managed via bun.

This repo is split off from the main monorepo specifically because Solana tooling isn't compatible with everything we use over there. The standing rule that follows from that: **always succumb to whatever Solana tooling wants from us.** Match its version pins, directory layout, env-var conventions, and config defaults rather than fighting them — concretely, if `cargo-build-sbf` expects platform-tools at `~/.cache/solana/v<X>/`, pin v<X> in the flake; don't pick a different version and try to make Solana like it.

- Program ID: `5nNVyzESLk4QNQh7HgxAAwFmHnN37WUz1aCttBLwFo2e` (declared both in `programs/fund/src/lib.rs` and `Anchor.toml`'s `[programs.localnet]` — keep in sync)
- Anchor toolchain: `package_manager = "bun"` (not yarn/npm)
- Rust toolchain pinned via `rust-toolchain.toml` (1.95.0)
- Dev shell provided by Nix flake + devenv (`flake.nix`); `.envrc` loads it via direnv
- `flake.nix` exposes a `cargo-build-sbf` shim that pre-fetches Solana platform-tools (version pinned to match what `solana-cli` from nixpkgs expects) and symlinks them into `.devenv/sbf-home/.cache/solana/v<X>/platform-tools` so `anchor build` runs offline (currently `aarch64-darwin` only)

## Common commands

```bash
# Build the on-chain program (.so artifact under target/deploy/)
anchor build

# Run all Rust tests (also configured as Anchor.toml's test script)
cargo test

# Run a single test
cargo test --package fund --test test_initialize -- test_initialize --exact

# Lint / format JS/TS
bun run lint
bun run lint:fix

# Nix dev shell (auto-loaded via direnv)
nix develop
```

Pre-commit hooks (via `git-hooks.nix`) run `nixfmt`, `nil`, `cargo fmt`, `prettier`, and `taplo`. Run `nix flake check` to invoke them outside a commit.

## Test architecture (important)

Tests do **not** use `anchor test`. They use [`litesvm`](https://docs.rs/litesvm) directly from Rust integration tests under `programs/fund/tests/`.

The pattern (`tests/test_initialize.rs`):

1. `include_bytes!("../../../target/deploy/fund.so")` — pulls the compiled BPF binary at compile time.
2. `LiteSVM::new()` + `svm.add_program(program_id, bytes)` — loads it into an in-memory SVM.
3. Construct an `Instruction` using the Anchor-generated `fund::instruction::*` (data) and `fund::accounts::*` (account metas) types, then send a `VersionedTransaction`.

**Consequence: `anchor build` must run before `cargo test`.** Without `target/deploy/fund.so`, the integration test fails to compile. CI/local workflows should chain build → test.

## Code layout

`programs/fund/src/`:

- `lib.rs` — `declare_id!` and the `#[program] mod fund { ... }` block. Each public program function is a thin delegate to `instructions::<name>::handler`.
- `instructions/` + `instructions.rs` — one file per instruction; the parent module re-exports with `pub use <name>::*` so `Initialize` (account context) and `handler` are reachable as `fund::Initialize` / `fund::initialize::handler`.
- `state.rs` — account state structs (currently empty).
- `constants.rs`, `error.rs` — shared constants and `#[error_code]` enums.

When adding a new instruction:

1. Create `programs/fund/src/instructions/<name>.rs` with `#[derive(Accounts)] pub struct <Name>` and `pub fn handler(...)`.
2. Add `pub mod <name>; pub use <name>::*;` to `instructions.rs`.
3. Add a thin wrapper inside `#[program] mod fund` in `lib.rs` that calls `<name>::handler(ctx, ...)`.

## Wallet / cluster

`Anchor.toml` points provider at `localnet` with wallet `~/.config/solana/id.json`. `litesvm` tests don't touch this — only `anchor deploy` / `anchor migrate` / on-chain interactions do.

## Security

Every new instruction must be reviewed against the Solana/Anchor attack catalogue in @docs/sealevel-attacks.md before merging. Treat the checklist at the bottom of that document as a hard gate — each `#[derive(Accounts)]` struct should be walked through it explicitly.
