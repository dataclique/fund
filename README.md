# fund

Solana program (Anchor) for the moneymentum fund/vault: depositors swap a quote
token (e.g. USD Coin (USDC)) for share tokens, capital is deployed to
Turnkey-controlled trading wallets across venues, and net-asset-value
(NAV)-based accounting governs share pricing, fees, and a two-phase withdrawal.
It lives in its own repository because the Solana toolchain pins versions the
main monorepo can't use.

> [!NOTE]
> Early development. The first instructions — `create_fund` and `deposit` — and
> the design spec at `programs/fund/SPEC.md` are in review; withdraw, NAV
> attestation, and fee collection are in progress.

## Develop

The dev shell (Nix flake + devenv, auto-loaded via direnv) provides the Solana
toolchain, the offline `cargo-build-sbf` shim, and the GitButler command-line
interface (CLI; `but`, sourced from the
[`but.nix`](https://github.com/data-cartel/but.nix) flake input — in review).

```bash
direnv allow   # or: nix develop
anchor build   # compile the on-chain program -> target/deploy/fund.so
cargo test     # Rust tests
```

Run `anchor build` before `cargo test`: litesvm integration tests under
`programs/fund/tests/` `include_bytes!` the compiled `fund.so` (see the test
architecture section in [`AGENTS.md`](AGENTS.md)).

## Layout

- `programs/fund/src/lib.rs` -- `declare_id!` and the `#[program]` module; each
  instruction is a thin delegate to `instructions::<name>::handler`.
- `programs/fund/src/instructions/` -- one file per instruction.
- `programs/fund/src/state.rs` -- on-chain account state.
- `programs/fund/src/constants.rs`, `error.rs` -- shared constants and error
  codes.

## Docs

- [`CLAUDE.md`](CLAUDE.md) -- repo conventions, the SBF toolchain shim, agent
  rules.
- [`docs/sealevel-attacks.md`](docs/sealevel-attacks.md) -- the security
  checklist every instruction is reviewed against before merge.
- `docs/anchor.md` and `programs/fund/SPEC.md` -- Anchor essentials and the
  fund/vault design.
