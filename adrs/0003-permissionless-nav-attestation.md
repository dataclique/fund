# ADR 0003: Permissionless NAV attestation

Status: proposed, awaiting review. Answers ADR 0002's open decision 2 (the
cross-chain-read trust model) and replaces the manager/backend as the
attestation submitter. Builds on `0001-donation-resistant-share-pricing.md`
(pricing invariants) and `0002-tiered-off-solana-nav-inclusion.md` (tiering,
rails, two-sided pricing); neither is modified — where this ADR is stricter,
this ADR wins on the attestation path.

## Context

The system is deliberately centralized for MVP: Turnkey custody, one
execution backend, one manager. The moneymentum SPEC currently says "the
backend aggregates positions across all venues, computes total NAV, and
posts signed attestations to the vault program" — that single sentence is a
Stream-Finance-class trust assumption. If the operator alone tells the vault
what the off-Solana capital is worth, share pricing is manager-attested and
the rest of the security design decorates a trusted core. NAV attestation is
the one subsystem that must be permissionless in the MVP: anyone can submit,
anyone can dispute, anyone can verify, and the operator holds no special
write authority over the redeemable price.

### What the research established (2026-06-10)

Verified against primary sources (each marked V); research-grade leads that
survived fetch but not full adversarial verification are marked R and must
be re-verified before implementation freezes a dependency on them.

- **V — HyperCore state is not commitment-anchored.** The official node
  docs describe binary-only distribution, snapshots as local `.rmp` dumps,
  and contain no mention of any Merkle/state root or commitment over
  exchange state; HyperBFT's block/QC structure and signature scheme are
  undocumented. Consequence: there is nothing for a ZK proof of account
  equity to anchor to. No design can eliminate observation trust for
  Hyperliquid today; it can only diversify and bond it.
- **V — Wormhole Queries is "permissioned but trustless."** The CCQ server
  brokers requests behind an API key, but responses are signed by the
  Guardian set and verifiable by anyone, on Solana included. (The exact
  response signature-quorum threshold — whether it matches the VAA
  13-of-19 rule — is **unverified-pending-fixture** per ADR 0002 and must
  be pinned by a recorded `QueryResponse` fixture before the verifier
  relies on it.)
  The supported-networks table lists HyperEVM (chain 47) with `eth_call`
  and `eth_call_by_timestamp` (no finality variant). The gate is therefore
  on *requesting*, not on *verifying*: a key holder can withhold but cannot
  forge. Integrity decentralized; liveness permissioned.
- **V — Wormhole's HyperEVM launch integration is asset transfers only**
  (Portal/Connect); the launch material documents no Queries/NTT/messaging
  for HyperEVM. The Queries support above comes from the reference table,
  and ADR 0002 already requires pinning it with a recorded fixture.
- **V — LayerZero lzRead documents only Arbitrum, Base, Ethereum, and
  Optimism as data chains** — neither HyperEVM as a read target nor Solana
  as a destination is confirmed. Not a candidate path today.
- **V — Switchboard On-Demand is live on Solana** with permissionless feed
  creation over arbitrary HTTP jobs; oracle operators are an open set gated
  by TEE hardware attestation (verified Switchboard code only) with $SWTCH
  stake slashable for downtime or withholding. Trust root: TEE integrity
  plus economic security — per the security design's BitForge lesson, a
  TEE is one layer, never the perimeter.
- **V — Groth16 verification on Solana is production-real.** Light
  Protocol's `groth16-solana` verifies in <200K CU on the alt_bn254
  syscalls (mainnet since 1.18) and was audited in the Light v3
  assessment; Succinct's `sp1-solana` (~280K CU) exists but is explicitly
  unaudited. The settlement substrate for proof-carrying attestations is
  ready; SP1's wrapper needs an audit before production.
- **R — Reclaim Protocol has a deployed Solana zkTLS verifier program**
  (`8rYXFrtST4ePpMWcEqhazFyRG2DtCUqgtFmKT7FdjRyp`) with an official Anchor
  example. zkTLS trust models (notary/attestor sets, MPC-TLS vs proxy)
  carry their own collusion and freshness caveats.
