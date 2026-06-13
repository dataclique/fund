# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Practices that apply repo-wide

- **"Document" always means in this repo.** Notes, learnings, conventions,
  rationale, and rules belong in tracked markdown files (this file,
  `programs/fund/README.md`, ADRs under `adrs/`, or a dedicated doc). Agent
  memory, scratchpads, or `.tmp/` files do **not** count as documentation —
  the next reader (human or agent) will not see them.
- **Non-trivial shell logic lives in `./scripts/<name>.nu` with a paired
  `./scripts/<name>.test.nu`.** "Non-trivial" is anything beyond ~3 lines, any
  command with non-obvious quoting/escaping, or any one-shot probe that might
  be useful again. The test suite must run as part of the script's nix
  derivation (`checkPhase`) so a broken script fails the build. Do not write
  inline bash inside `flake.nix` strings or as `Bash` tool commands for
  anything more than read-only one-liners (`ls`, `git status`, `cargo check`).

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

Pre-commit hooks (via `git-hooks.nix`; the authoritative list is `hooks.nix`) run `nil`, `nixfmt`, `statix`, `deadnix`, `cargo fmt`, `prettier`, and `taplo`. Run `nix flake check` to invoke them outside a commit.

## Version control (GitButler)

This repo uses the GitButler CLI (`but`) for all version-control write
operations (commits, branches, push). Conventions:

- **Pre-commit hooks:** `but commit` runs them. If you must `but amend`/`but rub`
  (which skip hooks), run `nix flake check` afterward and fold in any fixes
  before pushing.
- **Commit messages:** lowercase; use conventional prefixes where they fit —
  `feat:`, `fix:`, `docs:`, `chore:`, `refactor:`, `test:`. Explain _why_ in
  the body. Never add "Generated with ..." or co-author trailers.
- **Branch names:** `<type>/<kebab-description>`. Always pass an explicit name
  to `but branch new`.

The dev shell's generated gitbutler skill (`.claude/skills/gitbutler`, via the
but.nix `devenvModule` in `flake.nix`) points here — this section is the
source of truth.

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

## SBF toolchain (the `cargo-build-sbf` shim)

`anchor build` ultimately invokes `cargo-build-sbf` (a cargo extension shipped
with `solana-cli` from nixpkgs). Naively it tries to download platform-tools
from `github.com/anza-xyz/platform-tools/releases/latest` at build time, which
defeats reproducibility and breaks on flaky networks. The dev shell wraps it
with `./scripts/cargo-build-sbf.nu` to make builds **offline and pinned**.

### What the wrapper does

1. **Pre-fetches platform-tools via nix** — `flake.nix` pins
   `platformToolsVersion` and the asset `sha256`. Nix downloads + extracts the
   tarball into the store once; CI / contributors share the cache.
2. **Materializes a writable copy** under
   `.devenv/sbf-home/.cache/solana/v<X>/platform-tools/`. cargo-build-sbf
   writes marker files in there, so a read-only nix-store symlink isn't
   enough — the script `cp -r` + `chmod -R u+w`.
3. **Shims `rust/bin/rustc`.** cargo-build-sbf 3.0.12 validates the toolchain
   by running `rustc --version` and matching the regex
   `(rustc [0-9]+\.[0-9]+\.[0-9]+).*toolchain-v`. The released platform-tools
   rustc only prints `rustc 1.89.0-dev`, without the `toolchain-v` marker, so
   the check fails and reports "Solana toolchain is corrupted". The shim
   re-emits `rustc 1.89.0-dev toolchain-v<X>` for `--version` only; every
   other invocation execs the real rustc (renamed to `rustc.real`).
4. **Prepends `platform-tools/rust/bin/` to `PATH`** so cargo-build-sbf and
   the `cargo` it spawns both pick up the Solana rustc (the one with the
   `sbpf-solana-solana` target).
