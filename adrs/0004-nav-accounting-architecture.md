# Architecture Decision Record (ADR) 0004 -- Net asset value (NAV) accounting architecture: hub-and-spoke evaluation and a double-entry internal ledger

Status: **proposed** (awaiting review)

Drives: a review of the LayerZero x Centrifuge report "Unlocking Tokenized Fund
Composability" (its "NAV and pricing across chains" section and the
combined-solution sections) against this fund's cross-chain NAV oracle design
(ADRs [0001](0001-donation-resistant-share-pricing.md),
[0002](0002-tiered-off-solana-nav-inclusion.md),
[0003](0003-permissionless-nav-attestation.md), and
[security-design.md](../docs/security-design.md)). Question posed by the owner:
can we re-architect NAV management to follow that report's pattern, and would it
improve anything?

## Context

The report and this fund both say "multi-chain NAV management," but they solve
**inverse** problems:

- **The report (LayerZero / Centrifuge).** One fund; its share token is
  distributed across many chains. NAV is computed at a single **hub** chain and
  propagated _outward_ to spoke chains, where investors deposit, mint, and burn.
  The problem solved is **price _consistency_ across distribution endpoints**.
  The hub is **trusted by assumption** -- a transfer agent / administrator
  computes the true NAV -- and the per-spoke "oracle" in their diagram is a
  price _receiver_, not a price _discoverer_.
- **This fund.** Solana _is_ the hub and the only chain where shares live. The
  hard problem is trustlessly _ingesting_ off-Solana asset value (Hyperliquid
  perpetual-swap equity, Derive options) _inward_ to **compute** NAV at all. The
  problem solved is **trust-minimized _valuation_**.

The report therefore has no story for our central question -- _who is honest
about the off-chain number_ -- because it assumes the hub already knows it. That
assumption is precisely the one ADR 0003 opens by rejecting: "if the operator
alone tells the vault what the off-Solana capital is worth, share pricing is
manager-attested... a Stream-Finance-class trust assumption."

Decomposing the report's pattern into components lets us judge each on its own:

1. **Hub-and-spoke authoritative state** -- one chain owns accounting / NAV /
   share-class / compliance; others are endpoints.
2. **Generalized cross-chain messaging** (LayerZero Omnichain App (OApp))
   carrying arbitrary state (NAV updates, compliance) between hub and spokes.
3. **Async settlement** -- the ERC-7540 (asynchronous tokenized-vault standard)
   request/claim lifecycle: orders raised on any chain travel to the hub, enter
   a global epoch queue, price at hub NAV, settle back.
4. **Double-entry bookkeeping with complete-state reconciliation** -- every
   value movement is a balanced journal entry; "a NAV recalculation only
   triggers when a spoke's full state has arrived, so incomplete data never
   affects pricing."
5. **Accounting tokens for in-flight value** -- a receipt token on the source
   and a liability token on the destination so capital mid-transfer does not
   "disappear from NAV."

