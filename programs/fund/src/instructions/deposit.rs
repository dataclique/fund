//! Deposit: an investor swaps quote tokens for freshly-minted fund shares.
//!
//! The steps are deliberately ordered -- read the pre-deposit accounting, move
//! quote in, mint shares, then raise `total_assets` -- so the price is struck
//! against `total_assets` and `supply` as they stand *before* this deposit. The
//! depositor's own `amount` must not be counted in the basis it is buying into:
//! pricing against the post-update `total_assets` would put `amount` in the
//! denominator of the share formula and mint them too *few* shares, diluting the
//! incoming depositor in favor of existing holders.
//!
//! ## Pricing basis (donation-resistant internal accounting, ADR 0001)
//!
//! Shares are priced off `fund.total_assets` -- the quote the program accounts as
//! fund-owned -- and `shares_mint.supply`, and **never** off `vault.amount`. A
//! direct token transfer into the vault raises `vault.amount` but not
//! `total_assets`, so it cannot move the share price: that is what closes the
//! classic ERC-4626 first-depositor inflation attack. The price applies a virtual
//! offset `(V_ASSETS, V_SHARES) = (1, 1)` (offset 0): internal accounting is the
//! *complete* donation defense, so the offset's only job is to keep the
//! first-deposit denominator non-zero, and keeping `V_SHARES = 1` preserves 1:1
//! first-deposit pricing and the shares-decimals == quote-decimals invariant. See
//! `adrs/0001-donation-resistant-share-pricing.md` and the SPEC deposit Share
//! math.
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

    /// The fund being deposited into. `mut` because the deposit raises
    /// `total_assets` by `amount` after the share math. Re-derived from its
    /// *stored* canonical bump (never a caller-supplied bump) so a non-canonical
    /// look-alike PDA cannot be substituted; `Account<Fund>` additionally enforces
    /// program ownership and the 8-byte discriminator (bump-canonicalization +
    /// owner + type defenses).
    #[account(
        mut,
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

    // Price against internal accounting, never `vault.amount`: a direct donation
    // raises the vault balance but not `total_assets`, so it cannot move the price
    // or the capacity check (ADR 0001 donation resistance). This is the
    // pre-deposit value; `total_assets` is raised only after the math and the mint
    // succeed (below).
    let total_assets = ctx.accounts.fund.total_assets;

    // Enforce capacity against *projected* accounted AUM (`total_assets + amount`)
    // so this deposit cannot push the fund past its ceiling. Adding `amount` is
    // exact because the classic SPL Token program has no transfer fee, so the
    // vault receives exactly `amount` and the `total_assets += amount` update
    // below stays truthful. (A Token-2022 transfer-fee mint would land *less* than
    // `amount`; `token_program` is pinned to classic Token, so that cannot arise
    // here -- revisit if Token-2022 support is added.)
    let projected_assets = total_assets
        .checked_add(amount)
        .ok_or(FundError::MathOverflow)?;
    require!(
        projected_assets <= ctx.accounts.fund.capacity,
        FundError::CapacityExceeded
    );

    // Args in order: quote deposited, shares outstanding, accounted assets before.
    // All three are bare `u64`, so the compiler cannot catch a transposed call --
    // keep this site and the signature in lockstep.
    let shares_out = shares_for_deposit(amount, ctx.accounts.shares_mint.supply, total_assets)?;

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

    // Raise internal accounting last -- after the quote has moved and the shares
    // are minted. This is the *only* thing that moves `total_assets`; a vault
    // donation cannot, which is precisely what makes the price donation-resistant.
    ctx.accounts.fund.total_assets = projected_assets;

    Ok(())
}

/// Virtual asset/share offsets for donation-resistant pricing (ADR 0001).
/// Internal `total_assets` accounting is the *complete* donation defense, so the
/// offset's only remaining job is to keep the first-deposit denominator non-zero.
/// `(V_ASSETS, V_SHARES) = (1, 1)` (offset 0) does that while keeping the first
/// deposit at 1:1 and the shares-mint decimals equal to the quote mint's; a
/// larger `V_SHARES` would force a decimals bump for no benefit. `u128` because
/// they are only ever used inside the widened share-math product.
const V_ASSETS: u128 = 1;
const V_SHARES: u128 = 1;

/// Shares minted for a deposit of `amount` quote tokens into a fund that accounts
/// `total_assets` quote internally (the donation-resistant basis -- *not*
/// `vault.amount`) and has `supply` shares outstanding.
///
/// Implements the ADR 0001 offset formula
/// `shares_out = floor(amount * (supply + V_SHARES) / (total_assets + V_ASSETS))`.
/// The first deposit (`supply == 0 && total_assets == 0`) mints 1:1. Shares round
/// **down** -- adverse to the depositor, in favor of the pool -- and a zero
/// result is rejected (`ZeroShares`) so a dust deposit cannot take tokens for
/// nothing. `supply > 0 && total_assets == 0` is rejected
/// (`EmptyVaultWithShares`): deposits and withdrawals keep `total_assets` and
/// `supply` in lockstep, so a zero basis under outstanding shares is corruption,
/// and with the offset it would otherwise over-mint `amount * (supply + 1)`.
fn shares_for_deposit(amount: u64, supply: u64, total_assets: u64) -> Result<u64> {
    require!(
        supply == 0 || total_assets > 0,
        FundError::EmptyVaultWithShares
    );

    // u128 intermediate so the product cannot overflow: `amount <= u64::MAX` and
    // `supply + V_SHARES <= 2^64`, so their product is at most
    // `(2^64 - 1) * 2^64 < u128::MAX`. The only reachable overflow is narrowing
    // the quotient back to u64 (the share count outgrowing u64 as the fund fills),
    // which the `try_from` catches -- that single guard is the real one.
    let numerator = u128::from(amount) * (u128::from(supply) + V_SHARES);
    let denominator = u128::from(total_assets) + V_ASSETS;
    let shares = u64::try_from(numerator / denominator).map_err(|_| FundError::MathOverflow)?;

    // Reject a dust deposit that rounds down to zero shares: the quote has been
    // transferred in, so minting zero would take the investor's tokens for
    // nothing.
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
    fn outstanding_shares_against_zero_total_assets_are_rejected() {
        // Shares outstanding against zero accounted assets is corruption (deposits
        // and withdrawals keep total_assets and supply in lockstep); reject rather
        // than over-mint amount * (supply + 1) shares against the offset.
        assert_eq!(
            shares_for_deposit(1_000, 1_000, 0),
            Err(FundError::EmptyVaultWithShares.into())
        );
    }

    #[test]
    fn proportional_deposit_mints_pro_rata() {
        // 1000 accounted assets against 1000 shares; depositing 500 ->
        // floor(500 * (1000 + 1) / (1000 + 1)) = 500.
        assert_eq!(shares_for_deposit(500, 1_000, 1_000).unwrap(), 500);
    }

    #[test]
    fn shares_round_down() {
        // floor(100 * (2 + 1) / (7 + 1)) = floor(300 / 8) = floor(37.5) = 37.
        assert_eq!(shares_for_deposit(100, 2, 7).unwrap(), 37);
    }

    #[test]
    fn dust_deposit_rounding_to_zero_shares_is_rejected() {
        // floor(1 * (1 + 1) / (1000 + 1)) = floor(2 / 1001) = 0 -> rejected,
        // otherwise the investor's tokens are taken for nothing.
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