- **R — Wormhole x Boundless run a production ZK prover of Ethereum
  consensus finality, live on EVM chains only as of Aug 2025; Solana
  verification not live.** Herodotus-class tooling proves OP-Stack L2
  storage against L1 output roots. The Derive unlock exists Ethereum-side;
  the Solana leg is the missing piece.

## Decision

Introduce an **open attestation protocol** on Solana — a `nav-attestor`
program (sibling to the vault; the vault consumes its finalized epochs) —
with four properties: open submitter set, channel-typed evidence, diversity
quorum, and bonded disputes. The manager's backend becomes one submitter
among any, with no special rights.

### 1. Open submitter set

Anyone may submit an attestation for epoch `E` for a `(venue, asset)` leg
by locking a bond in the attestor program. No allowlist, no role accounts.
Submissions carry the claimed equity value, the observation block/time, and
**evidence** of one channel type below. All signature and proof
verification happens on-chain; the off-chain observer software is open
source so any third party can run it against public endpoints.

### 2. Channel-typed evidence

Channels are ranked by evidence class. Each channel has independent failure
modes, which is what the quorum buys.

| Class | Channel | On-chain verification | Failure mode |
| ----- | ------- | --------------------- | ------------ |
| A | Wormhole Queries response: guardian-signed `eth_call` of the L1Read precompiles (`accountMarginSummary` 0x80F preferred) at a head-bound block, per ADR 0002 rails 1-2 | Guardian-set signature check against the stored guardian set (quorum threshold unverified-pending-fixture per ADR 0002) | Guardian collusion; honest-transmission-of-false-state (HL RPC lies to guardians); withholding by key holders |
| B | Switchboard On-Demand result over the same precompile read (EVM-RPC job) and/or the HL info API | Switchboard quote-program Ed25519 `verified_update` flow | TEE compromise; operator-set capture; source RPC lies |
| C | zkTLS proof of the HL info API `clearinghouseState` response | Reclaim-class verifier program | Notary/attestor collusion; TLS termination point; response freshness/replay |
| D | Bare bonded observation (no proof) | Bond lock only | Anything — pure economic security |

Class A is the anchor channel: ADR 0002's rails (head binding, freshness on
`block_time_us`, lower-of-two consumption, monotonic consumed tuples)
apply to it unchanged. Classes B-D are diversity channels. Class D exists
so that a total outage of proof infrastructure degrades the system to
bonded-optimistic rather than dead — it can never finalize alone.

### 3. Diversity quorum

An epoch leg finalizes when, within the freshness window:

- at least `K` distinct channels agree within tolerance `epsilon`
  (parameters per `(venue, asset)`, owner-ratified; default K=2), AND
- at least one agreeing submission is class A, AND
- submissions come from at least `K` distinct bonded submitters.

The finalized value consumed by the vault is **two-sided per ADR 0002**:
redemptions price against the minimum agreeing value (then floors,
haircuts, caps); deposits against the maximum (adverse-high). Disagreement
beyond `epsilon` across channels is a data-integrity signal: the leg enters
the existing ADR 0002 halt machinery (deposits quarantined, redemptions at
the floor), not a "pick one" resolution.

### 4. Bonded disputes

During window `W` after finalization, any bonded party may dispute by
submitting a contradicting observation of an **equal or higher evidence
class**. Resolution ladder: A > B > C > D; an A-vs-A contradiction (two
guardian-signed responses disagreeing beyond tolerance for the same bound
block) is by construction a data-integrity halt, never auto-resolved.
The losing side's bond is slashed to the disputer. Withholding — the one
attack no bond can punish — fails closed: no fresh finalized epoch means
ADR 0002 stale-read rules apply automatically.

Slashing pays from attestation bonds only. It does not touch fund capital,
and the cumulative-outflow latch (ADR 0002 rail 6) remains the backstop
against any value extraction a wrong-but-finalized epoch enabled.

### Per-venue instantiation

