# ADR 0001 — Donation-resistant share pricing

Status: **proposed** (awaiting review)

Drives: [#9](https://github.com/data-cartel/fund/issues/9) (deposit inflation
attack). Couples with the future NAV / off-vault AUM accounting feature.

## Context

`deposit_handler` prices shares from `vault.amount`, the raw SPL balance of the
vault token account. Anyone can transfer quote tokens directly into that account
without minting shares, so the AUM that drives the share price is
attacker-controllable. This is the classic ERC4626 first-depositor inflation
attack (full write-up in #9): a 1-unit first deposit plus a large direct transfer
lets an attacker deny or steal from later depositors.

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

Price with a constant offset: `shares = amount * (supply + V_SHARES) /
(aum_before + V_ASSETS)`. `V_ASSETS` fixes the unrecoverable fraction of any
donation, while `V_SHARES = 10^offset` is what drives the attack cost: it
raises the pricing precision, so rounding a victim deposit of size `D` down to
zero shares takes a donation of roughly `D * V_SHARES / V_ASSETS`. For a
reasonable offset the attack costs far more than it can capture.

- Pro: minimal change (formula only), well-understood (OpenZeppelin default).
- Con: still reads `vault.amount` as AUM, so it does **not** address off-vault
  accounting; only makes the attack costly, not impossible.

### B. Internal asset accounting (`total_assets` on `Fund`)  — recommended

Track `total_assets` on the `Fund` account: increment on deposit, decrement on
withdraw (and later adjust on fee accrual / NAV attestation). Price shares from
`total_assets`, never from `vault.amount`. A direct transfer raises
`vault.amount` but not `total_assets`, so it cannot move the share price.

- Pro: fully donation-proof; this is the same internal-AUM mechanism the
  off-vault NAV model needs anyway, so it is not throwaway work.
- Con: `total_assets` must be maintained correctly across **every** value flow
  (deposit, withdraw, fees, off-vault P&L) — a maintenance bug mis-prices shares.
  The first-deposit rounding edge still needs a guard.

### C. Dead shares / minimum first deposit

Burn the first N shares, or require a minimum initial deposit.

- Pro: simplest.
- Con: imperfect, wastes value, ignores the off-vault accounting problem.

## Recommendation

**B, combined with a small virtual offset from A** for the first-deposit rounding
edge. Rationale: the fund must track AUM internally to value off-vault positions,
so making the share price depend on internal accounting (not `vault.amount`)
eliminates the donation vector *and* lays the foundation for NAV attestation
(the future fees / off-vault feature). The virtual offset neutralizes the
first-depositor rounding manipulation that internal accounting alone still leaves.

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

## Open question for review

Do we want `total_assets` now (v0, vault-only) and generalize to attested NAV
later, or design the NAV-attestation authority up front so deposit pricing is
correct against off-vault positions from day one? This changes how much of the
NAV feature must land before deposit is safe to ship.
