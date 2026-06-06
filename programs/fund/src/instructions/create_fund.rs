use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::error::FundError;
use crate::state::{CreateFundParams, Fund};

/// Maximum fee, in basis points (100%).
const MAX_FEE_BPS: u16 = 10_000;

#[derive(Accounts)]
#[instruction(params: CreateFundParams)]
pub struct CreateFund<'info> {
    #[account(mut)]
    pub manager: Signer<'info>,

    #[account(
        init,
        payer = manager,
        space = 8 + Fund::INIT_SPACE,
        seeds = [b"fund", manager.key().as_ref(), &params.name],
        bump,
    )]
    pub fund: Account<'info, Fund>,

    pub quote_mint: Account<'info, Mint>,

    #[account(
        init,
        payer = manager,
        seeds = [b"vault", fund.key().as_ref()],
        bump,
        token::mint = quote_mint,
        token::authority = fund,
    )]
    pub vault: Account<'info, TokenAccount>,

    #[account(
        init,
        payer = manager,
        seeds = [b"shares", fund.key().as_ref()],
        bump,
        mint::decimals = quote_mint.decimals,
        mint::authority = fund,
    )]
    pub shares_mint: Account<'info, Mint>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}

pub fn handler(ctx: Context<CreateFund>, params: CreateFundParams) -> Result<()> {
    // Anchor's `init` CPIs (fund, vault, shares_mint) have already run by the
    // time this body executes, so the Fund PDA was derived from the raw
    // `params.name` bytes before validation. That is safe because a rejection
    // here reverts the whole transaction atomically — no account survives —
    // but it means `fund.name` is only guaranteed canonical as long as every
    // path that writes it routes through `validate_params`.
    validate_params(&params)?;
    let fund = build_fund(
        ctx.accounts.manager.key(),
        ctx.accounts.quote_mint.key(),
        &params,
        ctx.bumps,
    );
    ctx.accounts.fund.set_inner(fund);
    Ok(())
}

/// Rejects manager-supplied parameters that would create an invalid fund: a fee
/// above 100%, an all-zero (empty) `name`, or a `name` that is not canonically
/// zero-padded (which would let two byte encodings render as the same
/// human-readable name).
fn validate_params(params: &CreateFundParams) -> Result<()> {
    require!(
        params.management_fee_bps <= MAX_FEE_BPS,
        FundError::FeeTooHigh
    );
    require!(
        params.performance_fee_bps <= MAX_FEE_BPS,
        FundError::FeeTooHigh
    );
    require!(
        params.name.iter().any(|&byte| byte != 0),
        FundError::NonCanonicalName
    );
    require!(
        is_canonically_padded(&params.name),
        FundError::NonCanonicalName
    );
    Ok(())
}

/// A name is canonically padded when no non-zero byte follows a zero byte, so
/// distinct byte encodings cannot trim to the same displayed name.
fn is_canonically_padded(name: &[u8; 32]) -> bool {
    name.iter()
        .skip_while(|&&byte| byte != 0)
        .all(|&byte| byte == 0)
}

/// Pure constructor for the `Fund` state from validated parameters and the
/// PDA bumps Anchor derived. Using struct-literal construction means adding a
/// field to `Fund` without deciding its value here is a compile error, and the
/// unit tests below can exercise it without an Anchor `Context`.
fn build_fund(
    manager: Pubkey,
    quote_mint: Pubkey,
    params: &CreateFundParams,
    bumps: CreateFundBumps,
) -> Fund {
    Fund {
        manager,
        name: params.name,
        quote_mint,
        management_fee_bps: params.management_fee_bps,
        performance_fee_bps: params.performance_fee_bps,
        capacity: params.capacity,
        withdrawal_delay_days: params.withdrawal_delay_days,
        fund_bump: bumps.fund,
        vault_bump: bumps.vault,
        shares_mint_bump: bumps.shares_mint,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a 32-byte name from `text`, zero-padded on the right.
    fn padded_name(text: &[u8]) -> [u8; 32] {
        let mut name = [0u8; 32];
        name[..text.len()].copy_from_slice(text);
        name
    }

    /// Sample params reused across the unit tests below.
    fn sample_params() -> CreateFundParams {
        CreateFundParams {
            name: padded_name(b"test-fund-1"),
            management_fee_bps: 200,
            performance_fee_bps: 2_000,
            capacity: 1_000_000_000_000,
            withdrawal_delay_days: 7,
        }
    }

    #[test]
    fn build_fund_copies_every_field_and_maps_bumps() {
        let manager = Pubkey::new_unique();
        let quote_mint = Pubkey::new_unique();
        let params = sample_params();
        let fund = build_fund(
            manager,
            quote_mint,
            &params,
            CreateFundBumps {
                fund: 200,
                vault: 100,
                shares_mint: 50,
            },
        );
        assert_eq!(fund.manager, manager);
        assert_eq!(fund.quote_mint, quote_mint);
        assert_eq!(fund.name, params.name);
        assert_eq!(fund.management_fee_bps, params.management_fee_bps);
        assert_eq!(fund.performance_fee_bps, params.performance_fee_bps);
        assert_eq!(fund.capacity, params.capacity);
        assert_eq!(fund.withdrawal_delay_days, params.withdrawal_delay_days);
        assert_eq!(fund.fund_bump, 200);
        assert_eq!(fund.vault_bump, 100);
        assert_eq!(fund.shares_mint_bump, 50);
    }

    #[test]
    fn validate_params_accepts_in_range_fees_and_canonical_names() {
        assert!(validate_params(&sample_params()).is_ok());

        let mut full_name = sample_params();
        full_name.name = [b'x'; 32];
        assert!(validate_params(&full_name).is_ok());
    }

    #[test]
    fn validate_params_rejects_management_fee_above_one_hundred_percent() {
        let mut params = sample_params();
        params.management_fee_bps = MAX_FEE_BPS + 1;
        assert_eq!(validate_params(&params), Err(FundError::FeeTooHigh.into()));
    }

    #[test]
    fn validate_params_rejects_performance_fee_above_one_hundred_percent() {
        let mut params = sample_params();
        params.performance_fee_bps = MAX_FEE_BPS + 1;
        assert_eq!(validate_params(&params), Err(FundError::FeeTooHigh.into()));
    }

    #[test]
    fn validate_params_rejects_non_canonical_names() {
        let mut gap = sample_params();
        gap.name = [0u8; 32];
        gap.name[0] = b'a';
        gap.name[2] = b'b';
        assert_eq!(
            validate_params(&gap),
            Err(FundError::NonCanonicalName.into())
        );

        let mut trailing = sample_params();
        trailing.name = padded_name(b"abc");
        trailing.name[31] = b'x';
        assert_eq!(
            validate_params(&trailing),
            Err(FundError::NonCanonicalName.into())
        );
    }

    #[test]
    fn validate_params_rejects_an_all_zero_name() {
        let mut params = sample_params();
        params.name = [0u8; 32];
        assert_eq!(
            validate_params(&params),
            Err(FundError::NonCanonicalName.into())
        );
    }
}
