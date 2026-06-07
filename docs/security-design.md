# Security design

Status: **proposed, for review.** This grounds the fund's security architecture
in real DeFi incidents (Stream Finance, Cream, Mango, Nomad, Ronin, Kamino,
Balancer, Bybit, Wormhole, Drift, and more). It was produced from a multi-agent
research + adversarial red-team pass; every mandate cites the incident class it
defends against. It supersedes the narrower direction in
[ADR 0001](../adrs/0001-donation-resistant-share-pricing.md) (which addressed
only the donation/inflation slice; ADR 0001's release gate now prices off-vault
deployment against the ADR-0002 two-sided redeemable NAV
(`redeemable_nav_deposit` / `redeemable_nav_redeem`) -- Tier-3 `attested_nav`
stays excluded -- so its hard rule and this document agree) and answers the
deposit-pricing vulnerability tracked in issue
[#9](https://github.com/data-cartel/fund/issues/9).

Section 11 lists the decisions that require a human call before implementation
begins. Nothing here is implemented yet; this is the contract we design against.

---

**The single load-bearing correction.** The naive design's central promise --
"per-venue caps bound the loss to the cap" -- is false, because share price is
**global**: a corrupted off-Solana leg inflates one NAV and one share price, so
the loss propagates to 100% of investor capital through deposits and redemptions,
regardless of how little capital was *deployed* to that venue. The whole design
is organized around the fix: **redeem and price against what you can prove on
Solana; ring-fence everything you can only be told.** Until off-Solana value is
removed from the redeemable price, this fund is Stream Finance in a zk costume --
the exact outcome we must avoid.

---

## 1. Security principles (non-negotiables)

Each principle is a hard invariant tied to an incident class. Violating any one
is grounds to block deployment.

1. **Internal accounting only; never `balanceOf`.** Share price derives from an
   authoritative internal ledger, never from a vault PDA's SPL balance. A direct
   token transfer into the vault moves share price by exactly zero. *(Cream/yUSD
   ~$130M: donation-manipulable balanceOf-as-oracle. On Solana anyone can
   transfer into a PDA's ATA -- not theoretical.)*

2. **NAV separation is mandatory, not optional.** `verifiable_nav` (on-Solana,
   oracle-priced, natively readable positions only) and `attested_nav`
   (off-Solana, venue-API-attested) are distinct quantities. The **redeemable
   share price and the deposit price settle against the redeemable-NAV pair --
   `verifiable_nav` plus independently-read Tier-2 value behind a
   total-loss-tolerable cap, at the pessimistic floor for redemptions
   (`redeemable_nav_redeem`) and at the fully-recognized, adverse-high value
   for deposits (`redeemable_nav_deposit`)
   ([ADR 0002](../adrs/0002-tiered-off-solana-nav-inclusion.md)) -- and against
   nothing else.** Raw `attested_nav` (Tier-3 venue-API value) may inform
   reporting and the side-pocketed claim, but may **never** move the redeemable
   price. The separation stands: Tier 2 earns inclusion by meeting all ADR-0002
   rails; Tier 3 does not. *(Stream Finance ~$93M; Hyperliquid JELLY pooled
   accounting. This is the master fix -- redeem against what you can prove or
   independently read.)*

3. **Attested value is trusted value.** Any number originating from a venue API
   -- even behind zkTLS and a notary committee -- is **venue-API-trusted with
   bounded exposure**, not trust-minimized. We label it as such to investors, in
   those words. *(zkTLS proves "the API returned X at time T," not that X is
   economically true.)*

4. **Liveness of redemption is decoupled from liveness of off-Solana proofs.** A
   proof outage, censored notary, or tripped anomaly breaker must **never**
   freeze redemption of the on-Solana tranche. "Trapped forever" must be
   structurally unrepresentable. *(Analogy: JELLY was an active validator
   governance override that selectively force-settled one contract; the
   construction here is passive -- a manager who simply stops submitting
   proofs achieves the same selective freeze.)*

5. **Fail-closed against theft, fail-open against being trapped.** Halts on
   *data-integrity* signals (oracle divergence, proof staleness, confidence
   blowout) are correct. Halts on *NAV-magnitude moves* (a real crash) are
   forbidden **on the redemption/exit path** -- investors must be able to exit
   at a real, bad price. Scope: two distinct breaker families exist.
   *Integrity breakers* (Section 6) freeze a leg on bad data and must never
   trip on magnitude; the *drawdown latch* (Section 4) is magnitude-triggered
   by design but halts trading and upward recognition only -- never
   redemptions (ADR 0002, pricing mechanics). *(Conflating the two turns
   every crash into a freeze precisely when investors need out.)*

6. **The on-chain program is the only perimeter that must hold alone.** Turnkey's
   enclave policy and the backend are assumed compromisable *together*. The
   program's constraints must be sufficient by themselves. *(BitForge /
   CVE-2023-33241: TEE/MPC is not self-justifying. Turnkey's policy engine is
   configurable, hence not independent of a config-compromise.)*

7. **The backend can destroy value, only slowly and only through allowlisted
   venues -- and that channel is latched shut, not merely rate-limited.** We do
   **not** claim "a backend compromise cannot drain." It can, via
   adversarial-but-allowlisted trades, up to a latching drawdown ceiling.
   *(BigONE ~$27M / CoinDCX ~$44M: backend compromise auto-approved outflows with
   keys intact.)*

8. **Parse, don't validate; unproven => rejected.** No default, zero, or
   uninitialized value may satisfy a NAV-fresh, proof-valid, or
   withdrawal-authorized check. *(Nomad ~$190M: default `0x00` treated as proven.
   Qubit ~$80M: minted on an event without confirming escrow.)*

9. **Caps bound proven exposure, not actual exposure.** Because liabilities can
   hide in un-proven venue endpoints, the only honest cap is one sized so the
   fund can absorb the *entire* off-Solana tranche as a total loss without
   harming on-Solana redeemers. *(Stream's hidden 4.1x leverage was
   off-balance-sheet in endpoints a balance proof never touches.)*

10. **Make invalid states unrepresentable in the type system.** Persistent IDs
    are newtypes; NAV states are enums carrying exactly their valid data;
    `overflow-checks = true`; zero `unwrap`/`expect`/`panic!` in non-test code.

---

## 2. Share accounting + deposit/withdraw pricing math

### Recommendation

**Internal ledger.** The program reads the authoritative share supply from
`shares_mint.supply` (ADR 0001: the SPL mint, with the program PDA as sole
mint authority, is the share counter -- `Fund` gains no redundant share-supply
field; `total_shares` in the formulas below denotes that supply) and computes
price from accounted position state, never from the vault's SPL balance.

**Two-NAV share price.** All deposit and redeem conversions use `verifiable_nav`
exclusively:
- `shares_out = deposit_assets * (total_shares + 10^offset) / (verifiable_nav + 1)` -- round **down**
- `assets_out = redeem_shares * (verifiable_nav + 1) / (total_shares + 10^offset)` -- round **down**

> **Revision note (ADR 0002):** on ratification, the substitution is
> **per-side**, never one symbol into both formulas: `verifiable_nav` in the
> deposit formula becomes `redeemable_nav_deposit` (fully-recognized Tier-2
> value, adverse-HIGH `max(last-good, current)` under staleness/divergence)
> and in the redeem formula becomes `redeemable_nav_redeem = verifiable_nav +
> floored_capped_tier2_nav` (pessimistically floored, capped, slow-up-lagged)
> ([ADR 0002](../adrs/0002-tiered-off-solana-nav-inclusion.md), pricing
> mechanics); the virtual offsets apply unchanged to each combined base. Raw
> Tier-3 `attested_nav` stays excluded.

`attested_nav` is reported separately and is tracked in the explicitly
non-redeemable accounting partition -- a single fungible share class with
claim-token bookkeeping, per Section 11 item 3 (DECIDED), never a second
share mint -- and is never an input to the two equations above.
Realized Tier-3 proceeds credit the non-redeemable partition's
holders-of-record before any residual enters `verifiable_nav` (ADR 0002,
pricing mechanics) -- a deposit made while value sits in Tier 3 acquires no
claim on its later realization, so manager-timed realization cannot transfer
value to fresh deposits.

**Virtual-offset + seed + dead-shares triple defense** (OpenZeppelin ERC-4626
v4.9+ virtual-offset pattern, ported to fixed-point u128/u256; math per the
[OZ ERC-4626 inflation-attack analysis](https://docs.openzeppelin.com/contracts/4.x/erc4626),
Contracts v4.9+). At init the protocol deposits a non-trivial USDC seed and
mints the first shares to an immutable dead PDA. OZ's incident-derived math: an
attacker holding initial deposit `a0` owns share fraction `a0/(1+a0)` and so
recovers only that fraction of a donation; the inflation attack succeeds only
if `10^offset * victim_deposit <= loss` (OZ's condition, with `loss` the
donation fraction the vault captures). A higher offset makes it exponentially more
expensive. *(BakerFi C4 2024-05 #39; JPEG'd HIGH; Stella HIGH; Morpho/MetaMorpho
mitigation.)*

**Seed non-withdrawability is a checked invariant.** The seed and dead shares are
locked by the longest governance timelock, never releasable by routine manager or
backend action. Every redemption asserts: dead shares are never redeemable, and
seed never drops below the offset-protection threshold. The seeder is the
manager's entity (there is no other party at init) and is therefore the adversary
most motivated to find a release path -- the invariant denies it.

**Single fused mul-div, one vault-favoring rounding direction.** Never store a
pre-divided/rounded exchange rate; compute price as one fused operation each
time. u128 intermediates (u256 via two-limb where products exceed u128), checked
arithmetic, `overflow-checks = true` (Solana release builds wrap silently
otherwise). Rounding pinned per EIP-4626: deposit -> shares DOWN, mint -> assets
UP, withdraw -> shares-burned UP, redeem -> assets-out DOWN. *(Kamino Lending
precision-loss bug, caught by Certora, $0 loss: stored a pre-divided truncated
rate, divided by an understated rate, rounded redemption UP in the user's favor.
The fused `collateral * total_liquidity / total_collateral_supply` round-DOWN is
our exact pattern. Balancer V2 ~$128.6M, Nov 2025 -- not the Apr 2023
read-only-reentrancy incident: a rounding-direction mismatch in
ComposableStablePool scaling (`_upscaleArray` rounded down where the
EXACT_OUT swap path required rounding up) let batched micro-swaps compound
precision loss and suppress the invariant; see the incident references.)*

**Forward pricing with precisely pinned, adverse-to-actor semantics.** Pinned in
one place to avoid the request-time/settlement-time free option:
- **Deposits settle at the conversion yielding the FEWER shares**: evaluate the
  full mint formula -- NAV and `total_shares + V_SHARES` snapshotted together
  -- at request and at settlement, and take `min(shares_out)`. A depositor
  never benefits from intra-period appreciation they did not fund.
- **Redemptions settle at the conversion yielding the FEWER assets**: evaluate
  the full burn formula at both epochs and take `min(assets_out)`. A redeemer
  never benefits from intra-period appreciation either.
- The rule is over the **conversion result**, never raw NAV: share supply moves
  between request and settlement (queued deposits, fee minting, dead-share
  handling), so the higher raw NAV can still be the more favorable per-share
  price -- a raw-NAV `max`/`min` reintroduces the free option at epoch
  boundaries.
- This removes the free directional option a single-sided forward price hands
  every requester.
- Each actor is assigned the **adverse NAV side regardless of fund net
  exposure**. We compute a bid/ask NAV pair (confidence-bounded both ways, see
  Section 3) and give depositors the high side, redeemers the low side.
  "Conservative for the fund" is not monotonic when the manager chooses position
  sign -- conservatism must be applied *against the actor*, not against the
  position.

**Cancellation is disallowed after commit** (or forfeits a fee), so the
inter-epoch option cannot be exercised.

**Locked-profit smoothing is removed from the deposit path.** Yearn's
`lockedProfitDegradation` defends *redemption* sandwiching but, under two-phase
forward deposits, *opens* a deposit-side capture vector: deposit at the
smoothed-understated NAV right after a realized gain, capture the deferred
profit. Smoothing is applied only where it is adverse-to-actor consistent with
the `max`/`min` rule above, never as a standalone understatement a depositor can
buy into.

### Defends against
Cream (balanceOf-as-oracle), Kamino (precision/rounding), Balancer V2 (paired
rounding), Yearn yETH/iEarn (stale cached rate, share price excluding strategy
assets), the first-depositor inflation corpus (BakerFi/JPEG'd/Stella),
Harvest/Yearn yDAI (same-block at-NAV arb), and the forward-pricing free-option
class.

### Trust assumptions
`verifiable_nav` is sound (Section 3). Mint authority is the program PDA alone.
Fixed-point precision suffices for the asset universe (Kamino's bug bit only
above 2^59, beyond any token supply; we use the fused pattern regardless).

### Tradeoffs
Requesters do not know exact price at request time (correct safety/UX tradeoff;
ERC-7540/Lido/Ribbon norm). Virtual offset + seed dilute real holders
negligibly. The seed is permanently locked protocol capital -- a real cost
justified by killing the inflation class and by the non-withdrawability
invariant.

---

## 3. NAV & off-chain valuation -- the hard problem, honestly

### 3a. On-Solana legs (Drift, Jupiter perps, Kamino) -- genuinely trust-minimized

**Read positions directly from on-chain program state** (Drift user accounts,
Kamino obligations, Jupiter perp positions). **Price with Pyth/Switchboard under
layered fail-closed checks:**

- **Confidence bounding, applied adverse-to-actor:** not "assets at price-conf,
  liabilities at price+conf" monotonically, but a bid/ask NAV pair where each
  actor gets the side adverse to them. The monotonic rule is exploitable when the
  manager chooses net position sign and deposit timing.
- **Staleness, freshest-update-required:** reject prices older than a bounded
  window *and* require the settlement instruction to consume the **freshest
  available** update written *in the same instruction*, verifying publish-time is
  within a tight bound of the settlement slot. A mere "not older than N slots"
  lets the backend select the most favorable valid-age update
  (oracle-update-selection / staleness-window MEV). Drift's verified reference
  windows: ~10 slots AMM-sensitive, ~120 slots margin (verified against
  [drift-labs/protocol-v2](https://github.com/drift-labs/protocol-v2)
  `programs/drift/src/state/state.rs`, `OracleGuardRails` defaults
  `slots_before_stale_for_amm: 10` / `slots_before_stale_for_margin: 120`,
  master as of 2026-06-10).
- **TWAP-deviation circuit breaker:** reject when spot deviates too far from a
  smoothed TWAP. Drift's verified guardrails: `TooVolatile` outside oracle/TWAP
  ratio [1/5, 5] (`too_volatile_ratio: 5`, symmetric max/min check);
  `TooUncertain` when confidence > 2% of price
  (`confidence_interval_max_size: 20_000`, scaled by a per-operation
  multiplier); `NonPositive` on a non-positive price (verified against
  [drift-labs/protocol-v2](https://github.com/drift-labs/protocol-v2)
  `programs/drift/src/state/state.rs` and `programs/drift/src/math/oracle.rs`,
  master as of 2026-06-10).
- **Cross-validate Pyth against Switchboard; reject on divergence** -- never pick
  the favorable one. Pyth is a **pull-based oracle over a permissioned publisher
  set** with publisher-reported confidence; it is trust-*minimized*, not
  trustless, and we label it so.
- **Never credit unrealized PnL or self-referential/thin-market marks as
  redeemable equity.** *(Mango ~$110-117M: unrealized perp PnL on a thin
  self-pumped feed credited as borrowable collateral.)* Apply illiquid-asset
  haircuts and price-impact modeling. *(GMX V1 ~$565K: zero-slippage fills at
  oracle spot. Solend USDH ~$1.26M: single thin-pool feed.)*
- **Cap NAV fraction in any asset whose confidence exceeded threshold in the last
  K epochs**; treat sustained-wide-confidence assets as haircut-to-illiquid.

> **Numeric-threshold honesty:** specific bps figures (60s staleness, 5%
> confidence, 10% TWAP) are **not** official Pyth recommendations -- Pyth gives
> only qualitative guidance. Drift's 10/120-slot, [1/5,5]-ratio, and
> 2%-confidence numbers *are* verified, against the protocol-v2 source cited
> above. We own our thresholds as documented design decisions; we do not cite
> them as authority.

### 3b. Off-Solana legs (Hyperliquid L1 perps, Derive L2 options) -- the irreducible trust gap

**State of the art, honestly.** No production on-chain fund (Enzyme, Sommelier,
dHEDGE) values positions on a *foreign* chain in a trust-minimized way. Worse,
for these
venues specifically: Wormhole supports **HyperEVM**, not the Hyperliquid **L1
perp core** where positions live; Hyperliquid's default RPC `eth_call` "only
supports the latest block; historical state queries are not supported"
(per the [HyperEVM JSON-RPC docs](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/hyperevm/json-rpc),
accessed 2026-06-10; independent archive nodes exist -- ADR 0002 rail 2
records the reconciled read-path facts); Derive
(L2) verifiable-read availability is uncorroborated. **A trust-minimized read of
Hyperliquid L1 perp positions does not exist off-the-shelf today.**

**Attesting position quantity and multiplying by a Pyth price produces *wrong*
NAV, not merely trusted NAV:**
- **Hyperliquid perps:** the economically meaningful quantity is **account
  equity = margin + unrealized PnL - funding owed - liquidation buffer**, a
  nonlinear function of the venue's *internal mark*, funding accrual, and the
  maintenance-margin schedule. It cannot be reconstructed from
  `quantity * Pyth_price`, and the error direction is **manager-controllable** by
  choosing positions where venue-mark and Pyth diverge.
- **Derive options:** value is `f(spot, strike, IV, time, rate)`. There is no
  Pyth feed for a specific OTM strike, and re-pricing Black-Scholes on-chain with
  an untrusted IV surface is not trust-minimized. The "price from Pyth" story
  collapses here.

**Corrected attestation requirement.** Attest the **venue's own complete account
state** -- equity, margin, unrealized PnL, funding, **borrows/liabilities**, open
orders, per series for options -- atomically from an endpoint that returns them
together, and:
- **Pin a fund-controlled venue-side account identifier** into program state at
  venue-onboarding (governance action); every proof's TLS payload must contain
  that exact identifier or is rejected (defeats the "proof of a different, richer
  account" substitution).
- **Treat any un-proven dimension as worst-case** -- a missing liabilities
  endpoint => liabilities = infinity => that leg haircut to zero (defeats
  hidden-leverage a la Stream).
- **Sanity-bound the venue's mark against Pyth** (reject if venue-mark deviates
  > X% from Pyth -- fail closed) rather than substituting Pyth for the venue's
  valuation. For options, bound each series' venue mark by on-chain
  no-arbitrage (`price in [intrinsic, spot]`); do not re-price.
- **Bind every proof to (epoch, venue-account-id, monotonic nonce), consumed
  exactly once** (a favorable past proof replayed at a later epoch is a
  manipulated NAV with a valid signature). Proof accounts use `init` + `close`
  like withdrawal requests.
- **Require a fresh proof struck inside the settlement window**, not a pre-window
  snapshot, to shrink the irreducible TOCTOU gap (the backend can move/encumber
  off-Solana funds between proof time T and settlement T+delta). TOCTOU on a
  foreign chain is irreducible -- disclose it, bound it by cap.

**The optimistic dispute layer is removed entirely for off-Solana NAV.** It is a
**category error**: UMA-style optimism works for *public* facts (election
results); the contents of a private trading account only the manager can
authenticate to are **unfalsifiable by outsiders** -- an honest disputer cannot
produce a counter-proof any more than the proposer can produce truth. Layering it
in adds false assurance *and* a griefing-freeze surface (spurious disputes to
delay settlement = another redemption-freeze vector). A dispute layer may police
**only publicly verifiable inputs** (oracle prices, on-Solana positions), where
contradicting evidence can actually be produced.

**Notary committee economic-security model is specified, not hand-waved.**
"Multiple independent notaries" does **not** add Byzantine fault tolerance: all
notaries observe the *same TLS session to the same venue endpoint*; a compromised
venue, a MITM at the manager's egress, or manager-favorable data to that session
makes **all notaries attest the same lie in unison**. Committee independence
defends against one notary defecting, nothing against one corrupt upstream.
Therefore: the notary set must be staked/AVS-slashable with a corruption cost
specified and **sized so corruption-profit < corruption-cost given the per-venue
cap.** Absent that number, "proof-by-committee" is the same hand-wave we reject in
a manager-signed NAV.

**Recommended off-Solana NAV (corrected):**
1. **The read mechanism is tier-specific.** Tier 2 (Hyperliquid majors):
   HyperEVM precompile reads of the venue's own consensus state via Wormhole
   Queries, verified on Solana by the fund-owned guardian-signature verifier
   ([ADR 0002](../adrs/0002-tiered-off-solana-nav-inclusion.md) rails 1-2) --
   no zkTLS, no notary committee. Tier 3 (Derive, any CEX) **only**:
   off-Solana **full account state** (assets AND liabilities AND open orders)
   via zkTLS web-proofs from a staked/slashable committee, account-id-pinned,
   freshness-bound, replay-bound -- feeding reporting and the side-pocket,
   never the redeemable price.
2. Venue marks sanity-bounded against on-Solana oracles (divergence => fail
   closed); options bounded by no-arbitrage.
3. **Conservative, adverse-to-actor haircuts** on all off-Solana value.
   *(ADR 0002 revision: Tier 2 gets the per-tier `h` haircut behind the
   pessimistic floor, cap, and rails -- not a single flat haircut; Tier 3 stays
   fully ring-fenced. Read per the header banner's tiered model.)*
4. **Per-venue caps sized to total-loss-tolerable** -- the fund can eat the whole
   off-Solana tranche without harming on-Solana redeemers.
5. **Ring-fencing is Tier-3-only under
   [ADR 0002](../adrs/0002-tiered-off-solana-nav-inclusion.md).** Tier-2
   Hyperliquid value (consensus-read, all rails met) is NOT ring-fenced out: it
   enters `redeemable_nav` at a pessimistic floor behind the cap. Only Tier-3
   `attested_nav` (Derive, any CEX) is ring-fenced out of the redeemable price
   entirely (Principle 2) -- and for Tier 3 this, not the optimistic layer, is
   what actually bounds blast radius. Items 2-4 apply to Tier 2 in their
   ADR-0002 form (rails 3-6); item 1's zkTLS/committee leg is Tier-3-only --
   Tier 2's read is the precompile + Queries path, not a modified committee.
6. **Data-integrity (not magnitude) circuit breakers, fail-closed on bad data
   only.**

**Manager-signed NAV remains rejected outright.** *(Stream Finance ~$93M
self-reported NAV, no breakers, ~4.1x hidden leverage; the bridge trusted-signer
class -- Ronin, Harmony, Multichain.)*

### Defends against
Stream Finance (the exact architecture: on-chain shares backed by off-chain
self-reported NAV), Mango/GMX/Solend (thin/self-referential/no-slippage feeds),
the quantity-vs-equity mis-valuation class, hidden-leverage via un-proven
endpoints, proof replay, account substitution, and
notary-collusion-undetectable-on-chain.

### Trust assumptions (stated bluntly)
**On-Solana (Tier 1):** trust Pyth/Switchboard publisher sets and honest
on-chain position state (natively verifiable) -- genuinely trust-minimized.
**Off-Solana, Tier 2 (Hyperliquid, ADR 0002):** no notary committee -- trust
(a) the Wormhole guardian quorum read channel (honest transmission of a false
venue state is the dominant failure and the quorum does NOT defend it), (b)
HyperBFT economic truthfulness (governance trust), (c) aggregate venue
solvency, (d) the irreducible foreign-chain TOCTOU within the settlement
window -- bounded by the pessimistic floor, the six rails, and the
total-loss-tolerable cap, and labeled "guardian-quorum-read,
cross-chain-read-trusted, capped". **Off-Solana, Tier 3 (Derive, any CEX):**
trust (a) venue-API honesty (now *more* load-bearing because we attest
equity, the honest cost of correctness), (b) notary-set non-collusion sized
against the cap, (c) account-id pinning correctness, (d) the same TOCTOU. All
four are **disclosed to investors as "venue-API-trusted with bounded
exposure"**, and the ring-fence (not the caps alone) ensures the Tier-3
residuals cannot move the redeemable price.

### Tradeoffs
Attesting full equity makes venue-API-honesty more central -- but it is *honest*,
whereas quantity x Pyth is *wrong*. Haircuts understate true NAV when off-Solana
legs are healthy (performance drag, in the fund's favor). Per-venue caps and the
ring-fence constrain strategy concentration -- intentional. Under ADR 0002 the
project's **single largest engineering and trust risk is the Tier-2 read
path**: the fund-owned Wormhole Queries response verifier (response parsing,
guardian-signature verification, quorum, guardian-set rotation/expiry) and
the verification of HyperEVM precompile semantics (ADR 0002's
pre-implementation gate) -- surfaced to the user as such. zkTLS + staked
committee (now Tier-3 reporting only) stays immature and operationally heavy,
a second-order risk because it can no longer touch the redeemable price.

---

## 4. Manager / operator trust boundary

### Recommendation

**Decouple "who runs the strategy" from "who can move or value funds."** The
manager proposes/executes trades within hard on-chain constraints; cannot move
principal to arbitrary destinations, cannot set NAV, cannot drain. *(Enzyme
model: the manager moves assets within allowed positions; NAV follows
automatically from oracle-valued positions -- the manager never writes a NAV
number.)*

**On-chain, program-enforced, backend-uneditable constraints** (Enzyme policy
toolkit):
- **Venue/adapter allowlist** (`AllowedAdaptersPolicy`).
- **Asset allowlist** (`AllowedAdapterIncomingAssetsPolicy`).
- **No untracked external positions** (`AllowedExternalPositionTypesPolicy`) --
  blocks the Yearn iEarn "share price excludes strategy assets" and Stream
  "hidden leverage" classes.
- **Destination allowlist** -- principal moves only to allowlisted venue
  addresses / vault PDAs. *(BigONE/CoinDCX: perfect key custody does not save you
  when the backend deciding what to sign is compromised.)*
- **Latching cumulative drawdown breaker, not a refilling rolling window.** A
  time-decaying `CumulativeSlippageTolerance` is an **unbounded-over-time drain
  channel**: a bribed/self-dealing manager extracts the tolerance budget
  repeatedly as it decays-and-refills -- "a slow bounded leak running forever is
  a salary paid to the attacker." Replace with a breaker that **latches**: on
  hitting a standing drawdown ceiling it **halts and requires governance to
  reset.** Bound the integral (lifetime/quarterly), not just the derivative.
  *(Revision note, ADR 0002: on ratification the latch is denominated on the
  pessimistic-floored `redeemable_nav_redeem` -- Section 11 item 6 -- so a
  real Hyperliquid crash trips it and withholding cannot suppress it; it
  halts trading and upward recognition only, never redemptions: Principle 5's
  scope.)*
- **Counterparty / self-trade detection and per-trade execution-anomaly checks**
  where venues expose it: realized vs. expected execution price per trade, not
  only cumulative. Any rebalance whose counterparty is not a public AMM/order-book
  (possible self-dealing) routes through the same multisig + timelock as
  privileged changes.
- **NAV-deviation bound per action + total-shares constant across rebalances**
  (Sommelier H-1/H-2 fixes: `allowedRebalanceDeviation` + constant shares +
  `shareLockPeriod`).
- **Per-tx and per-window outflow caps** (ERC-7265-style breakers; Squads
  spending-limit semantics on-chain).

**On-chain, rules-based fee model -- fees are first-class, not an afterthought.**
A fund with no fee model is not a fund; a fund with an *off-chain* fee model
reintroduces a **manager-controlled mint path**, contradicting "the manager
cannot mint shares."
- **High-water mark enforced on-chain.**
- **Performance fees crystallize only at finalized (not smoothed) epoch
  `verifiable_nav`**, computed on *realized* growth net of any smoothing reversal
  -- so the manager cannot crystallize carry on smoothed-high NAV that then
  reverts.
- **Fee-share minting is subject to the same caps and anomaly breakers** as every
  other mint; HWM-gaming via deposit-timed dilution is blocked by HWM-on-chain.

**What the manager CAN do:** open/close/adjust positions on allowlisted venues
with allowlisted assets, within deviation/loss/cap limits, via Turnkey-signed
CPIs scoped to those trade shapes.

**What the manager CANNOT do:** set the redeemable NAV, or **raise** Tier-2
NAV by signing a mark (no manager signature is ever a NAV input); move funds
to non-allowlisted destinations; exceed outflow caps or the latching drawdown
ceiling; create untracked positions; mint/burn fund shares outside the
rules-based fee model; change allowlists/caps/oracles/haircuts/code (governance +
timelock, Section 5); halt or override the rules-based gate (no manager
kill-switch); induce a freeze of the **on-Solana tranche** by withholding
proofs (Section 6). *(Revision note, ADR 0002: withholding Tier-2 reads
remains a real manager lever -- Wormhole Queries is gated, so the manager
backend sits in the submission path -- but it is a liveness and
downward-recognition lever only: a withheld read floors the leg and
quarantines its deposits per rail 1 and the stale-read pricing rules; it can
never raise credited value.)*

**Manager-independent redemption path** (Friktion lesson): if the
manager/backend goes dark or rogue, **on-Solana-tranche redemption executes via
the program against the vault PDA with no backend signature required** -- so
vendor liveness (Turnkey down) and proof-withholding cannot strand investors.
Gating is in-protocol and rules-based, never a manager kill-switch or validator
oracle override. *(The JELLY "fix" -- centralized oracle override + selective
freeze -- is the anti-pattern to avoid.)*

### Defends against
Stream (self-reported NAV + hidden leverage + no constraints), BigONE/CoinDCX
(backend compromise), Sommelier H-1/H-2/H-3 (rebalance sandwich/frontrun/
oracle-latency), the slow-drain-via-allowlisted-venue and refilling-window
classes, the off-chain-fee manager-mint hole, and Friktion (manager-independent
exit).

### Trust assumptions
Allowlists/caps/haircuts set correctly at deployment, changed only via governance
multisig + timelock with the asymmetric ratchet (Section 5). The oracle layer
(Section 3) is sound -- deviation bounds and the drawdown breaker are denominated
in `verifiable_nav` *(revision note, ADR 0002: on ratification the drawdown
breaker is denominated on the pessimistic-floored `redeemable_nav_redeem`,
per ADR 0002 pricing mechanics and Section 11 item 6, so it trips on a real
Hyperliquid crash and cannot be suppressed by withholding; deviation bounds
follow the same base)*, so Section 3's fail-closed data-integrity checks are
load-bearing here too.

### Tradeoffs
Hard caps/allowlists/latching breaker constrain agility -- adding a venue or
resetting a tripped breaker is a timelocked governance action, not a backend
deploy. Intentional. A latching breaker can halt trading on a genuine
bad-but-legitimate move and require a governance reset -- acceptable; the
alternative is the attacker's salary.

---

## 5. Custody & key management (Turnkey-TEE execution backend)

### Recommendation

**Four separated authority domains** (attestation is split out from execution):

1. **Execution authority (Turnkey TEE)** -- signs only trade CPIs matching
   allowlisted shapes. Turnkey: AWS Nitro Enclaves, in-enclave Policy Engine,
   quorum-share key reconstruction in-enclave, remote attestation linking source
   to live binary (QuorumOS/StageX reproducible builds), no known breach. Signing
   policies scoped to specific tx shapes mirror the on-chain allowlists (defense
   in depth). **The enclave policy is assumed compromisable together with the
   backend** (policies are *configured*; a Turnkey-org-admin phish degrades the
   in-enclave wall to whatever the attacker sets). We therefore do **not**
   double-count it as independent -- the on-chain program must suffice alone
   (Principle 6, Section 4's latching breaker).

2. **Attestation-submission authority (separate domain)** -- the key that submits
   off-Solana proofs is *not* the execution key. A compromised execution backend
   cannot move principal to a bad address, but if shared it could feed a fake
   high NAV through the pipeline. Splitting the domain, plus the ring-fence
   (attested value never moves the redeemable price), closes the valuation axis.
   > **Revision note (ADR 0002).** Under the tiered model this statement holds
   > only for Tier-3 `attested_nav`. The attestation-submission key is also one
   > of rail 1b's >=2 Tier-2 read submitters, and Tier-2 reads DO move
   > `redeemable_nav_redeem` -- so the domain split *narrows* the valuation
   > axis rather than closing it. The residual is bounded by the on-chain
   > guardian-quorum verification of the read, the lower-of-two-submissions
   > rule, the pessimistic floor and haircuts, and rail 6's cumulative-outflow
   > latch against the total-loss-tolerable ceiling.

3. **Program upgrade authority (Squads V4 multisig + timelock) -- NOT burned.**
   The Solana upgrade authority has *arbitrary control over deposited funds*
   (Neodyme), so it MUST be a Squads V4 multisig (>=3-of-5, **independent humans
   on independent hardware**) behind a timelock (Squads V4: timelocks, spending
   limits, roles; secures >$10B across Raydium/Marginfi/Kamino/Jupiter; formally
   verified). For a fund that reads *foreign venue program layouts* and may need
   to disable an exploited NAV leg, burning is a **stuck-funds generator**, not a
   guarantee -- when Drift/Kamino/Jupiter upgrade and change layouts, or a venue
   is exploited, an immutable program misreads NAV or cannot safely redeem.
   Correct trust-minimization here is **constrained, transparent, timelocked
   upgradeability with a guaranteed exit**, kept permanently. (Compound's 48h
   Governor Bravo timelock is the floor; longer for a fund.)

4. **Privileged-parameter authority (separate Squads multisig + timelock)** --
   allowlists, caps, oracle config, **haircuts**, venue onboarding (account-id
   pinning). The privileged multisig is the **highest-value target in the
   system**: it can add a manager-controlled destination, add a manager venue
   adapter, lower a haircut to recognize fake gains, or point the oracle at a
   thin feed. Mitigations:
   - **Asymmetric ratchet:** caps tighten / haircuts raise **instantly**; caps
     loosen / haircuts lower / destinations add / oracle swaps only after a
     **long timelock** -- long enough for investors to fully exit against
     `verifiable_nav` before any loosening takes effect. *(Drift ~$285M, Apr
     2026 -- the largest 2026 DeFi hack and the costliest Solana incident after
     Wormhole. Not a contract bug: North-Korea-linked UNC4736/AppleJeus -- the
     same actor as Radiant -- social-engineered a six-month vault onboarding,
     then used privileged access to whitelist a worthless token (CVT) at a fake
     price as collateral and drained $285M in real assets. No timelock stood
     between the privileged allowlist change and the withdrawal -- whitelisting
     a collateral/venue and pointing at its price is exactly a "loosen" action
     this ratchet gates. Multisig governance did not save them; process and
     timelocks would have.)*
   - **3-of-5 is weaker on the dimension that actually fails.** Radiant fell at
     3-of-11 to blind-signing, so raw N is not the lever -- **signer independence
     and out-of-band verification are.** Publish a real signer-independence
     attestation; "Squads secures $10B" is an appeal to authority, not a property
     of *our* signer set.

**Out-of-band hash verification for every privileged human action.** *(Bybit
~$1.46B, Radiant ~$50-58M at 3-of-11 -- a low threshold, not "11 signers failed"
-- WazirX ~$235M: multisig + simulation + hardware wallets all failed against
blind-signing of a forged display.)* Signers verify the tx hash out-of-band; the
program **re-derives intent and validates structural invariants** rather than
trusting backend-supplied calldata; no temporary delegation shortcuts *(Ronin's
never-revoked Axie DAO allowlist was the 5th signature)*.

**Attested-enclave code pinning.** *(BitForge/CVE-2023-33241 extracted full keys
from GG18/GG20 MPC-TSS via a missing Paillier-modulus ZK proof.)* Pin and verify
Turnkey's attested measurements; treat the TEE as one layer, not the perimeter.

**Vendor-liveness is a stuck-funds vector.** Turnkey is one company; if it is
down, the backend cannot sign, so trading and buffer-refill stall. The
on-Solana-tranche redemption path must execute **without any backend or Turnkey
signature** (Section 4), so single-vendor downtime cannot strand the verifiable
tranche.

### Defends against
Neodyme (upgrade-authority backdoor -- now timelocked, not burned),
Drift (privileged collateral/oracle whitelisting with no timelock),
Multichain/BigONE/CoinDCX (operator/backend compromise), Bybit/Radiant/WazirX
(blind-signing forged displays), Ronin/Harmony (threshold + independence + stale
delegations), BitForge (TEE not self-justifying), and the
valuation-axis-rides-the-same-backend hole (attestation split + Tier-3
ring-fence; for Tier-2, on-chain guardian-quorum read verification + the
total-loss-tolerable cap and outflow latch).

### Trust assumptions
AWS Nitro isolation and QuorumOS attestation hold (residual -- the program is the
backstop, not the enclave). Squads signers are independent and hardware
uncompromised at signing (out-of-band verification mitigates display compromise).
Because upgrade authority is retained, the timelock window + always-available
on-Solana redemption are the user protection against a malicious upgrade.

### Tradeoffs
Retaining upgrade authority keeps a residual backdoor (mitigated by multisig +
timelock + guaranteed exit) -- the deliberate choice, because immutability for a
multi-venue foreign-layout-dependent fund is more dangerous than
patchable-but-timelocked. Timelocks slow legitimate emergency fixes; the
in-protocol data-integrity breakers (not a kill-switch) cover the fast path. Four
domains are operationally heavier -- non-negotiable given the corpus.

---

## 6. Withdrawals & liquidity under deployed capital

### Recommendation

**No atomic at-NAV redemption while capital is deployed** *(Harvest $24M, Yearn
yDAI $11.1M flash-loan honeypot; Maple FCFS first-mover race)*. A **request ->
epoch-finalized claim** state machine (ERC-7540 Pending->Claimable->Claimed as a
*reference*, re-derived for Solana).

**Two-tranche redemption is the core liveness fix:**
- **On-Solana tranche -- always redeemable.** Priced from on-chain state
  (`verifiable_nav`), capped per epoch, **never frozen by off-Solana proof
  availability.** A proof outage, censored notary, or tripped data-integrity
  breaker halts only the off-Solana slice. "Trapped forever" is structurally
  impossible.
- **Off-Solana tranche -- redeems only when fresh, account-id-pinned,
  replay-bound proofs exist.** *(Revision note, ADR 0002: the timeout rule is
  the stale-read rule, not "last verified minus a haircut" -- a last-verified
  base would let a withheld read preserve a stale high mark.)* On staleness
  the Tier-2 leg **floors to its conservative lower bound immediately**
  (rail 1a: fail-safe down, fail-closed up); redemptions settle against that
  floor, pro-rata against whatever liquid assets exist; deposits into the leg
  are rejected or priced on the adverse high side (stale-read deposit rule);
  Tier-3 value stays side-pocketed, non-redeemable until realized. Late
  redeemers may be worse off but are **never trapped**. A passive
  freeze-by-proof-withholding is removed.

**State machine:**
1. **Request:** shares burned at request (or escrowed in a request PDA);
   recorded in a queue keyed to the next NAV epoch. Deposits symmetric.
2. **Settle:** at epoch lock, `verifiable_nav` is checkpointed (becomes the
   `redeemable_nav_deposit` / `redeemable_nav_redeem` pair per ADR 0002 on
   ratification); pricing is the adverse-to-actor
   `max`/`min`-over-conversion rule (Section 2). The manager has the
   inter-phase interval to unwind and refill the USDC buffer.
3. **Claim:** USDC released when the buffer covers the batch *(Lido:
   AccountingOracle finalizes daily batches; buffer prioritizes withdrawals)*.

**Per-epoch outflow caps + pro-rata loss socialization** *(Maple ~$36M, ~80% LP
loss because FCFS cash race with no socialization)*. Each epoch finalizes at most
a capped fraction of NAV; shortfalls fill pro-rata within the epoch so no fast
redeemer escapes a loss. Partial fills **re-price at each epoch they roll into**
-- never carry a stale price across epochs (which would hand the fund's redeemers
a free option).

**FIFO is removed in favor of pure pro-rata.** FIFO-across-epochs +
pro-rata-within-epoch makes the epoch boundary a gameable seam (race into the
current batch before bad news; the run just relocates to "earliest epoch").
*(Revision note, ADR 0002:)* the redeemable price includes
pessimistically-floored Tier-2 value, which can only move **down** before a
batch settles (stale reads floor immediately, fail-closed up), so there is no
inflated price to race toward; upward re-recognition lands only after the
batch clears, and the slow-up rule keeps recognition lag from becoming a
deposit-side capture. We additionally drop FIFO to eliminate the boundary
race; consider randomized intra-epoch ordering.

**Disputed-down / data-integrity-conservative pricing:** there is no optimistic
dispute layer for off-Solana NAV (Section 3), so the dispute-window-straddle
vector is removed at the root. Where any input enters a *data-integrity* halt,
that leg settles at its conservative lower bound; the queue does not let actors
condition on a pending correction.

**Solana-native re-derivation:**
- Queue entry = a PDA keyed to (investor, target epoch); no ERC-721, no
  `msg.sender` controller model, no single-block atomicity.
- **Build our own protocol-triggered timeout-settlement path** -- ERC-7540
  deliberately does not standardize cancellation (funds can be "stuck in
  Pending"). This is **not investor-cancellable**: it fires automatically after
  a defined staleness window and force-settles in place; it never returns shares
  to the investor or allows re-entry at a later price (which would re-open the
  inter-epoch free option Section 2 disallows after commit). The timeout's
  payout is **fully specified**: on-Solana tranche refunds at `verifiable_nav`;
  off-Solana tranche force-settles at the leg's conservative lower bound per ADR
  0002's stale-read rule (the floored Tier-2 value -- never
  last-verified-minus-haircut, which preserves a stale high mark), pro-rata. No
  "refund of what, at what price?" ambiguity.
- Request/proof PDAs use `#[account(init)]` (NOT `init_if_needed`) and
  `#[account(close = destination)]` on claim (atomic zero + closed
  discriminator) -- blocks revival/double-claim/replay.
- **Minimum request size + per-account requests-per-epoch cap + paginated,
  resumable settlement cursor**: a griefer cannot bloat the batch past Solana's
  compute limits to brick settlement (another freeze vector).
- **Settlement atomically bundles the oracle update it validates**: no foreign tx
  can interpose between the update and the price it strikes; the consumed update
  must be written in the same instruction.

**Data-integrity vs. market-move breaker distinction:** a breaker trips on *bad
data* (oracle divergence, proof staleness, confidence blowout) -> freeze that
leg; it does **not** trip on *bad market* (a real crash) -> the on-Solana tranche
stays liquid so investors can exit at the real, bad price. Conflating the two
turns every crash into a freeze.

**Wind-down semantics:** a governance-gated full-settlement mode where, once all
positions are USDC and NAV is fully on-Solana-verifiable, the final batch is
priced with the same round-down rules as every other operation (ADR 0001's
round-adverse-to-actor invariant is unconditional), and the **residual dust
that round-down strands in the vault is then distributed explicitly** to the
final batch's redeemers alongside the dead-share residual -- so the last
redeemers are not stranded by compounding round-toward-vault + haircuts, and
no rounding-direction exception is introduced.

**The gate is rules-based and in-protocol, never a manager kill-switch.** Halts
are objective on-chain conditions (data-integrity signals, buffer shortfall), not
manager discretion, and the manager cannot induce a freeze of the on-Solana
tranche.

### Defends against
Harvest/Yearn yDAI (atomic at-NAV honeypot), Maple (FCFS no-socialization), Iron
Finance/TITAN ~$2B (lagging-TWAP first-mover + mid-run revert lockout), stETH
de-peg (on-demand claim against non-instant capital), the
whole-fund-freeze-by-proof-withholding class, the epoch-boundary race,
settlement-bricking DOS, settlement-tx MEV, and the crash-becomes-freeze
anti-pattern. Lido is the settlement-engine reference; GMX/Hyperliquid the
position-cap reference.

### Trust assumptions
The epoch `verifiable_nav` checkpoint is sound (Section 3). The manager unwinds
enough between phases to refill the buffer; if not, caps + pro-rata + the
two-tranche protocol-triggered timeout-settlement bound the damage and prevent
silent lockout. The
off-Solana tranche's redeemers accept that proof unavailability degrades their
exit to settlement at the leg's conservative lower bound (ADR 0002 stale-read
rule) -- disclosed.

### Tradeoffs
Investors cannot exit instantly (honest and unavoidable; instant at-NAV is the
exploit). Per-epoch caps mean a large redemption spans epochs. Pro-rata means
timing affects outcome (fairer than FCFS; `max`/`min` forward pricing minimizes
gameability). The off-Solana tranche is genuinely less liquid and less safe than
the on-Solana tranche -- which is the point: investors see exactly which slice of
their claim is verifiable.

---

## 7. Solana / Anchor program hardening

Each item maps to the incident where its omission was the bug.

**Account validation (every account, every instruction):**
- [ ] `Account<'info, T>` for all program accounts -- verifies program ownership
      AND the 8-byte discriminator. Never bare `AccountInfo` where typed access
      is possible.
- [ ] Bind every account to a root of trust via `seeds`/`bump` (canonical),
      `has_one`, or explicit `address =`. *(Cashio ~$48M: chained fake accounts
      compared only to each other -- no root of trust. Crema ~$8.8M: real tick
      address written into a fake account to bypass an owner check.)*
- [ ] Validate `mint`, `owner`, `address` on every token/oracle/vault account a
      verification step reads. *(Cashio = unvalidated `mint`.)*
- [ ] Verify sysvar account identity explicitly; never deprecated unchecked
      variants. *(Wormhole ~$326M: `load_instruction_at` didn't verify the
      Instructions sysvar address -> spoofed sysvar bypassed signature
      verification.)*
- [ ] Signer checks on every authority. *(Jet ~$20M at risk, patched: unbound
      `deposit_note_account` + shared `market_authority` PDA as burn authority.)*
- [ ] Duplicate-mutable-account checks where logic assumes distinctness.
- [ ] **No arbitrary CPI** -- pin program IDs of all CPI targets to the venue
      allowlist.

**Venue (foreign-program) dependency hardening:**
- [ ] Pin venue **program IDs and expected program-data hash** where possible. A
      venue program upgrade can change account layout so the fund's `Account<T>`
      reader misreads a wrong-but-parseable value -> wrong NAV via a dependency
      the fund doesn't control.
- [ ] Treat any venue upgrade as a **governance-gated event** that re-validates
      layout before re-enabling NAV reads from that venue.
- [ ] Validate **every venue account's owner-program AND authority binding on
      every read**, not just program-ID pinning -- the backend supplies venue
      accounts; without owner+authority validation it can feed look-alike
      accounts (Crema/Cashio at the CPI boundary).
- [ ] Size each venue's per-venue cap to its admin-key risk (a venue with a hot
      upgrade key -> smaller cap).

**State & accounting invariants:**
- [ ] Never compute shares/NAV from the vault PDA's raw SPL balance (Cream).
- [ ] **Assert `verifiable_nav` = sum of on-Solana legs on every read**;
      `attested_nav` tracked separately and never folded into the redeemable
      price. *(Cypher ~$1M: isolated-subaccount state not propagated to master +
      broken margin check -- an invariant-desync bug.)*
- [ ] No untracked positions (`AllowedExternalPositionTypesPolicy` analog).
- [ ] **Deposit-side false-read bound before Tier-2 deposit inclusion**
      (Section 11 item 17 / ADR 0002 open decision 9 -- **blocking**). Rail 6's
      latch bounds redemption outflow only; a persistent clean false-high read
      over-prices every deposit on an unbounded channel. Implement the ratified
      deposit-side mechanism (a second cumulative latch over the Tier-2 deposit
      premium, or exclusion of unreconciled Tier-2 value from the deposit price)
      before enabling Tier-2 in `redeemable_nav_deposit`; until then deposits
      price Tier-1-only. Ratify the exact mechanism (item 17) before writing the
      failing-test contract.

**Arithmetic:**
- [ ] `overflow-checks = true` in `Cargo.toml` (release builds wrap silently
      otherwise).
- [ ] Checked arithmetic + u128/u256 intermediates; single fused mul-div, round
      toward the vault (Kamino).
- [ ] No panicking index ops (`vec[i]` -> `.get(i)`); zero
      `unwrap`/`expect`/`panic!`/`unreachable!` in non-test code.

**Lifecycle / revival / replay:**
- [ ] `#[account(init)]`, NOT `init_if_needed`, for request/withdrawal/**proof**
      PDAs (blocks revival/double-withdraw/**proof-replay**).
- [ ] `#[account(close = destination)]` on claim (atomic zero + closed
      discriminator).
- [ ] **Bind every off-Solana proof to (epoch, venue-account-id, monotonic
      nonce), consumed once;** reject any proof whose embedded epoch != the
      settling epoch.
- [ ] "Unproven => rejected" everywhere -- no default/zero value satisfies a
      NAV-fresh, withdrawal-authorized, or proof-valid check. *(Nomad ~$190M:
      default `0x00` as proven. Qubit ~$80M: minted on an event without
      confirming escrow.)*

**Settlement robustness:**
- [ ] Minimum request size + per-account per-epoch request cap + **paginated,
      resumable settlement cursor** so batch size can't exceed compute limits and
      brick settlement.
- [ ] Settlement **atomically bundles the oracle update it validates** (consume
      the update written in the same instruction) so no foreign tx interposes
      between update and price.

**Cross-chain proof verification (if any off-Solana proof is verified on-chain):**
- [ ] The verifier MUST enforce ALL structural/well-formedness invariants. *(BNB
      Chain ~$570M minted: under-constrained IAVL range-proof verifier didn't
      enforce that a path node can't have both children.)*
- [ ] The Wormhole Queries response verifier (ADR 0002 Tier-2 read) is
      **fund-owned program logic** -- parse the response format, verify guardian
      signatures against the core-bridge-owned guardian set, enforce quorum,
      handle guardian-set rotation/expiry. Queries responses are not VAAs; the
      core bridge has no instruction that verifies them, so this verifier is
      fully in scope for every item in this checklist. (CCQ protocol facts --
      response format, signing prefix, no core-bridge verify instruction --
      are cited with access dates in ADR 0002's Context; the exact
      signature-quorum rule is unverified-pending-fixture and must be pinned
      with a recorded QueryResponse in the contract PR.)
- [ ] Never credit value on a claimed event without confirming the underlying
      value exists and is escrowed (Qubit).

**Operational:**
- [ ] **Data-integrity** circuit breakers (halt the affected leg on oracle
      divergence / proof staleness / confidence blowout) -- **not** NAV-magnitude
      breakers.
- [ ] Per-tx and per-window outflow caps + the **latching** drawdown breaker in
      immutable program logic (BigONE/CoinDCX: backend can't be the enforcer;
      rolling window is an unbounded-over-time drain).
- [ ] Don't leak the patch before deploying. *(Wormhole's fix hit public GitHub
      ~9h before the hack.)*

---

## 8. Cross-chain capital movement

### Recommendation

**Treat every bridge as a trusted-signer or verification-logic attack surface,
and bound the blast radius on-chain.** *(2022: ~$2B across 13 bridge hacks, ~69%
of all funds stolen that year.)* Two failure classes, both defended:

- **Trusted-signer/MPC-quorum collapse** *(Ronin 5/9 with 4 keys one entity
  ~$625M; Harmony 2/5 ~$100M; Multichain MPC-under-one-operator ~$126M)*. Never
  route fund principal through a bridge whose security is a small multisig the
  manager or one operator controls. Prefer light-client / ZK-succinct-proof
  bridges (zkBridge / Polyhedra) where available. Where only committee bridges
  exist, require 2/3+ supermajority AND an on-chain challenge window, and accept
  this is trust-minimized, not trustless.
- **Verification-logic forgery** *(Wormhole unchecked sysvar; Nomad
  default-as-proven; BNB under-constrained verifier; Qubit
  mint-on-event-without-escrow)*. The on-chain receiver validates
  owner/program-id/address/`has_one` on every account a verification step reads,
  enforces "unproven => rejected," and (if verifying proofs) enforces all
  structural invariants. Never mint shares or credit a position on a claimed
  event without confirming escrow.

**Light clients are trust-MINIMIZED, not trustless.** Cosmos ICS-07/Tendermint:
if >2/3 of a source chain's validator power is malicious it can forge commitment
roots the light client can't detect -- safety inherits the *weaker* chain's
consensus. Any cross-chain capital movement or NAV evidence inherits the weakest
venue's consensus security and must be paired with caps + challenge windows.

**Honest acknowledgment:** no trust-minimized bridge exists for Hyperliquid L1 /
Derive today, so **the actual capital path runs over a trusted-signer bridge --
the Ronin/Multichain class this section condemns.** We therefore:
- **Size the per-venue cap to "we can lose this entire amount to a bridge hack"**
  (Principle 9). The cap is the real defense; caps + challenge windows on a
  committee bridge are the same speed-bumps that didn't save Ronin (6 days
  undetected), so the cap must assume the bridge fails.
- **In-transit bridge risk is split from settled-venue risk
  ([ADR 0002](../adrs/0002-tiered-off-solana-nav-inclusion.md)).** Bridged
  capital never counts toward `verifiable_nav`: in transit it is excluded from
  the redeemable price entirely **and** from the side-pocket claim token -- it
  is the fund's own moving principal, not a Tier-3 venue position, so it is
  tracked as a distinct non-redeemable, non-claim-bearing in-transit state, not
  folded into `attested_nav` (which accrues the Tier-3 claim). Once settled on the
  fund's inclusion-eligible Hyperliquid account, it may enter `redeemable_nav`
  only as floored, capped Tier-2 value under all ADR-0002 rails -- a consensus
  read, not a bridge attestation. Tier-3 value (Derive, any CEX) stays excluded
  until provably repatriated to Solana or realized: "provably back before it is
  redeemable" continues to hold for Tier 3.

**Operational controls that demonstrably bounded losses:**
- **Pausable + per-window outflow caps on the bridge legs.** *(BNB froze ~$430M
  of ~$570M by halting; bridges without a halt bled for days -- Ronin ~6 days,
  Nomad's crowdsourced drain.)* Our two-phase withdrawal window is the natural
  home for a settlement/timelock window, per-window caps, data-integrity anomaly
  detection, and a pausable rules-based circuit breaker.
- **Constrained destinations.** Bridged principal lands only at allowlisted vault
  PDAs / venue addresses (Section 4), never arbitrary addresses
  (Multichain/BigONE/CoinDCX).
- **No mint-on-event.** Credit fund value only after confirming escrow on both
  sides *(Qubit's `address(0)` + no-`SafeERC20`-revert minted unbacked qXETH)*.

### Defends against
Bridges are the #1 historical drain vector and the exact mechanism by which
manager trust gets silently relocated. Light-client/ZK bridges + 2/3+ thresholds
+ challenge windows + on-chain caps + destination allowlists are the verified
defenses; the ring-fence (in-transit excluded from both the redeemable price and
the claim token) prevents bridge risk from inflating the redeemable price.

### Trust assumptions
Where a light-client/ZK bridge exists, its verifier is correctly constrained
(under-constrained = the bug) and source-chain consensus is honest above >2/3
(inherited, irreducible). For Hyperliquid/Derive, the available committee bridge
is assumed *capable of total failure*, and the per-venue cap is sized to absorb
that loss -- disclosed.

### Tradeoffs
Cross-chain deployment to these venues inherits the worst bridge's security; the
cap and the ring-fence do the heavy lifting. Per-window caps + challenge windows
slow capital deployment -- intentional; the alternative is an unbounded
cross-chain drain.

---

## 9. The honest verdict on trust-minimized off-chain NAV

**Fully trust-minimized valuation of Hyperliquid L1 perps and Derive L2 options
does not exist with shipping technology today.** Any claim otherwise is
relocating manager trust into a backend/notary attestor -- the Stream Finance /
Ronin / Multichain failure mode. The achievable target is **trust-MINIMIZED with
explicitly bounded, disclosed residual trust, plus a structural ring-fence so the
residual cannot move the redeemable price.**

What each viable option actually achieves:

| Option | What it proves | Irreducible residual trust | Verdict |
|---|---|---|---|
| **A. zkTLS + staked notary committee, full account state (equity+liabilities), account-id-pinned, freshness/replay-bound** | The venue's authenticated API returned this *complete* account state at time T for *this* account | (1) **venue-API honesty** -- proves the venue *said* it, not that it is economically true; (2) **notary-set non-collusion sized against the cap** -- all notaries see one upstream session, so committee independence != Byzantine fault tolerance of the *data source*; (3) **foreign-chain TOCTOU** within the settlement window | **Recommended for off-Solana quantities/equity, ring-fenced out of redeemable price.** The honest label is "venue-API-trusted, bounded exposure." |
| **B. Optimistic bonded dispute (UMA-style)** | A posted NAV that survives a challenge window | **Category error for private accounts:** outsiders cannot produce a counter-proof of a privately-authenticated account, so disputes are unfalsifiable; CoC>PfC also fails once NAV-at-stake exceeds bond/token cap (UMA Polymarket: ~$237M secured by a ~$95M-cap token); adds a griefing-freeze surface | **Rejected for off-Solana NAV.** Permitted only to police *publicly verifiable* inputs (oracle prices, on-Solana positions). |
| **C. Native verifiable read (consensus-backed venue state proof)** | The venue's *consensus* attests the state | Source-chain consensus honesty (inherited) | **The only path to genuinely trust-minimized off-Solana NAV** -- but does not exist for HyperCore / Derive today. The frontier to migrate to when it ships. |
| **D. Manager-signed NAV** | Nothing -- the manager's word | Total manager trust | **Rejected outright.** This is Stream Finance. |

**Bottom line.** With Option A + ring-fencing, an investor can always redeem
against `verifiable_nav` (genuinely trust-minimized: Pyth/Switchboard + native
on-chain reads), and sees the off-Solana slice clearly marked as
venue-API-trusted with a capped, total-loss-tolerable exposure that **cannot move
the price they redeem at.** That structural separation -- not a "blast-radius
cap" on a global share price -- is the real difference between this fund and
Stream Finance. The off-Solana tranche is honestly riskier; the on-Solana tranche
is honestly safe; and no off-Solana failure can silently inflate the price at
which the safe tranche redeems.

---

## 10. Anti-pattern checklist (never do these)

- **Never compute share price from a vault `balanceOf`.** *(Cream ~$130M.)*
- **Never fold `attested_nav` into the redeemable share price.** *(Stream ~$93M;
  Hyperliquid JELLY pooled accounting -- the master anti-pattern.)*
- **Never value an off-Solana perp as `quantity * oracle_price`.** Value the
  venue's own equity/margin/PnL/funding/liabilities. *(Mango ~$110-117M.)*
- **Never accept a balance proof without an atomic
  liabilities/borrows/open-orders proof for the same account.** *(Stream's hidden
  4.1x leverage lived in un-proven endpoints.)*
- **Never let an off-Solana proof move NAV without pinning a fund-controlled
  venue-account-id and a per-epoch nonce.** *(Account substitution; proof
  replay.)*
- **Never freeze the whole fund on an off-Solana proof outage.** The on-Solana
  tranche must stay redeemable. *(The JELLY-class selective freeze, reachable
  here via proof-withholding -- JELLY itself was an active governance
  override.)*
- **Never trip a redemption freeze on a NAV-magnitude move (a real crash).**
  Freeze only on data-integrity signals.
- **Never use a refilling rolling-window loss tolerance as the drain backstop.**
  Use a latching drawdown breaker. *(BigONE/CoinDCX slow drain.)*
- **Never run an off-chain fee model** -- it is a manager-controlled mint path.
  Enforce HWM and crystallization on-chain at finalized NAV.
- **Never burn upgrade authority for a multi-venue, foreign-layout-dependent
  fund.** Keep it timelocked with a guaranteed exit.
- **Never credit a single oracle reading you can choose within the staleness
  window.** Require the freshest update, bundled in the settlement instruction;
  cross-validate Pyth vs Switchboard, reject on divergence.
- **Never claim "the backend cannot drain."** It can, slowly, through allowlisted
  adversarial trades up to the latching ceiling. *(BigONE/CoinDCX.)*
- **Never use `init_if_needed` for request/withdrawal/proof PDAs.** Use `init` +
  `close`.
- **Never treat a default/zero/uninitialized value as proven.** *(Nomad ~$190M;
  Qubit ~$80M.)*
- **Never use `load_instruction_at`-style unchecked sysvar access.** *(Wormhole
  ~$326M.)*
- **Never compare accounts only to each other with no root of trust; never trust
  backend-supplied venue accounts without owner+authority validation.** *(Cashio
  ~$48M; Crema ~$8.8M.)*
- **Never ship an under-constrained proof verifier.** *(BNB ~$570M.)*
- **Never mint/credit on an emitted event without confirming escrow on both
  sides.** *(Qubit ~$80M.)*
- **Never rely on a manager kill-switch or validator oracle override for
  gating.** Rules-based, in-protocol only. *(Hyperliquid JELLY "fix.")*
- **Never route fund principal through a small-multisig/single-operator bridge
  without sizing the per-venue cap to total bridge-loss.** *(Ronin ~$625M;
  Multichain ~$126M; Harmony ~$100M.)*
- **Never blind-sign a privileged tx from the wallet display alone.** Out-of-band
  hash verification + program re-derives intent. *(Bybit ~$1.46B; Radiant
  ~$50-58M at 3-of-11; WazirX ~$235M.)*
- **Never leave a temporary signer delegation un-revoked.** *(Ronin's Axie DAO
  allowlist = the 5th signature.)*
- **Never count bridged-in-transit capital toward redeemable NAV.** It is
  excluded from the redeemable price and the side-pocket claim token alike --
  the fund's own moving principal, tracked as a distinct non-redeemable
  in-transit state, not `attested_nav` -- until provably landed and ours.
- **Never let the seed/dead-shares be releasable by routine action.** Lock behind
  the longest timelock; assert non-redeemability on every redemption.
- **Never publish a security patch before deploying it.** *(Wormhole's fix leaked
  ~9h early.)*

---

## 11. Decisions requiring a human call

Each has a real tradeoff and material consequence for investor money. These are
surfaced for review; nothing is implemented until they are settled. Items 3 and 4
are now **decided** (marked inline below); the rest remain open.

1. **Off-Solana exposure ceiling (the biggest one).** Given off-Solana NAV is
   venue-API-trusted (Option A) and ring-fenced out of the redeemable price, how
   much of the fund may sit in the off-Solana (non-redeemable-until-proven)
   tranche? Recommendation: size it to total-loss-tolerable -- the on-Solana
   tranche must be unharmed if the entire off-Solana tranche is lost.

2. **Notary committee economic-security model.** Staked? AVS-slashed? What
   corruption cost vs. the per-venue cap? Recommendation: require a
   staked/slashable set with corruption-cost > corruption-profit at the chosen
   cap.

3. **Two-tranche share structure** -- **DECIDED: single fungible class with a
   non-redeemable accounting partition.** The on-Solana redeemable claim is one
   fungible class; `attested_nav` is tracked as a clearly disclosed,
   separately-priced partition that is non-redeemable until proven. Separate
   share classes were rejected as unnecessary structural complexity for the same
   ring-fencing guarantee.

4. **Upgrade authority** -- **DECIDED: retain a timelocked Squads multisig
   permanently (no eventual burn).** Kept behind a >=3-of-5 independent-signer
   Squads multisig + long timelock. Open sub-decisions remain: the timelock
   length (Compound's 48h is the floor; longer for a fund) and the
   signer-independence attestation to publish.

5. **Asymmetric-ratchet timelock lengths.** How long must the loosen-side
   timelock be (add destination / lower haircut / widen cap / swap oracle)?
   Recommendation: long enough for investors to fully exit the on-Solana tranche
   before any loosening takes effect.

6. **Latching drawdown ceiling + reset policy.** What standing drawdown halts
   trading, and what governance action resets it? Recommendation: a hard
   lifetime/quarterly integral ceiling, reset only via the privileged multisig +
   timelock.

7. **Epoch length / withdrawal cadence and per-epoch outflow cap.** Shorter +
   higher = better UX, less unwind time, more run-risk; longer + lower = safer,
   worse liquidity. Ribbon weekly / Lido daily are references.

8. **Off-Solana proof-staleness escape-hatch parameters.** After how many missed
   epochs does the off-Solana tranche force-settle, and at what punitive haircut?
   Recommendation: an `N` short enough that investors are never trapped
   indefinitely, with a haircut conservative enough that force-settlement never
   over-pays a redeemer at others' expense.

9. **Conservative haircut levels per off-Solana venue/asset.** Higher = safer
   NAV, more drag; lower = closer to true NAV, more manipulation room.

10. **Virtual-offset value + seed size.** Offset 0 (OZ default, already makes
    inflation unprofitable with a seed) up to ~12 (USDC-6-decimal analog of
    MetaMorpho's `max(0, 18-decimals)`). Recommendation: a meaningful offset plus
    a real seed + dead shares, locked non-withdrawable. (The 3-6 range is
    illustrative in OZ docs, not an OZ-recommended band.)

---

### Through-line

Every mandate is grounded in a correctly-characterized incident. The single root
error to avoid is **pooling unverifiable, manager-influenced value into the same
NAV and share price as the verifiable assets.** The master correction is
**two-NAV ring-fencing: redeem and price against what you can prove on Solana;
everything you can only be told is capped, clearly marked, and structurally
barred from moving the redeemable price.** That, plus an always-redeemable
on-Solana tranche, a latching drawdown breaker, an on-chain fee model, attested
*equity* (not quantity) with full liabilities, and retained-timelocked
upgradeability, is the difference between this fund and Stream Finance wearing a
zk costume.

---

### Incident references

One source per load-bearing incident characterization (official or audit-firm
post-mortems where they exist, rekt.news otherwise); the remaining incident
cites are checkable from these or from the venue's own disclosures:

- **Stream Finance** (~$93M, self-reported NAV, external-manager loss, hidden
  leverage): [CoinDesk, 2025-11-04](https://www.coindesk.com/markets/2025/11/04/stream-finance-faces-usd93-million-loss-launches-legal-investigation)
- **Cream yUSD** (~$130M, donation-manipulable share price via
  `pricePerShare`): [rekt.news/cream-rekt-2](https://rekt.news/cream-rekt-2)
- **Mango Markets** (~$115M, thin-feed self-pump, unrealized PnL borrowed
  against): [rekt.news/mango-markets-rekt](https://rekt.news/mango-markets-rekt)
- **Nomad** (~$190M, default `0x00` root treated as proven):
  [rekt.news/nomad-rekt](https://rekt.news/nomad-rekt)
- **Ronin** (~$625M, 5-of-9 with the never-revoked Axie DAO allowlist as the
  5th signature): [rekt.news/ronin-rekt](https://rekt.news/ronin-rekt)
- **Hyperliquid JELLY** (validator-vote forced settlement at a chosen price --
  a governance action, not a 51% attack):
  [Halborn, 2025-03](https://www.halborn.com/blog/post/explained-the-hyperliquid-hack-march-2025)
- **Kamino Lending precision bug** (caught pre-exploit, $0 loss; the rounding
  error is material only above 2^59; fixed with the fused mul-div
  round-down):
  [Certora, "Securing Kamino Lending"](https://www.certora.com/blog/securing-kamino-lending)
- **Radiant Capital** (~$50M+, 3-of-11 multisig; blind-signing of a forged
  display from malware-compromised signer devices):
  [Radiant Capital post-mortem, 2024-10](https://medium.com/@RadiantCapital/radiant-post-mortem-fecd6cd38081)
- **Drift Protocol** (~$285M, 2026-04; largest 2026 DeFi hack, costliest Solana
  incident after Wormhole. Not a contract bug -- privileged-access / social
  engineering: a six-month vault onboarding, then a worthless token (CVT)
  whitelisted as collateral at a fake price and drained, with no timelock on the
  allowlist change. North-Korea-linked UNC4736/AppleJeus, same actor as
  Radiant): [Chainalysis](https://www.chainalysis.com/blog/lessons-from-the-drift-hack/)
- **Balancer V2** (~$128.6M, 2025-11; rounding-direction mismatch in
  ComposableStablePool scaling, compounded via batched micro-swaps to
  suppress the invariant):
  [Trail of Bits analysis, 2025-11-07](https://blog.trailofbits.com/2025/11/07/balancer-hack-analysis-and-guidance-for-the-defi-ecosystem/)