- **Tier 1 (on-Solana)** — unchanged: direct reads, no attestation needed.
- **Hyperliquid (Tier 2)** — MVP ships channels A + B (both live today;
  C added when the Reclaim-class verifier and the HL API's TLS posture are
  validated). K=2 with A mandatory. The pessimistic floor, latching caps,
  and stale-read deposit quarantine from ADR 0002 stay exactly as ratified;
  this ADR only replaces *who can feed* the rails.
- **Derive / EVM L2s (Tier 3 today)** — remains ring-fenced for MVP. The
  concrete unlock chain, in dependency order: (1) Derive output roots land
  on Ethereum L1 (OP Stack); (2) an Ethereum-consensus ZK verifier on
  Solana (the Wormhole x Boundless prover exists; its Solana verifier is
  the missing piece — the Groth16 substrate on Solana is ready and Light's
  verifier is audited); (3) storage proofs of Derive settlement-contract
  state against the proven L1 root (Herodotus-class). When (2) exists,
  Derive inclusion graduates from "impossible" to "channel-A-equivalent
  with Ethereum L1 as the anchor" — strictly better evidence than the
  Hyperliquid leg, because it anchors to a real consensus commitment.
- **CEXes** — permanently Tier 3 under this ADR; zkTLS of CEX APIs is at best
  a single-source class-C leg, and the quorum requires class A mandatory plus
  K>=2 distinct channels, so no CEX leg can ever reach quorum finalization.

### The custom-cryptography track (flagged for external review)

Two components need cryptographer review before implementation; both are
upgrades, not MVP blockers:

1. **Aggregated channel-A verification.** Verifying a full guardian quorum of
   secp256k1 signatures (≈13, pending the ADR 0002 fixture) per leg per epoch
   on Solana is CU-expensive. An SP1/RISC
   Zero circuit that verifies the guardian quorum (and the response
   parsing) off-chain and submits one Groth16 proof (<200K CU via the
   audited verifier) amortizes cost and removes parsing risk from the
   program. Standard recursion, but the circuit's statement ("this is a
   valid guardian-quorum response for this query at this block") must be
   reviewed for under-constraint bugs — the classic ZK failure mode.
2. **HyperBFT light-client readiness.** Today this is blocked on facts,
   not proofs: HyperCore commits state nowhere we can verify (V). The
   actionable artifact is a precise public ask to Hyperliquid — a
   consensus-signed state commitment over clearinghouse state (even at
   snapshot cadence) — plus a pre-specified circuit sketch (HotStuff-class
   QC verification over ed25519, precedented by Tendermint-X/Blobstream)
   so the Tier 2b upgrade in ADR 0002 becomes implementable the day such a
   commitment exists. Until then, claims that any vendor "verifies
   Hyperliquid state" cryptographically should be treated as marketing.

## Consequences

- A new `nav-attestor` Anchor program: bonds, submissions, channel
  verifiers (guardian-set check first; Switchboard CPI; zkTLS verifier CPI
  later), quorum/finalization state, dispute/slash flow. The vault reads
  only finalized epochs from it.
- The moneymentum SPEC's "NAV oracle" paragraph must be rewritten: the
  backend's NAV computation becomes an internal estimate and one channel
  submission, not the attestation.
- Per-epoch cost: bonds are rent-class deposits; channel-A verification
  CU-heavy until the aggregation circuit lands; cranks are permissionless
  (any submitter finalizes once quorum exists).
- Open parameters for ratification: bond size, K, epsilon, W, per-leg
  freshness; these join ADR 0002's open decisions 1/4 sizing discussion.
- Re-verify before freeze (research-grade): lzRead path matrix, Boundless
  Solana verifier status, Reclaim Solana program provenance and audit,
  Switchboard slashing parameters, CCQ access-tier roadmap.

## Open questions

1. Bond denominations and sizing — fixed SOL/USDC bond vs scaled to leg
   exposure; griefing economics of dispute spam at K=2.
2. Whether class-D submissions should earn fees (incentivizing a standing
   permissionless observer set) or exist purely as a degraded mode.
3. Dispute-window length vs the epoch cadence ADR 0002 assumes; W longer
   than an epoch forces carrying two candidate NAVs.
4. Whether channel B should require its own K>=2 distinct Switchboard
   feeds (different operators) before counting as one agreeing channel.
