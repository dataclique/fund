//! Deposit: an investor swaps quote tokens for freshly-minted fund shares.
//!
//! The steps are deliberately ordered -- price, move quote in, then mint shares
//! out -- so the share price is struck against the fund's state *before* the
//! inbound transfer changes it. The depositor's own `amount` must not be counted
//! in the basis it is buying into: pricing after the transfer would put `amount`
//! in the denominator of the share formula and mint them too *few* shares,
//! diluting the incoming depositor in favor of existing holders.
//!
//! ## Pricing basis (v0, and why it is not yet ADR 0001)
//!
//! Shares are priced off `vault.amount` -- the raw SPL balance of the vault. That
//! is the donation-manipulable basis ADR 0001 (donation-resistant share pricing)
//! exists to replace with an internally-tracked `Fund.total_assets`. We ship the
//! vault-balance basis for v0 anyway because `total_assets` requires a `Fund`
//! layout change plus a state update on every value-moving instruction (the
//! ADR's Option B) -- that is its own feature, sequenced separately. The residual
//! is the ERC-4626 first-depositor inflation attack, **accepted for v0 and
//! recorded in SPEC.md**, not a property this code claims to defend. Do not
//! describe this instruction as donation-resistant until the ADR 0001 migration
//! lands. See `adrs/0001-donation-resistant-share-pricing.md` and the SPEC
//! deposit section.
//!
//! ## Security
//!
//! The program never trusts a caller-supplied account: the fund/vault/shares
//! PDAs are re-derived from their stored canonical bumps, the vault and shares
//! mint are pinned to the fund's own mints and authority, the token program is
//! pinned via `Program<Token>`, and the investor authorizes the quote transfer
//! as a `Signer`. The per-field docs below record which sealevel-attack each
//! constraint defends (see `docs/sealevel-attacks.md`).

use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, MintTo, Token, TokenAccount, Transfer};

use crate::error::FundError;
use crate::state::Fund;

/// Accounts for [`handler`]. Every account is constrained so a caller cannot
/// substitute one it controls; the field docs name the specific defense.
#[derive(Accounts)]
pub struct Deposit<'info> {
    /// The depositor. Must be a `Signer`: it authorizes moving quote out of its
    /// own ATA, and without that signature anyone could drain the investor's
    /// quote into the fund on their behalf (signer-authorization defense).
    #[account(mut)]
    pub investor: Signer<'info>,

    /// The fund being deposited into. Re-derived from its *stored* canonical bump
    /// (never a caller-supplied bump) so a non-canonical look-alike PDA cannot be
    /// substituted; `Account<Fund>` additionally enforces program ownership and
    /// the 8-byte discriminator (bump-canonicalization + owner + type defenses).
    #[account(
        seeds = [b"fund", fund.manager.as_ref(), &fund.name],
        bump = fund.fund_bump,
    )]
    pub fund: Account<'info, Fund>,

    /// The fund's quote vault. Pinned to the fund's own `quote_mint` and to the
    /// fund PDA as authority, so a caller cannot point the deposit at a vault they
    /// control and have shares minted against it (account-data-matching defense).
    #[account(
        mut,
        seeds = [b"vault", fund.key().as_ref()],
        bump = fund.vault_bump,
        token::mint = fund.quote_mint,
        token::authority = fund,
    )]
    pub vault: Account<'info, TokenAccount>,

    /// The fund's shares mint, with the fund PDA as the *sole* mint authority -- so
    /// the program is the only thing that can mint shares and supply tracks
    /// deposits exactly (no out-of-band minting can dilute holders).
    #[account(
        mut,
        seeds = [b"shares", fund.key().as_ref()],
        bump = fund.shares_mint_bump,
        mint::authority = fund,
    )]
    pub shares_mint: Account<'info, Mint>,

    /// Source of the deposited quote: the investor's own quote ATA, pinned to the
    /// fund's `quote_mint` and the investor's authority so the debit can only come
    /// from the depositor's legitimate account.
    #[account(
        mut,
        associated_token::mint = fund.quote_mint,
        associated_token::authority = investor,
    )]
    pub investor_quote_ata: Account<'info, TokenAccount>,

    /// Destination for the minted shares. `init_if_needed` is safe here despite
    /// the documented footgun: the `associated_token` constraints pin any
    /// pre-existing account to the canonical ATA for (investor, shares_mint), so
    /// an attacker cannot substitute an account with a different mint or
    /// authority -- the only accepted pre-existing state is the investor's own
    /// legitimate ATA. We init-if-needed (rather than require it already exists)
    /// so a first-time depositor is not forced into a separate ATA-creation
    /// transaction before they can deposit.
    #[account(
        init_if_needed,
        payer = investor,
        associated_token::mint = shares_mint,
        associated_token::authority = investor,
    )]
    pub investor_shares_ata: Account<'info, TokenAccount>,

    /// Pinned to the canonical SPL Token program so the transfer/mint CPIs below
    /// cannot be redirected to a malicious look-alike (arbitrary-CPI defense).
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

