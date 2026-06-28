# Architecture Decision Record (ADR) 0001 — Donation-resistant share pricing

Status: **proposed** (awaiting review)

Drives: [#9](https://github.com/dataclique/fund/issues/9) (deposit inflation
attack). Couples with the future net-asset-value (NAV) / off-vault
assets-under-management (AUM) accounting feature.

## Context

`deposit_handler` prices shares from `vault.amount`, the raw SPL (Solana Program
Library) balance of the vault token account. Anyone can transfer quote tokens
directly into that account without minting shares, so the AUM that drives the
share price is attacker-controllable. This is the classic ERC-4626 (Ethereum
tokenized-vault standard) first-depositor inflation attack (full write-up in
#9): a 1-unit first deposit plus a large direct transfer lets an attacker deny
or steal from later depositors.

Two facts shape the decision:

1. The `Fund` account holds **no internal asset accounting** today — the share
   price is read straight off the manipulable balance.
2. The fund's purpose is to **deploy capital off the vault** (to trading venues;
   see the SPEC's "off-vault positions and the corresponding AUM accounting").
   So `vault.amount` was never going to be the true AUM — once capital is
   deployed, AUM = idle vault quote + the value of off-vault positions. An
   internally tracked / attested AUM is required regardless of this attack.

## Options

### A. Virtual shares + virtual assets offset (ERC4626-style)

Price with a constant offset:
`shares = amount * (supply + V_SHARES) /
(aum_before + V_ASSETS)`. `V_ASSETS`
fixes the unrecoverable fraction of any donation, while `V_SHARES = 10^offset`
is what drives the attack cost: it raises the pricing precision, so rounding a
victim deposit of size `D` down to zero shares takes a donation of roughly
`D * V_SHARES / V_ASSETS`. For a reasonable offset the attack costs far more
than it can capture.

- Pro: minimal change (formula only), well-understood (OpenZeppelin default).
- Con: still reads `vault.amount` as AUM, so it does **not** address off-vault
  accounting; only makes the attack costly, not impossible.

### B. Internal asset accounting (`total_assets` on `Fund`) — recommended

Track `total_assets` on the `Fund` account: increment on deposit, decrement on
withdraw (and later adjust on fee accrual / NAV attestation). Price shares from
`total_assets`, never from `vault.amount`. A direct transfer raises
`vault.amount` but not `total_assets`, so it cannot move the share price.

- Pro: fully donation-proof; this is the same internal-AUM mechanism the
  off-vault NAV model needs anyway, so it is not throwaway work.
- Con: `total_assets` must be maintained correctly across **every** value flow
  (deposit, withdraw, fees, off-vault profit and loss (P&L)) — a maintenance bug
  mis-prices shares. The first-deposit rounding edge still needs a guard.

### C. Dead shares / minimum first deposit

Burn the first N shares, or require a minimum initial deposit.

- Pro: simplest.
- Con: imperfect, wastes value, ignores the off-vault accounting problem.

## Recommendation

**B, combined with a small virtual offset from A** for the first-deposit
rounding edge. Rationale: the fund must track AUM internally to value off-vault
positions, so making the share price depend on internal accounting (not
`vault.amount`) eliminates the donation vector _and_ lays the foundation for NAV
attestation (the future fees / off-vault feature). The virtual offset
neutralizes the first-depositor rounding manipulation that internal accounting
alone still leaves.

## Consequences

- `Fund` gains a `total_assets` field; `deposit` (and `withdraw`, when it lands)
  read and update it instead of `vault.amount`.
- The capacity check in `deposit_handler` migrates with the pricing basis: it
  must compare `total_assets + deposit_amount` against `fund.capacity`, not
  `vault.amount + amount`, so a direct donation cannot trigger a false
  capacity-exceeded rejection of legitimate deposits.
- The SPEC's deposit and withdrawal sections must define share pricing in terms
  of `total_assets`, and the NAV-attestation feature becomes the authority that
  reconciles `total_assets` with off-vault position values.
- A reproduction test (the inflation scenario) is the executable contract for
  this fix and must fail before it and pass after.

## Pricing invariants and formulas

This is the executable contract for the fix: the invariants, formulas, and
rounding rules below are binding, and the SPEC, the program, and the
reproduction tests must not drift from them. The single value left open is the
offset magnitude `(V_ASSETS, V_SHARES)`; the SPEC fixes it within the bounds in
the Constants section, and pinning it there is a prerequisite for implementation
— the reproduction-test oracle depends on it — not a forward reference back to
this ADR.

Invariants:

- The share price MUST be derived solely from internal accounting, never from
  `vault.amount`. In the vault-only specialization the internal basis is
  `Fund.total_assets` and the share mint supply; once off-vault exposure exists
  the basis becomes the two-sided redeemable NAV defined in the Release gate
  (per ADR 0002) — still internal accounting, never the raw vault balance.
- A direct token transfer into the vault MUST NOT change any mint or burn quote.
  Only deposit, withdraw, fee accrual, and NAV attestation may move
  `total_assets`, and the accounted idle-quote component changes only through
  program-mediated flows: NAV attestation may adjust only the off-vault position
  component of `total_assets` as defined by the tier model in
  `0002-tiered-off-solana-nav-inclusion.md` (Tier-1 on-chain values and capped
  Tier-2 consensus reads), never unsolicited vault surplus (`vault.amount` in
  excess of the accounted idle-quote component of `total_assets`). Surplus
  handling, if any, is a separate explicit decision. The reproduction tests must
  show that donations made before and after a NAV update leave mint and burn
  quotes unchanged.
- Minting rounds shares DOWN; burning rounds assets DOWN. Both round adverse to
  the actor and in favor of the remaining pool, so no sequence of operations can
  extract value through rounding.

Constants (the virtual offset from option A, applied on every quote, not only
the first deposit):

- `V_ASSETS` — virtual asset offset.
- `V_SHARES` — virtual share offset.

Following OpenZeppelin's ERC4626 default, `V_ASSETS = 1` and
`V_SHARES =
10^offset` for a small decimals `offset`; the larger the offset, the
costlier a donation attack. On Solana the offset also trades directly against
capacity: SPL mint supply is `u64`, and with `V_ASSETS = 1` a deposit of
`amount` quote units mints roughly `amount * 10^offset` share units, so the
offset must satisfy `capacity * 10^offset <= u64::MAX`. With internal accounting
as the primary defense, the offset only guards the rounding edge, so a small
offset (0 or 1) suffices. The SPEC fixes the concrete `(V_ASSETS, V_SHARES)`
pair within those bounds — weighing the offset against the quote-token decimals,
the fund capacity, and the u64 share-supply ceiling — and records the chosen
value; it must not defer back to this ADR, which states the bounds but not the
magnitude. Any nonzero offset also breaks the SPEC's `create_fund` rule that the
shares mint's decimals match the quote mint's (1 quote unit would mint
`10^offset` share units); that SPEC statement must be revised — or the
shares-mint decimals bumped by `offset` — when the offset is chosen.

Formulas (the multiply-then-divide uses a **wider `u128` intermediate** for the
product — `total_shares` / `total_assets` may approach `u64::MAX` as the fund
fills, so a u64 product would overflow a near-capacity deposit/withdraw; the
`checked` requirement applies to narrowing the result back to `u64`, not to that
intermediate product). Here `total_shares` refers to `shares_mint.supply` — the
SPL mint is the authoritative share counter; `Fund` gains no redundant
share-supply field:

- Mint:
  `shares_out = floor(deposit_amount * (total_shares + V_SHARES) / (total_assets + V_ASSETS))`
- Burn:
  `assets_out = floor(shares_in     * (total_assets + V_ASSETS) / (total_shares + V_SHARES))`

Both formulas read `total_assets` and `total_shares` as they stand **before**
this instruction's accounting update. The zero-output guards below are evaluated
against those pre-update values, and `total_assets` / `shares_mint.supply` are
mutated only after the guards pass. These single-base formulas are the
**vault-only specialization**: while the fund holds only idle vault quote the
deposit base equals the redemption base (`total_assets`), so one base suffices.
Once off-vault NAV inclusion lands the two bases diverge — deposits and
redemptions price against the separate bases named in the Release gate below and
formalized in ADR 0002.

Zero-output guards (mandatory, checked before any transfer, mint, burn, or
accounting update):

- A deposit MUST fail if `shares_out == 0`. The existing `FundError::ZeroShares`
  guard in `deposit_handler` is the precedent; it must survive the formula
  change.
- A share-in redemption MUST fail if `assets_out == 0` — no operation may
  consume a user's shares without returning at least one asset unit.

Both zero-output cases belong in the reproduction tests.

State updates:

- Deposit: `total_assets += deposit_amount`; the `mint_to` Cross-Program
  Invocation (CPI) raises `shares_mint.supply` by `shares_out`.
- Withdraw: `total_assets -= assets_out`; the `burn` CPI lowers
  `shares_mint.supply` by `shares_in`.

The virtual offset makes the first deposit price off `(V_SHARES, V_ASSETS)`
rather than `(0, 0)`, so there is no zero-denominator and no single-unit
first-depositor rounding edge to exploit.

## Release gate

Internal `total_assets` is correct only while it captures all fund value. As
long as the fund holds **only** idle vault quote (no off-vault positions),
vault-only `total_assets` is complete and pricing is correct. The moment capital
is deployed off the vault, vault-only `total_assets` understates AUM, and
deposits would misprice and dilute users.

Hard rule (must hold at all times, not an option):

- If off-vault positions can be opened, deposits and withdrawals MUST price
  against the **two-sided** redeemable NAV defined in
  [0002-tiered-off-solana-nav-inclusion.md](0002-tiered-off-solana-nav-inclusion.md).
  The two sides do not share a base: redemptions price against
  `redeemable_nav_redeem = verifiable_nav + floor(read_nav)` (the pessimistic,
  capped, slow-up-lagged Tier-2 floor), while deposits price against
  `redeemable_nav_deposit = verifiable_nav +` the fully-recognized (un-lagged,
  un-floored) capped Tier-2 value taken at the adverse HIGH (or are quarantined
  per ADR 0002's stale-read deposit rule). The pessimistic `floor(read_nav)`
  prices redemptions only and is **never** a deposit entry price — a deposit
  priced at an understated NAV mints excess shares that capture value on
  re-recognition, the exact cheap-entry vector ADR 0002 neutralizes. Tier-3
  `attested_nav` MUST stay excluded from both mint and burn pricing, and
  vault-only `total_assets` is not an acceptable basis once off-vault exposure
  exists. ADR 0002 owns the pricing formulas; this gate fixes only which bases
  are legal on each side.
- Until the ADR 0002 redeemable-NAV accounting is implemented, off-vault
  deployment MUST stay behind a feature flag that is disabled.

Release rule: off-vault enabled requires the ADR 0002 redeemable NAV; until it
lands, off-vault stays feature-flag disabled. This makes the sequencing safe
either way -- shipping the donation fix (vault-only `total_assets`) now is sound
_because_ off-vault stays disabled until the tiered redeemable-NAV model gates
pricing. This section decides the sequencing question (ship the vault-only fix
now vs design NAV inclusion first): ship now, with off-vault hard-gated. What
remains open lives in ADR 0002's owner-ratification list, not here.
