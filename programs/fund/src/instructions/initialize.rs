use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::state::{CreateFundParams, Fund};

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
    pub rent: Sysvar<'info, Rent>,
}

/// PDA bumps cached on the `Fund` account.
///
/// Exists so the pure `apply_params` function can be unit-tested without
/// pulling in Anchor's auto-generated `CreateFundBumps` (which lives
/// inside the `#[derive(Accounts)]` macro expansion).
pub struct FundBumps {
    pub fund: u8,
    pub vault: u8,
    pub shares_mint: u8,
}

/// Pure function that writes the manager-supplied parameters and PDA
/// bumps onto a freshly-allocated `Fund`. Lives outside `handler` so it
/// can be exercised from the unit tests below without constructing an
/// Anchor `Context`.
pub fn apply_params(
    fund: &mut Fund,
    manager: Pubkey,
    quote_mint: Pubkey,
    params: &CreateFundParams,
    bumps: FundBumps,
) {
    // Stub. Implemented in a later step of the create_fund TTDD loop;
    // the unit tests in this file and the `tests/create-fund.ts`
    // integration test both fail until this function actually sets the
    // Fund fields.
    let _ = (fund, manager, quote_mint, params, bumps);
}

pub fn handler(ctx: Context<CreateFund>, params: CreateFundParams) -> Result<()> {
    apply_params(
        &mut ctx.accounts.fund,
        ctx.accounts.manager.key(),
        ctx.accounts.quote_mint.key(),
        &params,
        FundBumps {
            fund: ctx.bumps.fund,
            vault: ctx.bumps.vault,
            shares_mint: ctx.bumps.shares_mint,
        },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sample params reused across the unit tests below.
    fn sample_params() -> CreateFundParams {
        CreateFundParams {
            name: *b"test-fund-1\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
            management_fee_bps: 200,
            performance_fee_bps: 2_000,
            capacity: 1_000_000_000_000,
            withdrawal_delay_days: 7,
        }
    }

    fn empty_fund() -> Fund {
        Fund {
            manager: Pubkey::default(),
            name: [0u8; 32],
            quote_mint: Pubkey::default(),
            management_fee_bps: 0,
            performance_fee_bps: 0,
            capacity: 0,
            withdrawal_delay_days: 0,
            fund_bump: 0,
            vault_bump: 0,
            shares_mint_bump: 0,
        }
    }

    #[test]
    fn apply_params_writes_manager_and_quote_mint() {
        let mut fund = empty_fund();
        let manager = Pubkey::new_unique();
        let quote_mint = Pubkey::new_unique();
        apply_params(
            &mut fund,
            manager,
            quote_mint,
            &sample_params(),
            FundBumps {
                fund: 255,
                vault: 254,
                shares_mint: 253,
            },
        );
        assert_eq!(fund.manager, manager);
        assert_eq!(fund.quote_mint, quote_mint);
    }

    #[test]
    fn apply_params_copies_every_param_field() {
        let mut fund = empty_fund();
        let params = sample_params();
        apply_params(
            &mut fund,
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            &params,
            FundBumps {
                fund: 0,
                vault: 0,
                shares_mint: 0,
            },
        );
        assert_eq!(fund.name, params.name);
        assert_eq!(fund.management_fee_bps, params.management_fee_bps);
        assert_eq!(fund.performance_fee_bps, params.performance_fee_bps);
        assert_eq!(fund.capacity, params.capacity);
        assert_eq!(fund.withdrawal_delay_days, params.withdrawal_delay_days);
    }

    #[test]
    fn apply_params_stores_bumps() {
        let mut fund = empty_fund();
        apply_params(
            &mut fund,
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            &sample_params(),
            FundBumps {
                fund: 255,
                vault: 254,
                shares_mint: 253,
            },
        );
        assert_eq!(fund.fund_bump, 255);
        assert_eq!(fund.vault_bump, 254);
        assert_eq!(fund.shares_mint_bump, 253);
    }
}