Of these we already have (3) in a stronger form -- the security-design.md
Section 6 request -> epoch-finalized claim machine, re-derived for Solana, with
adverse-to-actor `max`/`min` forward pricing -- and a scalar form of the
internal-ledger half of (4) (ADR 0001's `total_assets`). The live questions are
(1)/(2) (do we distribute shares multi-chain?) and the disciplined forms of
(4)/(5) (does restructuring internal accounting improve NAV correctness?).

## Decision

Split the evaluation into two directions with **opposite value signs**: adopt
the inbound accounting discipline, decline outbound share distribution absent a
product trigger, and (optionally) reframe the whole tier model as one
double-entry ledger.

### Direction 1 -- Inbound accounting discipline (ADOPT)

Three of the report's ideas improve NAV correctness **without changing where
shares live or any trust boundary**:

1. **Double-entry internal ledger.** Today share price derives from the scalar
   `total_assets` (ADR 0001), whose own stated residual is "a maintenance bug
   mis-prices shares" -- it must be updated correctly across _every_ value flow.
   Restructure internal accounting so each movement (deposit, withdraw, fee,
   off-vault profit and loss, cross-venue transfer, repatriation) is a balanced
   journal entry. The continuously-checkable invariant
   `assets = liabilities + equity` turns a class of silent mis-pricing bugs into
   an on-chain assertion failure. This is an implementation-discipline change,
   not a trust-model change.
2. **Complete-state reconciliation gate on the epoch NAV.** The report's "NAV
   recalculation only triggers when a spoke's full state has arrived" maps to:
   an epoch's `redeemable_nav` finalizes only once every leg for that epoch is
   _either_ freshly read _or_ explicitly floored. We already floor stale legs
   (Section 6), but framing the epoch NAV as a **reconciliation of all legs'
   journal entries** -- rather than an ad-hoc sum -- makes Principle 8
   ("unproven => rejected", with "no pricing on incomplete data" as this ADR's
   gloss) structural. One deliberate divergence from the report: their hub
   **waits** for completeness; we **never wait** (waiting = freeze, forbidden on
   the redemption path by Principles 4 and 5), so our gate is
   **"reconcile-or-floor," never "reconcile-or-block."**
3. **Proof-gated in-transit accounting token.** Adopt the report's
   receipt/liability pattern only in the proof-gated form specified in
   [ADR 0002](0002-tiered-off-solana-nav-inclusion.md)'s "In-transit accounting
   token" refinement: it smooths the repatriation NAV dip, but credits
   in-transit value only against rail 6's consume-once repatriation proof tuple,
   fails safe to the current Section 8 exclusion, and nets to zero against the
   cumulative-outflow latch.

**Unifying frame (optional presentation change).** The whole tier model can be
expressed as one double-entry ledger:

- **Tier 1** = native Solana journal entries (oracle-priced, full value);
- **Tier 2** = Hyperliquid consensus-read **liability tokens** (capped,
  pessimistically floored, slow-up-lagged per ADR 0002);
- **In-transit** = **proof-gated receipt tokens** (the ADR 0002 refinement);
- **Tier 3** = side-pocket claim-partition entries (non-redeemable until
  realized).

This changes no trust boundary and no pricing formula -- each entry keeps the
constraints its tier already carries -- but it makes the model's invariants
expressible as ledger assertions (every leg balances; the redeemable bases are
sums over entries of a known tier set), which is easier to test than prose.

### Direction 2 -- Outbound share distribution (DECLINE, absent a product trigger)

The report's literal architecture is multi-chain _share-token distribution_:
investors deposit/redeem on many chains, Solana as accounting hub, NAV pushed
outward. We _could_ build this (Solana <-> Ethereum Virtual Machine (EVM)
messaging exists via Wormhole and LayerZero), but it does **not improve NAV
management** -- it strictly enlarges the attack surface:

- It re-introduces the report's own **consistency gap**: the window between a
  hub NAV finalization and its arrival at each spoke, during which spokes quote
  stale prices. That is the 2003-Mutual-Fund-Scandal stale-price arbitrage the
  report itself cites, reappearing as a cross-chain artifact that "grows with
  each chain added."
- A spoke pricing local deposits/redemptions against a propagated NAV inherits
  every Tier-2 Hyperliquid residual, **plus** a new staleness gap, **plus** the
  outbound messaging layer's integrity -- a second cross-chain trust surface
  beside the inbound Wormhole Queries read, demanding the same channel scrutiny
  ADR 0003 applies to attestation channels.
- **One favorable nuance** versus the report: with Solana as hub, propagation
  flows _out of our most-verifiable chain_, so the report's central weakness (a
  trusted _manager_ hub) does not apply -- the origin is the trust root, not a
  manager's word. But the propagated value still embeds the Tier-2 read, so the
  favorable trust direction does not neutralize the added staleness and
  messaging surface; it only means the _new_ risk is consistency, not origin.

Outbound hub-and-spoke is therefore a **product / distribution** decision (do we
want multi-chain investor access?), not a NAV-management improvement. If a
product mandate for multi-chain investor access ever lands, the non-negotiable
constraints are:

- each spoke prices only against a **finalized epoch NAV**, never a live or
  interpolated price, and applies the same epoch / pessimistic-floor /
  deposit-quarantine discipline locally;
- the propagation messaging is verified to the standard ADR 0002 holds the
  inbound read to (channel integrity, freshness / head-binding, replay / nonce),
  and is added to ADR 0003's channel-typed evidence model rather than trusted as
  a hub broadcast;
- per-chain deposit/redeem capacity counts against the same global epoch caps,
  so distribution cannot widen outflow beyond the single-chain bound.

Until such a mandate exists, shares stay Solana-only and this direction is not
pursued.

## Consequences

- Direction 1 is adoptable incrementally and entirely within the existing trust
  model. The double-entry invariant and the reconcile-or-floor gate are both
  expressible as litesvm assertions, so they land **failing-tests-first** per
  the repo feature workflow. The in-transit accounting token's executable
  contract is owned by the ADR 0002 refinement.
- Direction 2 is explicitly deferred. Recording it here prevents re-litigating
  "why aren't we multi-chain like the report" and pins the constraints any
  future multi-chain-shares work must satisfy.
- No pricing formula and no trust boundary in ADRs 0001-0003 or
  security-design.md changes. This ADR is an accounting-architecture and scope
  decision, not a re-pricing.

## Open questions

1. Whether the double-entry ledger earns its Solana compute / space cost versus
   the current scalar `total_assets` plus targeted invariants -- the journal is
   more state to store and reconcile per epoch, against the benefit of a
   continuously-checkable balance invariant.
2. Whether the unified-ledger framing is reflected in SPEC.md now (as the model
   for the off-vault NAV feature) or only when off-vault inclusion implements.
3. Whether Direction 1's reconcile-or-floor gate should be a distinct,
   investor-visible epoch state (all-legs-fresh versus some-floored), so a
   persistently-floored leg is surfaced rather than silently absorbed into the
   redeemable price.
