# Anchor essentials

Distilled from the upstream Anchor docs at
[`solana-foundation/anchor/docs/content/docs`](https://github.com/solana-foundation/anchor/tree/master/docs/content/docs)
— specifically the `basics/program-structure` and `basics/idl` pages. Read this
before touching the on-chain program — it's the minimum mental model needed to
avoid fighting the framework.

## The four macros

Anchor's behavior is driven entirely by four macros. Everything else (account
validation, Interface Definition Language (IDL) generation, client codegen)
falls out of them.

| macro                 | applies to                             | role                                                                                                                                                |
| --------------------- | -------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| `declare_id!("…")`    | `lib.rs` top-level                     | sets the program's on-chain address. Run `anchor keys sync` to overwrite it with the keypair under `target/deploy/<program>.json`.                  |
| `#[program]`          | a `mod` (usually `mod <program_name>`) | every `pub fn` inside becomes an invocable instruction. The function's name is the instruction's name on the wire and in the IDL.                   |
| `#[derive(Accounts)]` | a struct (one per instruction)         | declares the accounts the instruction takes. Field names are arbitrary; Anchor uses them as IDL account names and for `ctx.accounts.<name>` access. |
| `#[account]`          | a state struct                         | makes a struct a first-class on-chain account: assigns program owner, prepends an 8-byte discriminator, provides serde.                             |

## File layout is _not_ part of Anchor

Anchor does **not** read `.rs` file names. The framework only looks at what's
inside the `#[program]` module and at the `#[derive(Accounts)]` / `#[account]`
structs reachable from `lib.rs`. Splitting code into
`src/instructions/<name>.rs`, `src/state.rs`, etc. is a project convention —
fine to move and rename files whenever the structure no longer reflects the
code. The Anchor command-line interface (CLI) scaffold ships an
`instructions/initialize.rs` only because the default scaffold instruction is
`initialize`; renaming the instruction frees us to rename the file.

## Instruction handler shape

Every instruction handler has the same skeleton:

```rust
pub fn my_instruction(ctx: Context<MyInstruction>, arg1: T, arg2: U) -> Result<()> {
    // ctx.accounts.<name>: validated accounts
    // ctx.bumps.<pda_name>: bumps Anchor derived during validation
    // ctx.program_id: this program's pubkey
    // ctx.remaining_accounts: unvalidated trailing accounts (use rarely)
    Ok(())
}
```

`Context<T>` is generic over the matching `#[derive(Accounts)]` struct. The
struct's name (`MyInstruction`) is also what IDL clients see.

## Account validation

Two complementary mechanisms; you'll usually combine them.

- **Account types** (`Signer<'info>`, `Program<'info, T>`, `Account<'info, T>`,
  `Sysvar<'info, T>`, `Interface<…>`) — Anchor checks ownership/shape based on
  the type alone. Always prefer a typed wrapper over raw `AccountInfo`.
  `UncheckedAccount<…>` is **not** one of these: it performs no validation at
  all (no owner, discriminator, or shape checks), requires explicit constraints
  plus a `/// CHECK:` comment, and should be avoided unless genuinely necessary.
- **`#[account(...)]` constraints** — extra invariants Anchor enforces before
  the handler runs. The most common ones:

  | constraint             | meaning                                                                                                                                                  |
  | ---------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
  | `init`                 | create this account in this instruction. Requires `payer`, `space`, and `system_program`.                                                                |
  | `init_if_needed`       | as above, but skip if the account already exists. **Behind a feature flag — avoid unless necessary; it's a known footgun.**                              |
  | `payer = <account>`    | who pays rent for an `init`'d account.                                                                                                                   |
  | `space = N`            | bytes to allocate. **Must include the 8-byte discriminator: `space = 8 + State::INIT_SPACE`.**                                                           |
  | `seeds = [...], bump`  | declares this account as a Program Derived Address (PDA) derived from the listed seeds. Anchor computes the canonical bump and stores it on `ctx.bumps`. |
  | `bump = <stored_bump>` | use the bump stored in account data instead of re-deriving (faster, used after creation).                                                                |
  | `mut`                  | the account will be modified by the handler. Required for any write.                                                                                     |
  | `has_one = <field>`    | the account's `<field>` (a `Pubkey` member) must equal another account in the struct. Cross-account integrity check.                                     |
  | `constraint = <expr>`  | arbitrary boolean over accounts/args, e.g. `constraint = vault.amount >= amount`.                                                                        |
  | `close = <account>`    | mark this account for closure on success; rent flows to `<account>`.                                                                                     |

  See `@docs/sealevel-attacks.md` for which constraints defend against which
  attack class.

  **Constraint order matters.** Anchor parses an `#[account(...)]` list in
  source order and rejects certain orderings at compile time: `init` must come
  before its `payer`, `space`, and any `token::*` / `mint::*` /
  `associated_token::*` sub-constraints; `seeds` must come before `bump`; `mut`
  before `close`. Out-of-order constraints are a build error (e.g.
  `init must
  be provided before payer`). So write `init` / `init_if_needed`
  first, then its `payer` / `space` / `seeds = [...], bump`, then any `mut` /
  `has_one` / `constraint` checks.

## State accounts and the 8-byte discriminator

`#[account]` makes a struct an on-chain account:

```rust
#[account]
#[derive(InitSpace)]
pub struct Fund { /* fields */ }
```

- **Discriminator**: Anchor prepends 8 bytes to every account's data, equal to
  the first 8 bytes of `sha256("account:<StructName>")`. On read, Anchor checks
  it and refuses to deserialize an account of the wrong type — this is what
  makes `Account<'info, Fund>` safe.
- **`#[derive(InitSpace)]`**: generates `Fund::INIT_SPACE` as the byte size of
  the struct's fields. Use it in `space = 8 + Fund::INIT_SPACE` so a layout
  change automatically reflows the allocation.
- **Discriminator and `space` for instructions**: instruction discriminators use
  `sha256("global:<instruction_name>")`. Anchor injects them transparently; you
  do not need to do anything.

## IDL is generated, not written

`anchor build` writes `target/idl/<program>.json` and a TypeScript types file at
`target/types/<program>.ts`. The IDL is derived from the Rust source:

- Each `pub fn` in `#[program]` becomes an entry under `instructions`.
- Each `#[derive(Accounts)]` struct becomes the `accounts` array of its
  instruction.
- Each `#[account]` struct becomes an entry under `accounts` and `types`.
- Argument types and account-data fields are mapped to IDL primitives.

Clients then consume the IDL:

```ts
import type { Fund } from "../target/types/fund";
const program = anchor.workspace.Fund as anchor.Program<Fund>;
await program.methods.createFund(params).accounts({ … }).rpc();
const fund = await program.account.fund.fetch(fundPda);
```

The `accounts({ … })` keys are the snake_case field names from the
`#[derive(Accounts)]` struct, converted to camelCase. The same camelCase
convention applies to instruction args and account fields fetched via
`program.account.<state>.fetch`.

## Conventions for _this_ repo

- Each instruction lives at `programs/fund/src/instructions/<name>.rs`,
  re-exported from `instructions.rs`. State (`#[account]`) goes in `state.rs`.
- `lib.rs` only contains `declare_id!` and the `#[program] mod fund { … }` block
  whose functions delegate to `<name>::handler`. Keep the bodies a one-liner.
- Account constraints on PDAs always store and reuse the bump (`bump = …` after
  creation; `bump` alone at `init` time). The stored bump goes on the parent
  account so descendants can re-derive without searching.
- Hand-deriving things Anchor already does — discriminators, account-owner
  checks, bump search — is a smell. Look for a constraint or typed wrapper
  first.