/// Deposit `amount` quote tokens and mint the investor their pro-rata share of
/// the fund.
///
/// Reverts the whole transaction (atomically, so no partial state survives) on a
/// zero deposit, a deposit that would push AUM past the fund's capacity, an
/// arithmetic overflow, or a deposit so small it would round to zero shares.
pub fn handler(ctx: Context<Deposit>, amount: u64) -> Result<()> {
    // Reject zero up front. A zero deposit is never legitimate and, left to flow
    // through, would still pay the `init_if_needed` rent for a no-op -- failing
    // immediately is strictly cheaper for the caller and keeps the happy path
    // free of a degenerate case.
    require!(amount > 0, FundError::ZeroDeposit);

    // Price against the vault balance *before* the inbound transfer, so the
    // depositor's own `amount` is not counted in the basis it is buying into.
    // Reading it after would put `amount` in the denominator of the share
    // formula and mint too few shares, diluting the incoming depositor. (v0
    // basis: this is the donation-manipulable `vault.amount` -- see the module
    // docs; ADR 0001 replaces it with `Fund.total_assets`.)
    let aum_before = ctx.accounts.vault.amount;

    // Enforce capacity against *projected* post-deposit AUM, not current AUM, so
    // this deposit cannot push the fund over its capacity ceiling. We add the
    // requested `amount`: the classic SPL Token program has no transfer fee, so
    // the vault receives exactly `amount` and projected AUM equals actual. (A
    // Token-2022 mint with a transfer-fee extension would land *less* than
    // `amount` in the vault, breaking that equality -- but `token_program` is
    // pinned to classic Token above, so that case cannot arise here. Revisit this
    // if Token-2022 support is ever added.)
    let projected_aum = aum_before
        .checked_add(amount)
        .ok_or(FundError::MathOverflow)?;
    require!(
        projected_aum <= ctx.accounts.fund.capacity,
        FundError::CapacityExceeded
    );

    // Args in order: quote deposited, shares outstanding, quote AUM before. All
    // three are bare `u64`, so the compiler cannot catch a transposed call -- keep
    // this site and the signature in lockstep. (Domain newtypes that would make a
    // swap a type error are tracked for the ADR 0001 pricing rewrite, which
    // changes this signature anyway.)
    let shares_out = shares_for_deposit(amount, ctx.accounts.shares_mint.supply, aum_before)?;

    // Move the quote in before minting. If the mint below fails, the whole
    // transaction reverts and this transfer reverts with it (Solana transactions
    // are atomic -- a propagated CPI error rolls back every account change), so
    // there is never a window where the fund holds the quote without having
    // minted the matching shares.
    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.investor_quote_ata.to_account_info(),
                to: ctx.accounts.vault.to_account_info(),
                authority: ctx.accounts.investor.to_account_info(),
            },
        ),
        amount,
    )?;

    // Surface a supply overflow as our own legible error. `mint_to` would itself
    // fail if the new supply wrapped a u64, but only as an opaque SPL error;
    // checking here returns `MathOverflow` instead. (Unreachable while capacity
    // bounds AUM and shares track AUM, but cheap insurance against a future
    // accounting change that decouples them.)
    ctx.accounts
        .shares_mint
        .supply
        .checked_add(shares_out)
        .ok_or(FundError::MathOverflow)?;

    // Mint the shares, signed by the fund PDA (the shares mint's sole authority).
    // The seeds reconstruct that PDA so the *program*, not any caller, authorizes
    // the mint -- re-deriving from the stored canonical bump keeps it the same
    // address the `mint::authority = fund` constraint validated.
    let bump = ctx.accounts.fund.fund_bump;
    let manager = ctx.accounts.fund.manager;
    let name = ctx.accounts.fund.name;
    let seeds: &[&[u8]] = &[b"fund", manager.as_ref(), name.as_ref(), &[bump]];
    token::mint_to(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            MintTo {
                mint: ctx.accounts.shares_mint.to_account_info(),
                to: ctx.accounts.investor_shares_ata.to_account_info(),
                authority: ctx.accounts.fund.to_account_info(),
            },
            &[seeds],
        ),
        shares_out,
    )?;

    Ok(())
}

