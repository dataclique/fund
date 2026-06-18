# Sealevel Attacks Reference

Concise field guide to the attack classes catalogued in
[coral-xyz/sealevel-attacks](https://github.com/coral-xyz/sealevel-attacks).
Each entry summarises the vulnerability, the Anchor/Solana mechanism that
enables it, and the idiomatic Anchor mitigation. Use this as a checklist when
designing or reviewing any instruction in the `fund` program.

## Signer Authorization

The handler treats an account as an "authority" but never verifies that the
account actually signed the transaction. Solana exposes signedness as a per-
account flag on `AccountInfo`; if the program does not check `is_signer` (or use
a typed wrapper that does), any caller can pass an arbitrary pubkey and
impersonate the authority. Mitigation: type the account as `Signer<'info>`
(or `#[account(signer)]`) so Anchor enforces the check at deserialisation. Any
account whose role is "must approve this action" must be a `Signer`.

## Account Data Matching

The handler deserialises an account (e.g. an SPL (Solana Program Library) token account) and acts on its
contents without validating that the related accounts agree with each other —
for example, reading a token balance without confirming the passed `authority`
is actually the token's `owner`. Anchor's loose account types (`AccountInfo`)
do not cross-check fields between accounts. Mitigation: add explicit
`#[account(constraint = a.field == b.key())]` predicates, or use Anchor's
`has_one = ...` shorthand when one account stores the pubkey of another.
Always assert the relationships your logic depends on.

## Owner Checks

The handler accepts an `AccountInfo`, deserialises bytes, and trusts them
without confirming that the account is owned by the expected program. On
Solana any program can write any layout into an account it owns, so failing to
check the `owner` field lets an attacker pass a look-alike account with
attacker-controlled fields. Mitigation: use Anchor's typed `Account<'info, T>`
or `Program<'info, T>` wrappers — they verify the owner program automatically.
For raw `AccountInfo`, assert `account.owner == &expected_program::ID`
explicitly.

## Type Cosplay

Two account structs from the same program have identical byte layouts, and the
handler deserialises bytes into the wrong type without distinguishing them.
Borsh has no inherent type tag, so a `Metadata { account: Pubkey }` account can
be successfully decoded as `User { authority: Pubkey }` and pass an authority
check that was never intended to apply. Mitigation: use Anchor's `#[account]`
macro for state structs (it prepends an 8-byte discriminator) and access them
via `Account<'info, T>` so Anchor checks the discriminator. Never hand-roll
Borsh deserialisation for program state.

## Initialization

A handler that "initialises" an account by writing fields into it can be tricked
into re-initialising an already-populated account, or initialising an account
that another program created with attacker-controlled contents. The hazard
exists because `AccountInfo` has no concept of "freshly created"; the handler
sees raw bytes either way. Mitigation: use Anchor's `#[account(init, payer =
..., space = ...)]` constraint, which CPIs the system program to allocate a
brand-new account and fails if the account already exists. For
re-initialisation across instructions, prefer `init_if_needed` only with
explicit guards, or store an "is_initialized" discriminator and check it.

## Arbitrary CPI (Cross-Program Invocation)

The handler invokes a "token program" or other dependency that was passed in by
the caller, without verifying the program ID. An attacker substitutes a
malicious program that mimics the real one, intercepting transfers or
returning forged results. Mitigation: type the program account as
`Program<'info, Token>` (Anchor checks the address) or add an explicit
`#[account(address = spl_token::ID)]` constraint. Always pin every CPI target
to a known program ID.

## Duplicate Mutable Accounts

A handler takes two accounts of the same type as `&mut` and operates on each,
assuming they are distinct. If the caller passes the same account in both
slots, the second write silently clobbers the first, breaking invariants like
"transfer from A to B." Solana's account model permits this aliasing because
both account metas can reference the same pubkey. Mitigation: add
`#[account(constraint = a.key() != b.key())]` whenever two accounts of the
same type must be different.

## Bump Seed Canonicalization

The handler accepts a Program Derived Address (PDA) bump from the caller and uses it to derive or sign
for an account, instead of computing the canonical bump itself. Multiple bump
values can produce valid PDAs for the same seeds, so accepting an arbitrary
bump lets an attacker forge a non-canonical address that bypasses the program's
intended account uniqueness. Mitigation: declare PDAs with `#[account(seeds =
[...], bump)]` to derive the canonical bump on first use, then store
`pool.bump` in account data and re-derive with `bump = pool.bump` thereafter.
Never read the bump from instruction data.

## PDA Sharing

A single PDA (often derived from a coarse seed like a mint) is reused as the
signing authority for resources that belong to different users, so anyone who
can invoke the program can authorise actions on resources they should not
control. The vulnerability is in seed design, not in any single check. The
handler's `has_one` constraints look correct yet still let an attacker drain
into a destination account they own. Mitigation: derive PDAs from
user-specific seeds (e.g. include the `withdraw_destination` pubkey), and
ensure each PDA's signing authority is scoped to exactly one user/resource. If
a PDA can sign for many things, every "authority" check downstream is moot.

## Closing Accounts

A naive close handler zeroes lamports and transfers them to a destination, but
leaves the account data intact and the account discriminator unchanged. The
transaction's same instruction (or a follow-up with a refund of lamports
through CPI) can revive the now-"closed" account, letting the attacker re-use
its state. Mitigation: use Anchor's `#[account(mut, close = destination)]`
constraint, which transfers lamports, zeroes the data, and assigns the
account to the system program — making revival impossible. If closing
manually, replicate all three steps (and ideally write the
`CLOSED_ACCOUNT_DISCRIMINATOR` sentinel).

## Sysvar Address Checking

The handler accepts a sysvar (e.g. Rent, Clock) as a raw `AccountInfo` and
reads from it without confirming the address matches the real sysvar. Solana
sysvars are just accounts at well-known pubkeys; an attacker can supply a
look-alike account with crafted data and trick the program into using bogus
rent/clock/etc. values. Mitigation: type sysvars as `Sysvar<'info, Rent>`
(or `Sysvar<'info, Clock>`, etc.) so Anchor verifies the address, or add an
explicit `#[account(address = solana_program::sysvar::rent::ID)]` constraint.

## How to use this list

Treat this as a pre-merge checklist for every new instruction in the `fund`
program. For each `#[derive(Accounts)]` struct ask:

- Is every account that authorises something a `Signer`?
- Are all cross-account relationships expressed via `has_one` or `constraint`?
- Are raw `AccountInfo`s avoided in favour of `Account<'info, T>` /
  `Program<'info, T>` / `Sysvar<'info, T>` wherever possible?
- Are PDAs derived from sufficiently specific seeds, with the canonical bump
  stored on-chain?
- Are accounts of the same type that must be distinct guarded by a
  `key() != key()` constraint?
- Are CPI targets pinned to known program IDs?
- Are account closures done via Anchor's `close = ...` constraint?
- Are initialisations guarded against re-initialisation and against
  attacker-supplied pre-existing state?

Any "no" requires either a justification in code review or a fix.