5. **Calls cargo-build-sbf with `--skip-tools-install --no-rustup-override`.**
   Skip-tools-install keeps it off the network; no-rustup-override is
   mandatory because rustup is not in the dev shell (cargo-build-sbf
   otherwise tries to install a `+solana` rustup toolchain).
6. **Strips the leading `build-sbf` arg** that anchor injects (because
   cargo-build-sbf is a cargo extension and gets re-invoked with its own
   subcommand name as `argv[1]`).
7. **Sets `HOME` to `.devenv/sbf-home/`** so cargo-build-sbf's caches never
   leak into the user's actual home.

### Version bumping

When `solana-cli` in nixpkgs (or the platform-tools release we want to use)
moves, update both `platformToolsVersion` and the asset `sha256` in
`flake.nix`. The wrapper's tests in `scripts/cargo-build-sbf.test.nu` are
version-agnostic; they exercise the shim and copy logic against tempdirs.

### Known unresolved issue: anchor-lang 1.0.x vs platform-tools v1.51

`anchor-lang = "1.0.2"` (matching the `anchor` CLI version from nixpkgs)
transitively requires several crates whose published manifests use the
`edition2024` cargo feature (e.g. `cpufeatures 0.3`, `digest 0.11`,
`crypto-common 0.2`, several `solana-*` 3.x crates). cargo 1.84 — the
cargo that ships inside `platform-tools v1.51` — cannot parse manifests
that opt in to `edition2024`, so `cargo metadata` / `cargo build` fail
during the `anchor build` step with:

> error: feature `edition2024` is required
>
> The package requires the Cargo feature called `edition2024`, but that
> feature is not stabilized in this version of Cargo (1.84.0).

Setting `resolver = "3"` and `rust-version = "1.84.0"` on the workspace
does **not** help: the MSRV-aware resolver still lands on `cpufeatures
0.3.0` (the lowest version the rest of the dep graph accepts), and that
version requires Rust 1.85+. The dep graph of anchor-lang 1.0.x has no
1.84-compatible solution.

Resolving this is a project-level version decision, **not** a wrapper
fix:
- **Downgrade `anchor-lang`** to an older version whose deps fit under
  cargo 1.84 (the `0.31.x` line is the obvious candidate). This also
  means using an `anchor` CLI to match.
- **OR upgrade the SBF toolchain**: bump `solana-cli` in nixpkgs (or
  override it in `flake.nix`) to a release whose platform-tools ships
  cargo 1.85+. cargo-build-sbf's internal version regex will need to be
  satisfied by the corresponding rustc — the shim already covers this
  for arbitrary `toolchain-v<X>` markers.

Pick a direction, then run `nix run .#regenerate-cargo-lock-sbf` to
re-pin the lockfile and `nix run .#probe-cargo-build-sbf -- --clean
--manifest-path programs/fund/Cargo.toml` to verify the build.

### Things we tried that don't work

- **Symlinking the nix store path** instead of copying: cargo-build-sbf wants
  to write into the install dir; read-only fails.
- **Unwrapped rustc** (just the released `rust/bin/rustc`): fails the
  `toolchain-v` regex; cargo-build-sbf reports "Solana toolchain is
  corrupted".
- **Setting `RUSTC` env**: cargo-build-sbf logs `Removed RUSTC from cargo
  environment` and drops it before spawning cargo, so RUSTC only affects its
  own version-probe — the real compile then can't find a Solana rustc.
- **Letting cargo-build-sbf manage the install** (drop the shim, drop
  `--skip-tools-install`): subject to network timeouts on the platform-tools
  download (`reqwest::Error { kind: Decode, source: TimedOut }`), which is
  exactly what the pre-fetch is meant to avoid.

## Security

Every new instruction must be reviewed against the Solana/Anchor attack catalogue in @docs/sealevel-attacks.md before merging. Treat the checklist at the bottom of that document as a hard gate — each `#[derive(Accounts)]` struct should be walked through it explicitly.