/// Shares minted for a deposit of `amount` quote tokens into a vault that holds
/// `aum_before` quote and has `supply` shares outstanding.
///
/// The first deposit (`supply == 0`) anchors share price at 1:1 -- any quote
/// donated into the vault beforehand simply accrues to the first depositor,
/// who dilutes no one. Outstanding shares against an empty vault is an
/// invariant break (it would let a depositor buy in at a fake 1:1 price and
/// dilute every holder), so it is rejected rather than priced. Otherwise
/// shares are pro-rata to the pre-deposit AUM, rounded down -- and a zero
/// result is rejected so a dust deposit can't take tokens for no shares.
///
/// This whole function -- including the `supply == 0` special case -- is
/// superseded by the ADR 0001 virtual-offset formula over `total_assets`
/// once that migration lands (see adrs/0001-donation-resistant-share-pricing.md
/// and the SPEC deposit section).
fn shares_for_deposit(amount: u64, supply: u64, aum_before: u64) -> Result<u64> {
    let shares = if supply == 0 {
        // First deposit anchors price at 1:1 regardless of any quote already in
        // the vault: with no holders there is no one to dilute, so a prior
        // donation just accrues to this depositor. Rejecting it instead would let
        // a 1-lamport donation brick the fund before anyone could deposit.
        amount
    } else if aum_before == 0 {
        return Err(FundError::EmptyVaultWithShares.into());
    } else {
        // Widen to u128 *before* multiplying so the product never overflows: two
        // u64s always fit in u128, so a plain `*` is correct here and a
        // `checked_mul` would be dead code (its error arm is unreachable). The
        // only reachable overflow is narrowing the u128 quotient back to u64,
        // which the `try_from` below catches -- that single guard is the real one.
        let scaled = u128::from(amount) * u128::from(supply);
        u64::try_from(scaled / u128::from(aum_before)).map_err(|_| FundError::MathOverflow)?
    };
    // Reject a dust deposit that rounds down to zero shares: the quote has already
    // been (or is about to be) transferred in, so minting zero would take the
    // investor's tokens for nothing.
    require!(shares > 0, FundError::ZeroShares);
    Ok(shares)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_deposit_mints_one_to_one() {
        assert_eq!(shares_for_deposit(1_000, 0, 0).unwrap(), 1_000);
    }

    #[test]
    fn first_deposit_into_a_donated_vault_still_mints_one_to_one() {
        // A donation before the first deposit cannot dilute anyone (there are
        // no holders); rejecting it would let a 1-lamport donation brick the
        // fund forever.
        assert_eq!(shares_for_deposit(1_000, 0, 500).unwrap(), 1_000);
    }

    #[test]
    fn outstanding_shares_with_an_empty_vault_are_rejected() {
        assert_eq!(
            shares_for_deposit(1_000, 1_000, 0),
            Err(FundError::EmptyVaultWithShares.into())
        );
    }

    #[test]
    fn proportional_deposit_mints_pro_rata() {
        // vault holds 1000 quote against 1000 shares; depositing 500 -> 500.
        assert_eq!(shares_for_deposit(500, 1_000, 1_000).unwrap(), 500);
    }

    #[test]
    fn shares_round_down() {
        // 100 * 3 / 7 = 42.85... -> 42
        assert_eq!(shares_for_deposit(100, 3, 7).unwrap(), 42);
    }

    #[test]
    fn dust_deposit_rounding_to_zero_shares_is_rejected() {
        // 1 * 1 / 1000 = 0 -> rejected, otherwise tokens are taken for nothing
        assert_eq!(
            shares_for_deposit(1, 1, 1_000),
            Err(FundError::ZeroShares.into())
        );
    }

    #[test]
    fn result_exceeding_u64_is_rejected() {
        assert_eq!(
            shares_for_deposit(u64::MAX, u64::MAX, 1),
            Err(FundError::MathOverflow.into())
        );
    }
}
