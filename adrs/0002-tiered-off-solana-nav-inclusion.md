# Architecture Decision Record (ADR) 0002 -- Tiered off-Solana net asset value (NAV) inclusion

Status: **proposed** (awaiting owner ratification)

Drives: the cross-chain product mandate -- the fund does holistic portfolio
management across venues and chains, with Hyperliquid (the deepest perp market)
central. Solana-only is not viable. Partially supersedes the
[security-design.md](../docs/security-design.md) Section 9 verdict ("ring-fence
all off-Solana value OUT of the redeemable price").

## Context

The original security design split portfolio value in two: on-Solana value
(oracle-priced, natively readable -- price and redeem against it) versus
off-Solana value (venue-API-trusted, where API is the venue's application
programming interface -- ring-fence it OUT of the redeemable price, side-pocket
it). That binary was correct for the technology it surveyed.

It is also incompatible with the product, which holds its core capital
off-Solana (Hyperliquid perps, Derive options). Ring-fencing the core out of the
redeemable claim makes investor shares unredeemable against the fund's actual
value -- useless.

One fact changes the picture for **Hyperliquid only**: HyperBFT (Hyperliquid's
Byzantine-fault-tolerant) consensus state is now readable on Solana. An
on-HyperEVM (Hyperliquid's Ethereum Virtual Machine) reader contract reads the
fund's HyperCore (Hyperliquid's exchange core) account via the read precompiles
(base `0x...0800`) documented in the Hyperliquid developer docs --
["Interacting with HyperCore"](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/hyperevm/interacting-with-hypercore)
and its published `L1Read.sol` interface; per-address inventory corroborated
against `hyperliquid-dev/hyper-evm-lib` (`HLConstants.sol`), both accessed
2026-06-10: `Position` 0x800, `SpotBalance` 0x801,
`UserVaultEquity{equity,
lockedUntilTimestamp}` 0x802, `Withdrawable` 0x803,
`markPx` 0x806, `oraclePx` 0x807, `L1_BLOCK_NUMBER` 0x809 -- and
`accountMarginSummary` 0x80F, which returns the venue-computed
`{accountValue, marginUsed, ntlPos, rawUsd}` and is therefore the preferred
single-field account-value read under the pre-implementation gate below.
Wormhole Queries (HyperEVM = chain 47) transports a guardian-signed result that
is verified on Solana by a **fund-owned verifier** against the core bridge's
guardian set (query-response parsing, signature quorum, guardian-set
rotation/expiry). Cross-Chain Queries (CCQ) protocol facts, per the Wormhole CCQ
whitepaper
([`wormhole-foundation/wormhole`, `whitepapers/0013_ccq.md`](https://github.com/wormhole-foundation/wormhole/blob/main/whitepapers/0013_ccq.md))
and the reference Solana verifier
([`wormholelabs-xyz/example-queries-solana-verify`](https://github.com/wormholelabs-xyz/example-queries-solana-verify)),
both accessed 2026-06-10: Queries responses are **not Verified Action Approvals
(VAAs)** -- guardians sign
`keccak256("query_response_0000000000000000000|" + keccak256(response))`, the
same guardian keys as VAAs but a different digest scheme; the core bridge
exposes **no instruction that verifies them** (the whitepaper proposes adding
one as a future optimization), so integrators verify with their own program
logic against the core-bridge-owned guardian-set accounts (the reference
verifier's `post_signatures` / `verify_query` flow); an `EthCallQueryResponse`
carries `{block_number, block_hash, block_time_us, results}`. The exact
signature-quorum rule for responses is NOT pinned by the whitepaper or docs
(only "a quorum of signatures suitable for on-chain submission") -- whether it
equals the VAA 13-of-19 rule is **unverified-pending-fixture**: pin it, with a
recorded query-proxy QueryResponse for a real HyperEVM `eth_call` as a test
fixture, in the failing-tests contract PR. This verifier is in scope for
security-design.md Section 7's cross-chain proof-verification checklist. This
lets Solana read the venue's own **consensus-computed account equity without a
manager signature**.

This does not make Hyperliquid trustless. It moves the residual from _manager
honesty_ (the Stream Finance failure mode) to _named venue-consensus +
cross-chain read-channel honesty + aggregate venue solvency_ -- a strictly
better residual to hold. Derive and any true centralized exchange (CEX) have no
such consensus read, so nothing changes for them.

## Decision

Tier value by what can be **independently verified**. Include a tier in the
redeemable price only when (a) an independent consensus read exists AND (b)
**two** independent oracle feeds (Pyth AND Switchboard -- rail 4's divergence
breaker requires both, so a single-feed asset is not eligible) price the asset.
Price every included tier at a **pessimistic floor**, behind a **latching
integral cap sized to total-loss-tolerable for the leg**. Ring-fence everything
else out, unchanged.

Inclusion is an earned property of a `(venue, asset)` pair, not a venue
allowlist:

- **Tier 1 -- on-Solana** (Drift / Kamino / Jupiter, oracle-priced, natively
  readable): in the redeemable price at full value. Genuinely trust-minimized.
- **Tier 2 -- Hyperliquid liquid majors with both independent oracle feeds (Pyth
  AND Switchboard)**, read via precompile + Wormhole Queries under all rails
  below: in the redeemable price at a pessimistic floor, behind the latching
  integral cap. Label: "venue-consensus-attested, cross-chain-read-trusted,
  capped" -- never "trustless". (Until rail 1's guardian node-sourcing
  requirement is confirmed, the honest label is "guardian-quorum-read,
  cross-chain-read-trusted, capped".)
- **Tier 3 -- Derive, any CEX, and Hyperliquid alts with no independent
  oracle**: ring-fenced OUT of the redeemable price, side-pocketed into a
  pro-rata claim token (the original Section 9 behavior, unchanged).

The redeemable price is **two-sided** -- never one symbol into both formulas:

- `redeemable_nav_redeem = verifiable_nav + floored_capped_tier2_nav` (the
  redemption-side base), where `floored_capped_tier2_nav` is the Tier-2
  consensus read **after** every rail haircut (per-tier `h`, aggregate-solvency,
  override detector), capped at the `L_standing` ceiling; its Tier-2
  contribution drops to `0` under a data-integrity halt (no usable read),
  whereas a valid-but-negative underwater read instead prices the leg to `<= 0`
  per the pre-implementation gate -- the two are distinct floors;
- `redeemable_nav_deposit = verifiable_nav` + the fully-recognized (un-lagged,
  un-floored) capped Tier-2 value at the adverse-high side (the deposit-side
  base) -- the floor is never a deposit entry price.

The full per-side formulas are in the Pricing mechanics section; the point of
stating both here is that the Decision-section contract is the two-sided pair,
not the single `redeemable_nav` a reader would otherwise extract. The shorthand
`floor(read_nav)` used elsewhere (e.g. ADR 0001's release gate) names this
domain operator -- the pessimistic floor -- never the mathematical round-down
function. Tier-3 `attested_nav` is not an input to either base.

**Release gate (blocking).** Tier-2 value MUST NOT enter
`redeemable_nav_deposit` -- the deposit price -- until open decision 9's
deposit-side false-read bound is ratified and ships in the failing-test
contract. Rail 6's latch bounds only the redemption channel; a persistent clean
false-high read otherwise over-prices every deposit on an unbounded channel the
latch does not cover (and the stale-read quarantine does not fire, because a
fresh false read is neither stale nor diverging). Until decision 9 lands, the
conservative interim is the strict Tier-1-only degenerate of decision 9's
candidate (ii): deposits into a Tier-2-exposed leg are priced at
`verifiable_nav` only (unreconciled Tier-2 excluded) or rejected; the redemption
side may include Tier-2 at the floored, latched `redeemable_nav_redeem`. This
mirrors ADR 0001's off-vault feature flag -- the mechanism gates its own
enablement, so Tier-2 deposit inclusion cannot ship ahead of its bound.

## Per-(venue, asset) residual-trust table

| Tier                                                                           | What is proven                                                                                                                                                                                                          | Irreducible residual                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | Verdict                                                          |
| ------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| 1 -- on-Solana                                                                 | on-chain state, oracle-priced                                                                                                                                                                                           | Pyth/Switchboard publisher honesty                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | In the price, full value                                         |
| 2 -- HL major + independent oracle, all rails                                  | a guardian quorum's remote-procedure-call (RPC) view of venue-consensus state at block N ("venue consensus computed this equity" holds only if guardians source from HyperBFT-validating nodes -- unconfirmed; rail 1d) | HyperBFT economic truthfulness (governance trust, not 51%-cost -- JELLY-class override is a governance action); read-channel honesty (Wormhole 13/19 today; honest-transmission-of-false-state is the dominant failure, quorum does NOT defend it) plus the queried RPC operators until rail 1d confirms sourcing; aggregate venue solvency; foreign-chain time-of-check-to-time-of-use (TOCTOU) + auto-deleveraging (ADL) in the settlement window; the rail-4 watcher for override classes with no precompile signal is an off-chain trusted component; a persistent clean false-high read over-prices deposits until the deposit-side bound lands (open decision 9); halt-timing redistribution against committed redemptions (open decision 10) | In the price at a pessimistic floor, capped total-loss-tolerable |
| 3 -- HL alt, no independent oracle (Tier-2 venue, Tier-3-treated asset)        | venue self-referential mark                                                                                                                                                                                             | Mango-class self-pump; divergence breaker disarmed                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | Ring-fenced OUT                                                  |
| 3 -- Derive, CEX (zero-knowledge Transport Layer Security (zkTLS) / venue-API) | the venue _said_ it                                                                                                                                                                                                     | venue-API honesty, not economic truth; notary committee adds no Byzantine fault tolerance (BFT) over one upstream session                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | Ring-fenced OUT (unchanged)                                      |
| Manager-signed NAV                                                             | nothing                                                                                                                                                                                                                 | total manager trust                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | Rejected. Stream Finance.                                        |

## Tier-2 inclusion rails (mandatory; type-driven test-driven-development (TTDD) contract)

Per the repo's strict two-PR rule, these ship as a **failing-tests contract
before any inclusion is enabled**. No rail, no Tier-2 inclusion.

1. **Gated transport, neutralized.** Wormhole Queries is closed beta, API-key
   gated, wallet-allowlisted -- it is NOT permissionless, so the submitter is in
   practice the manager's backend, who could withhold a read to hide a loss. Do
   not pretend otherwise; neutralize the lever: (a) absence/staleness of a fresh
   read forces the leg to its conservative LOWER bound immediately (fail-safe
   down, fail-closed up), so withholding can only lower credited value -- noting
   that "lower" is itself a timed redistribution lever against already-committed
   redemptions (pricing mechanics; open decision 10), not a loss-neutral
   direction; (b) >= 2 independent submitters (manager + watchtower), consume
   the LOWER equity within the freshness window -- and if fewer than 2
   quorum-valid submissions are retained when the window closes (e.g. the
   watchtower is offline and the manager submitted alone), the leg is treated as
   having no usable read for the epoch and floored per (a), regardless of the
   lone submission's content, so the `>= 2`-submitter diversity requirement is
   an on-chain invariant rather than a policy aspiration (at the cost of tying
   upward recognition to watchtower liveness). This is the minimum viable
   submitter model; ADR 0003, if ratified, supersedes it with an open bonded
   submitter set and a diversity quorum -- see open decision 2; (c) disclose the
   upward-recognition liquidity dependency on a gated service; (d) **node
   sourcing**: confirm how Wormhole guardians source HyperEVM queries
   (self-hosted HyperBFT-validating nodes vs third-party RPC) -- guardians
   cannot verify precompile output against anything consensus-signed, so an
   unconfirmed RPC operator is an additional, on-chain undetectable
   falsification point -- and require the watchtower submitter to run its own
   HyperBFT-validating node. Until sourcing is confirmed, "what is proven" is a
   guardian quorum's RPC view, and the investor label must say so.
2. **No block cherry-pick -- the freshness construction must be enforceable.**
   Queries signs whatever block is named and performs no freshness check, and a
   Solana verifier cannot infer "most recent finalized" from a single
   caller-selected historical read. Enforce it structurally: reject any
   caller-selected historical block unless the same submission carries an
   independently signed latest-head read; bind the state read to that head
   (within an enforced delta); enforce block-number monotonicity on the CONSUMED
   per-account tuple across epochs -- never as an admission filter within a
   freshness window: all quorum-valid submissions in the window are retained
   regardless of block order, and consumption waits for the window to close and
   takes the MIN equity over the full retained set (otherwise the faster
   submitter -- in practice the manager -- races the watchtower's earlier-block,
   lower-equity read out of rail 1b's take-the-lower comparison -- the
   oracle-update-selection maximal extractable value (MEV) security-design.md
   Section 3a kills for Pyth) -- noting the symmetric caveat that, because MIN
   is taken unconditionally over the retained set, any bonded submitter (manager
   OR a rogue/buggy watchtower) is a bounded _downward_ lever, not only by
   withholding: it can drive the consumed equity to the lowest guardian-signed
   value reachable within the enforced head-binding delta, a
   redemption-suppression / value-transfer lever bounded by tightening that
   delta and the consumed block's slot distance to the window-close head (see
   open decision 10); bound the response's `block_time_us` against Solana
   `Clock` at consumption -- noting, per the CCQ whitepaper cited above, that
   the `EthCallQueryResponse` timestamp is produced by the queried chain, NOT
   guardian-attested, so it sits on the same side of the trust boundary as the
   equity value: the `block_time_us`-vs-`Clock` check is therefore
   **defense-in-depth only** (it filters clearly-stale blocks from an honest
   submitter) and is NOT a security boundary against a lying submitter, who can
   forge the timestamp; the head-binding (not the timestamp) carries the
   anti-cherry-pick weight; bind it all into a consume-once
   `(epoch,
   account-id, block-N, nonce)` tuple. Infrastructure facts (checked
   2026-06-10): the
   [HyperEVM JSON-RPC docs](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/hyperevm/json-rpc)
   state `eth_call` serves only the latest block and "requests that require
   historical state are not supported" on the default RPC (independent archive
   nodes exist); Wormhole's
   [Queries supported-networks table](https://wormhole.com/docs/products/queries/reference/supported-networks/)
   lists HyperEVM (chain 47) with `eth_call` and `eth_call_by_timestamp` only --
   `eth_call_with_finality` is NOT listed for it. The rail therefore runs on
   latest-block reads plus the head-binding above, not on
   `eth_call_with_finality`; confirming guardian archive/finality support for
   HyperEVM is a pre-ratification blocker before this rail is treated as
   implementable.
3. **Aggregate-solvency cross-check.** The precompile returns a bookkeeping
   account value; position-book integrity is not collateral solvency (JELLY:
   faithfully-signed-but-economically-false equity on a socialized-loss book).
   Haircut by any readable system-level undercollateralization; carry a
   solvency-uncertainty haircut for what cannot be read; the
   total-loss-tolerable cap is the only honest bound on the unreadable
   remainder.
4. **Override / ADL / delisting detector**, separate from the mark-divergence
   breaker. Under a JELLY-class social override mark and oracle co-move, so the
   markPx-vs-oraclePx test does not fire. Primary breaker compares venue mark to
   an independent external reference (Pyth AND Switchboard); a separate event
   detector trips on override/forced-settlement/delisting/ADL, auto-side-pockets
   the position at the last independently-corroborated mark, and suspends upward
   recognition. The detector's observables, named per event class and all read
   through the same precompile + Queries channel: **forced closure / ADL** -- a
   per-account `Position` (0x800) delta between consecutive consumed reads with
   no matching manager-submitted order/fill record; **delisting** -- the asset's
   `markPx` (0x806) read reverting or the asset vanishing from perp metadata at
   the read block; **price override** -- a `markPx` discontinuity beyond a bound
   between consecutive consumed reads. Explicitly NOT detectable on-chain: a
   governance action that leaves position, mark, and oracle internally
   consistent at the read block (the full JELLY shape) produces no
   precompile-visible signal -- detecting it requires an **off-chain watcher**
   of HyperCore validator/governance actions, a named trusted component in the
   side-pocketing path (recorded in the residual-trust table). Without that
   watcher, rail 4 covers only the precompile-visible event classes above, and
   the investor label must say so (rail 1's honest-label requirement).
5. **Liquidity-fresh gating + lockup read.** Value-fresh is not liquidity-fresh.
   Read `lockedUntilTimestamp` at the same block; exclude equity locked past the
   epoch deadline from the redeemable portion; treat a lockup reset between read
   and settlement as a data-integrity halt; gate per-epoch off-Solana redemption
   at
   `min(static floor, recently-attested actual fills, depth-indexed capacity)`.
   The `min` is load-bearing: `recently-attested actual fills` reads through the
   same channel rail 6 flags as potentially lying, so it is admitted only as a
   floor-reducer -- it can tighten the gate, never raise it above
   `min(static floor, depth-indexed capacity)` -- and a false-high fills
   attestation therefore cannot widen redemption outflow.
   (`lockedUntilTimestamp` units and which lockups it covers -- vault deposits
   vs anything touching perp equity -- are not pinned by the official docs; pin
   them with a recorded precompile `eth_call` fixture before implementation.)
6. **Cumulative Tier-2-attributed outflow latch.** `L_standing` bounds the
   standing lie (a stock), not cumulative extraction: a persistent false read
   over-pays every epoch's redeemers in real Tier-1 USD Coin (USDC), and across
   `k` epochs the drain scales with `k x R_max` -- unbounded by the standing
   ceiling. The in-band checks do not catch it (the divergence breakers bound
   prices, not account-equity truth; the solvency cross-check and rail 5's
   attested fills read through the same channel that is lying). The **mandatory
   minimum** is the latch: a cumulative Tier-2-attributed-redemption-outflow
   integral latched against the same total-loss-tolerable ceiling -- no refill,
   governance reset only (this is the "cumulative-outflow latch" the
   Consequences section and open decision 2 depend on by name). The integral is
   **netted against USDC provably repatriated from the venue since latch
   inception** (`verifiable_nav`-class evidence a fake equity cannot produce):
   outflow funded by real repatriation does not consume the lie budget, while a
   false equity still cannot offset its drain. The repatriation evidence MUST be
   a consume-once proof tuple --
   `(source chain, pinned
   venue/bridge account id, tx id, amount, nonce)`
   tied to the same pinned Hyperliquid account the Tier-2 reads use, consumed at
   most once against the latch (the same consume-once discipline rail 2 applies
   to reads) -- so a forged or replayed repatriation cannot reduce latch
   consumption; the failing-test contract MUST include a vector where a fake or
   double-counted repatriation does not move the latch. Without the netting,
   honest operation alone consumes the budget -- for a fund whose core capital
   is Tier-2, routine redemptions cross a one-time-tolerable ceiling within a
   modest number of epochs, and routine governance resets train signers to
   rubber-stamp exactly the approval the latch exists to make exceptional. The
   expected reset cadence under honest operation is a sizing input to open
   decisions 1 and 4, and any reset requires a fresh independent reconciliation
   of venue equity against repatriation history, not just a vote. Funding the
   Tier-2 slice of each epoch's redemption payouts 1:1 from USDC provably
   repatriated since the prior epoch is strictly stronger, also satisfies this
   rail, and is admissible as an optional tightening on top of the latch --
   never as a substitute for it. The latch as stated bounds only the
   **redemption** channel: the same persistent clean false-high read also
   over-prices every deposit (the premium transfers pro-rata to incumbents, and
   when an incumbent later redeems, the latch consumes budget only on the
   Tier-2-attributed fraction of the payout), and the stale-read deposit
   quarantine does not fire -- a fresh, persistently false read is neither stale
   nor diverging. The deposit-side bound is **open decision 9**; until it is
   ratified, the deposit channel is the unbounded extraction path under a
   persistent clean false read.

Plus a hard pre-implementation gate: **the equity-reconstruction formula** (from
`Position` + `markPx` + isolated/cross + unrealized-profit-and-loss (uPnL)
sign + pending funding) is itself a re-pricing surface and is
manager-influenceable; until it is verified against HyperCore margining with
adversarial property-test vectors, reconstructed equity is unproven and the leg
is haircut to zero. Prefer reading a single venue-computed account-value field
-- one exists: `accountMarginSummary` (0x80F) returns `accountValue` directly.
Scope of the gate, explicitly: it applies to any **reconstruction** path.
Reading a venue-computed field directly (`accountMarginSummary.accountValue`, or
`UserVaultEquity.equity` for vault deposits) is NOT blocked by the
reconstruction gate, but carries its own separate verification requirement: the
field's semantics (what it includes and excludes, its response to
socialized-loss events, its relationship to liquidation buffers) must be
documented and verified with adversarial test vectors -- and pinned with a
recorded precompile `eth_call` response from a real HyperEVM node as a test
fixture -- before production. The fixture MUST record the exact
application-binary-interface (ABI) **width and sign** of every consumed field,
because the Solana verifier picks a Rust integer type to decode the opaque
CCQ-delivered bytes: account-value-class fields are the dangerous case -- the
cited `hyper-evm-lib` `HLConstants.sol` indicates `accountValue`/`rawUsd` are
**signed `int64`** (confirm against the live fixture, do not take this as
settled), so the verifier must decode the documented signed type and floor a
negative read (underwater / socialized-loss account) to its conservative lower
bound rather than wrapping. Include a negative-`accountValue` response as an
adversarial test vector asserting the leg prices to `<= 0`, not just that a
positive value round-trips. Rail 3's aggregate-solvency haircut applies to both
paths either way.

## Three claims this ADR explicitly does not rely on

Three load-bearing claims that an earlier framing of this tiering got wrong are
corrected below and recorded so the decision is not built on them; each states
the fact directly:

- **Wormhole Queries is gated, not permissionless.** The "manager removed / no
  freeze" claim is false as stated; rail 1 designs around the gate instead.
- **The cap is an integral, not a per-epoch flow.** A per-epoch recognition cap
  (`Delta_max` refilling each epoch) is the "salary to the attacker" leak the
  original Section 4 condemned. Standing recognized-but-unrealized value scales
  1:1 with allocation, so "economics close regardless of allocation size" is
  false. The binding constraint is `L_standing <= total-loss-tolerable cap`
  (integral); `Delta_max` (derivative) only bounds how fast a new lie enters.
- **Derive stays ring-fenced.** Derive is not a Wormhole Queries chain; no
  consensus read exists. Including Tier-3 venue-API value in the redeemable
  price re-creates Stream Finance. The verdict flip applies to Hyperliquid only.

## Pricing mechanics (augment ADR 0001 and Section 2; the deposit/withdraw math, virtual offset, seed, and adverse-to-actor max/min are retained)

- The ADR 0001 / Section 2 conversion formulas carry over with the pricing base
  substituted **per side** -- the two-sidedness is part of the formula contract,
  not a prose gloss; the virtual offsets (`V_SHARES = 10^offset`,
  `V_ASSETS = 1`) apply unchanged to each combined base:
  - `redeemable_nav_redeem = verifiable_nav + floored_capped_tier2_nav` (the
    Decision-section quantity: floored, capped, slow-up-lagged) -- the
    redemption-side base;
  - `redeemable_nav_deposit = verifiable_nav` plus the fully-recognized
    (un-lagged, un-floored) capped Tier-2 value, taken at the adverse HIGH side
    `max(last-good, current)` under any staleness/divergence condition (when
    deposits are not rejected outright) -- the deposit-side base. _(This is the
    post-gate target; interim until open decision 9 is ratified, Tier-2 is
    excluded and deposits price Tier-1-only at `verifiable_nav` -- see the
    Decision-section release gate.)_
  - `shares_out = deposit_assets * (total_shares + V_SHARES) / (redeemable_nav_deposit + V_ASSETS)`
    -- round down
  - `assets_out = redeem_shares * (redeemable_nav_redeem + V_ASSETS) / (total_shares + V_SHARES)`
    -- round down
- Pessimistic floor: a stale read or an independent-oracle stress move forces
  the leg to its lower bound immediately, before any down-proof arrives. The
  floor prices redemptions; it is never a deposit entry price -- see the
  stale-read deposit rule below.
- Stale-read deposit rule (mandatory; ships in the failing-test contract): the
  floor is safe only for redemptions -- a deposit priced at an understated NAV
  mints excess shares that capture value on re-recognition, so a withheld read
  becomes a cheap-entry lever for the gated submission path. The same
  staleness/divergence condition that floors the leg therefore also quarantines
  new deposits into it: reject them, or price them against the adverse high side
  `max(last-good, current)` Tier-2 value while redemptions use the low side.
- Latching integral cap on standing recognized-but-unrealized Tier-2 value, plus
  the `Delta_max` per-epoch derivative cap; manager fees crystallize only on
  realized off-Solana proceeds, never on attested marks.
- **Slow-up** (definition): the redemption floor `floored_capped_tier2_nav`
  recognizes upward moves _slowly_ and downward moves _immediately_ -- it rises
  by at most the `Delta_max` per-epoch derivative cap regardless of the latest
  verified read (a verified read above the current floor raises it by
  `min(verified - floor, Delta_max)`), while a verified read below the floor
  lowers it at once (rail 1a's fail-closed-up, fail-safe-down). This is what
  "how fast a new lie enters" bounds: `Delta_max` rate-limits recognition of an
  increase; its magnitude is sized under open decision 4. Slow-up applies to the
  redemption floor only; depositors are priced against the fully-recognized
  (un-lagged) value, so recognition lag is never a deposit-capture option.
- Tier-3 realization crediting: all Tier-3 value is continuously tracked in the
  non-redeemable accounting partition with holders-of-record bookkeeping --
  claim units accrue at every deposit and redemption, not only on
  breaker-triggered side-pockets. Realized Tier-3 proceeds (positions closed,
  USDC repatriated to Solana) credit the partition's holders-of-record before
  any residual enters the common `verifiable_nav`, so a deposit made while value
  sits in Tier 3 acquires no claim on its later realization and manager-timed
  realization cannot transfer value to fresh deposits.
- Deposits are quarantined from a leg under data-integrity halt (staleness and
  divergence included, per the stale-read deposit rule); the drawdown breaker is
  denominated on the pessimistic-floored `redeemable_nav_redeem` so it trips on
  a real Hyperliquid crash and cannot be suppressed by withholding; tripping
  latches trading and upward recognition only -- the redemption path stays open
  at the floored price (security-design.md Principle 5).
- The open-at-the-floor rule creates a named TOCTOU tension for
  already-committed redemptions: cancellation is disallowed after commit
  (Section 2), so a redemption committed in a healthy period force-settles at
  the floored (possibly zero-Tier-2) price if a halt spans settlement -- a value
  transfer from exiting to remaining holders (including the manager's stake and
  fee base) that a halt-controller can time, since the staleness condition is
  influenceable through the gated submission path. "Withholding can only lower
  credited value" is therefore an extraction direction as well as a safety
  property (rail 1a). Whether committed redemptions under an active halt (a)
  roll to the next epoch at the redeemer's standing instruction, or (b) settle
  the Tier-1 slice immediately and escrow the Tier-2 slice for a true-up against
  the first post-halt verified read (never manager re-priced), is **open
  decision 10**. The current default -- settle committed redemptions at the
  floor -- is Principle 5's open-at-floor exit-liveness; the unresolved tension
  is that it is simultaneously a halt-controller's timed transfer from exiting
  to remaining holders. Decision 10 must choose whether to keep the floored
  default or adopt mitigation (a)/(b) before Tier-2 settlement ships; until it
  is ratified the lever is disclosed, not yet mitigated (Section 6's timeout
  payout and the residual-trust table reflect the same open status).

## In-transit accounting token (candidate refinement; see ADR 0004)

The LayerZero x Centrifuge report "Unlocking Tokenized Fund Composability"
proposes "accounting tokens" for capital mid-transfer between chains -- a
receipt token on the source and a liability token on the destination so the
value does not "disappear from NAV." That pattern is admissible here, but
**only** as a proof-gated refinement of the
[security-design.md](../docs/security-design.md) Section 8 in-transit rule --
never as the report states it (an unbacked receipt/liability pair the issuer
mints on faith, which is exactly the bridge/manager trust Section 8 refuses).

Problem it addresses: Section 8 excludes bridged principal from `verifiable_nav`
and from the Tier-3 claim while it is in transit between Solana and the venue.
That exclusion is the safe direction, but it makes the fund's NAV dip while
principal is mid-bridge and jump when it lands -- a temporary mispricing window
(a depositor entering in the dip buys cheap shares; a redeemer settling in the
dip is short-changed), the report's "NAV drops temporarily then jumps."

Refinement: represent in-transit principal as a receipt entry that contributes
to NAV **only** against the same consume-once repatriation proof tuple rail 6
already requires --
`(source chain, pinned venue/bridge account id, tx id, amount, nonce)` -- so the
credit is evidence-backed, not manager-assertable, and a forged, replayed, or
double-counted proof cannot mint phantom in-transit value (rail 6's failing-test
vector already covers that case and extends to this credit). Invariants it must
hold:

- **Fail-safe to exclusion.** With no fresh proof the in-transit entry is `0` --
  the current Section 8 behavior -- never a stale credit.
- **Adverse-to-actor.** Smoothing redistributes value between depositors and
  redeemers inside the transit window, so in-transit value enters the two sides
  under the same `max`/`min`-over-conversion discipline as every other leg
  (Pricing mechanics): redeemers price against the low side, depositors the high
  side, never one symbol into both.
- **Latch-direction-correct.** The source-side receipt credit and the venue-side
  debit must net to zero against rail 6's cumulative-outflow integral; the token
  smooths _pricing_ across the bridge window and must not become a second
  channel that relaxes the latch.
- **Capped.** In-transit principal counts against the destination venue's
  total-loss-tolerable cap (it is the fund's own principal headed to that
  venue), never as extra headroom.

Status: candidate refinement, not ratified. It is strictly optional on top of
the Section 8 exclusion, which stays the safe default. If adopted it ships as a
failing-tests contract like every rail, with a vector asserting that an unproven
in-transit amount contributes `0`. Its architectural context -- and the broader
question of whether to adopt the report's hub-and-spoke pattern wholesale -- is
[ADR 0004](0004-nav-accounting-architecture.md).

## Consequences

- Investor shares redeem against `verifiable_nav` (trust-minimized) plus a
  floored, capped, override-aware Hyperliquid contribution, plus a side-pocketed
  claim token for Derive/CEX value.
- Inclusion improves the _marking_ residual (manager -> venue consensus). It
  does NOT escape the global-share-price correction (a corrupted leg harms 100%
  of capital) or the total-loss-tolerable cap. We do not claim it does.
- A large Hyperliquid allocation forces a real tradeoff: either a low standing
  ceiling (tracking error vs true NAV) or accepting that the leg's total loss is
  survivable at that size. You cannot bond a majority of assets under management
  (AUM), so the cap -- not a bond -- is the honest bound. Rail 6 is what makes
  that bound real: without the cumulative-outflow latch, a persistent false read
  extracts multiples of the standing ceiling through the redemption queue over
  time.
- Tier-2b migration target: a Solana-side zero-knowledge (ZK) light client of
  HyperBFT removes the guardian-quorum residual. It does not ship today.
- The six rails land as failing tests first (strict TTDD), then the
  implementation enabling inclusion.

## Open decisions for owner ratification

These gate inclusion and need an explicit human call (they revise/extend
security-design.md Section 11):

1. **Tier-2 (Hyperliquid) exposure ceiling** = total-loss-tolerable for the leg.
   What standing recognized value can the fund lose entirely without harming
   Tier-1 holders? (Ratify knowing the ceiling and rail 6's latch bound the
   redemption channel only; the deposit channel stays unbounded under a
   persistent clean false read until decision 9 lands.)
2. **Cross-chain-read economic-security model**: accept Wormhole 13-of-19 (with
   honest-transmission-of-false-state as the dominant, undefended-by-quorum
   failure, bounded by the standing cap together with rail 6's
   cumulative-outflow latch -- the standing cap alone bounds the stock, not the
   multi-epoch redemption drain), and require an independent second path to
   agree before RAISING value? (Proposal: ADR 0003 — an open bonded submitter
   set with a diversity quorum requiring K>=2 distinct channels; proposed,
   awaiting ratification. Note the exact Wormhole response quorum threshold
   remains unverified-pending-fixture per the Context above.)
3. **Asymmetric-ratchet loosen-side timelock** >= the time to unwind and
   repatriate the Tier-2 tranche to USDC at the venue-capacity-limited gate rate
   (because verifiable_nav is now the minority -- "exit during the timelock" no
   longer holds otherwise).
4. **The cap tuple**
   `{Delta_max, L_standing ceiling <= total-loss-tolerable, h
   per tier, R_max, rail 6's cumulative redemption-outflow latch}`
   -- ratify the integral binding constraint plus the latch, not a per-epoch
   bond.
5. **OES custody** (blocking): route Hyperliquid principal through an
   off-exchange-settlement (OES) custodian? If so it is a named trusted party
   with a no-rehypothecation proof-of-reserves (PoR); if the manager keeps the
   trade-direction relationship, "manager never holds it" is cosmetic --
   disclose.
6. **Bond-poster and slash trigger** (blocking): a real backstop needs a
   third-party / actively-validated-service (AVS) poster and an objective
   on-chain slash trigger (attested block-N equity vs a later independent read),
   never a governance vote. If neither exists, credited insurance = 0 and we
   disclose it.
7. **Derive: the mandate-vs-safety business call.** Derive cannot be Tier 2 (no
   consensus read) and cannot be redeemable as Tier 3 (Stream pattern), so its
   value is side-pocketed, non-redeemable-until-realized. Does side-pocketed
   Derive value satisfy the mandate, or is Derive deferred until a consensus
   read exists? (No-arbitrage haircuts crush most OTM Derive strikes toward zero
   anyway, so the redeemable value forgone may be small.) (OTM =
   out-of-the-money.)
8. **Inclusion-eligibility allowlist** (per asset): which Hyperliquid perps are
   eligible (Pyth AND Switchboard feeds + minimum depth/open-interest (OI)),
   with depth-indexed per-perp sub-caps. Everything else on Hyperliquid is
   Tier-3-treated.
9. **Deposit-side false-read bound** (blocking for Tier-2 inclusion; extends
   rail 6). Rail 6's latch bounds redemption outflow; a persistent clean
   false-high read symmetrically over-prices every deposit, the harvested
   premium lands in `verifiable_nav`, and the latch consumes budget only on the
   Tier-2-attributed fraction when it is later redeemed -- so cumulative
   depositor harm scales with cumulative deposits, unbounded by `L_standing`,
   `Delta_max`, or the latch. Candidate mechanisms: (i) track the cumulative
   Tier-2-attributed deposit premium
   (`deposit * tier2 / redeemable_nav_deposit`, summed across epochs) and latch
   it against the same ceiling class, governance-only reset; or (ii) exclude
   Tier-2 value from the deposit price entirely unless the read has been
   reconciled against provably repatriated USDC within K epochs. Tradeoff: (i)
   preserves deposit pricing fidelity but adds a second latch to size; (ii) is
   simpler and stronger but understates the entry price for honest depositors
   whenever reconciliation lags.
10. **Committed-redemption settlement under an active data-integrity halt**.
    Settling committed redemptions at the floor (the current default) keeps
    Principle 5's exit-liveness but hands a halt-controller a timed value
    transfer from exiting to remaining holders; the alternatives -- (a) roll to
    the next epoch at the redeemer's standing instruction, or (b) settle the
    Tier-1 slice now and escrow the Tier-2 slice for a true-up against the first
    post-halt verified read -- trade exit-latency or settlement complexity for
    removing the lever. Pick one; the tension is recorded in the pricing
    mechanics above and the residual-trust table.

A subordinate bonded dispute window over the relayed Tier-2 read (valid because
the state is publicly verifiable) is admissible as a cross-check, strictly
secondary to the cap.
