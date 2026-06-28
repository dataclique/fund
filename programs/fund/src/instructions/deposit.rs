use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, MintTo, Token, TokenAccount, Transfer};

use crate::error::FundError;
use crate::state::Fund;

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub investor: Signer<'info>,

    #[account(
        seeds = [b"fund", fund.manager.as_ref(), &fund.name],
        bump = fund.fund_bump,
    )]
    pub fund: Account<'info, Fund>,

    #[account(
        mut,
        seeds = [b"vault", fund.key().as_ref()],
        bump = fund.vault_bump,
        token::mint = fund.quote_mint,
        token::authority = fund,
    )]
    pub vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"shares", fund.key().as_ref()],
        bump = fund.shares_mint_bump,
        mint::authority = fund,
    )]
    pub shares_mint: Account<'info, Mint>,

    #[account(
        mut,
        associated_token::mint = fund.quote_mint,
        associated_token::authority = investor,
    )]
    pub investor_quote_ata: Account<'info, TokenAccount>,

    // `init_if_needed` is safe here despite the documented footgun: the
    // associated_token constraints pin any pre-existing account to the
    // canonical ATA for (investor, shares_mint), so an attacker cannot
    // substitute an account with a different mint or authority — the only
    // accepted pre-existing state is the investor's own legitimate ATA.
    #[account(
        init_if_needed,
        payer = investor,
        associated_token::mint = shares_mint,
        associated_token::authority = investor,
    )]
    pub investor_shares_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<Deposit>, amount: u64) -> Result<()> {
    require!(amount > 0, FundError::ZeroDeposit);

    // AUM before the inbound transfer drives the share price.
    let aum_before = ctx.accounts.vault.amount;
    let projected_aum = aum_before
        .checked_add(amount)
        .ok_or(FundError::MathOverflow)?;
    require!(
        projected_aum <= ctx.accounts.fund.capacity,
        FundError::CapacityExceeded
    );

    let shares_out = shares_for_deposit(amount, ctx.accounts.shares_mint.supply, aum_before)?;

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
/// The first deposit (`supply == 0`) anchors share price at 1:1 — any quote
/// donated into the vault beforehand simply accrues to the first depositor,
/// who dilutes no one. Outstanding shares against an empty vault is an
/// invariant break (it would let a depositor buy in at a fake 1:1 price and
/// dilute every holder), so it is rejected rather than priced. Otherwise
/// shares are pro-rata to the pre-deposit AUM, rounded down — and a zero
/// result is rejected so a dust deposit can't take tokens for no shares.
///
/// This whole function — including the `supply == 0` special case — is
/// superseded by the ADR 0001 virtual-offset formula over `total_assets`
/// once that migration lands (see adrs/0001-donation-resistant-share-pricing.md
/// and the SPEC deposit section).
fn shares_for_deposit(amount: u64, supply: u64, aum_before: u64) -> Result<u64> {
    let shares = if supply == 0 {
        amount
    } else if aum_before == 0 {
        return Err(FundError::EmptyVaultWithShares.into());
    } else {
        let scaled = u128::from(amount)
            .checked_mul(u128::from(supply))
            .ok_or(FundError::MathOverflow)?;
        u64::try_from(scaled / u128::from(aum_before)).map_err(|_| FundError::MathOverflow)?
    };
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
