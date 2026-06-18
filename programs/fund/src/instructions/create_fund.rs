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
    use proptest::prelude::*;

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

    // --- Property-based tests --------------------------------------------
    //
    // The example tests above pin specific cases; the proptests below assert
    // the same invariants across the whole input space, so a regression that
    // only shows up for some untried fee/name/bump combination still fails.

    /// A canonical, non-empty name: 1..=32 non-zero bytes right-padded with
    /// zeros — exactly the shape `validate_params` is meant to accept.
    fn canonical_name() -> impl Strategy<Value = [u8; 32]> {
        prop::collection::vec(1u8..=u8::MAX, 1..=32).prop_map(|bytes| {
            let mut name = [0u8; 32];
            name[..bytes.len()].copy_from_slice(&bytes);
            name
        })
    }

    /// A non-canonical name: a non-zero byte placed somewhere after a zero
    /// byte, so the encoding is not canonically zero-padded.
    fn non_canonical_name() -> impl Strategy<Value = [u8; 32]> {
        (any::<[u8; 32]>(), 0usize..31).prop_flat_map(|(base, zero_at)| {
            ((zero_at + 1)..32, 1u8..=u8::MAX).prop_map(move |(nonzero_at, byte)| {
                let mut name = base;
                name[zero_at] = 0;
                name[nonzero_at] = byte;
                name
            })
        })
    }

    /// Fully arbitrary parameters (any fee, any name, any caps/delays).
    fn any_params() -> impl Strategy<Value = CreateFundParams> {
        (
            any::<[u8; 32]>(),
            any::<u16>(),
            any::<u16>(),
            any::<u64>(),
            any::<u16>(),
        )
            .prop_map(
                |(
                    name,
                    management_fee_bps,
                    performance_fee_bps,
                    capacity,
                    withdrawal_delay_days,
                )| {
                    CreateFundParams {
                        name,
                        management_fee_bps,
                        performance_fee_bps,
                        capacity,
                        withdrawal_delay_days,
                    }
                },
            )
    }

    proptest! {
        /// `build_fund` is a total field-preserving map: every parameter and
        /// account key is copied through unchanged and each bump lands in its
        /// own field, for any input.
        #[test]
        fn build_fund_preserves_all_fields(
            manager_bytes in any::<[u8; 32]>(),
            quote_bytes in any::<[u8; 32]>(),
            params in any_params(),
            fund_bump in any::<u8>(),
            vault_bump in any::<u8>(),
            shares_mint_bump in any::<u8>(),
        ) {
            let manager = Pubkey::new_from_array(manager_bytes);
            let quote_mint = Pubkey::new_from_array(quote_bytes);
            let fund = build_fund(
                manager,
                quote_mint,
                &params,
                CreateFundBumps {
                    fund: fund_bump,
                    vault: vault_bump,
                    shares_mint: shares_mint_bump,
                },
            );
            prop_assert_eq!(fund.manager, manager);
            prop_assert_eq!(fund.quote_mint, quote_mint);
            prop_assert_eq!(fund.name, params.name);
            prop_assert_eq!(fund.management_fee_bps, params.management_fee_bps);
            prop_assert_eq!(fund.performance_fee_bps, params.performance_fee_bps);
            prop_assert_eq!(fund.capacity, params.capacity);
            prop_assert_eq!(fund.withdrawal_delay_days, params.withdrawal_delay_days);
            prop_assert_eq!(fund.fund_bump, fund_bump);
            prop_assert_eq!(fund.vault_bump, vault_bump);
            prop_assert_eq!(fund.shares_mint_bump, shares_mint_bump);
        }

        /// Any in-range fees combined with a canonical, non-empty name are
        /// accepted, whatever the capacity or withdrawal delay.
        #[test]
        fn validate_accepts_in_range_fees_and_canonical_names(
            name in canonical_name(),
            management_fee_bps in 0u16..=MAX_FEE_BPS,
            performance_fee_bps in 0u16..=MAX_FEE_BPS,
            capacity in any::<u64>(),
            withdrawal_delay_days in any::<u16>(),
        ) {
            let params = CreateFundParams {
                name,
                management_fee_bps,
                performance_fee_bps,
                capacity,
                withdrawal_delay_days,
            };
            prop_assert!(validate_params(&params).is_ok());
        }

        /// A management fee above 100% is always rejected with `FeeTooHigh`,
        /// regardless of the otherwise-valid parameters.
        #[test]
        fn validate_rejects_excessive_management_fee(
            name in canonical_name(),
            management_fee_bps in (MAX_FEE_BPS + 1)..=u16::MAX,
            performance_fee_bps in 0u16..=MAX_FEE_BPS,
        ) {
            let params = CreateFundParams {
                name,
                management_fee_bps,
                performance_fee_bps,
                capacity: 0,
                withdrawal_delay_days: 0,
            };
            prop_assert_eq!(validate_params(&params), Err(FundError::FeeTooHigh.into()));
        }

        /// A performance fee above 100% is always rejected with `FeeTooHigh`.
        #[test]
        fn validate_rejects_excessive_performance_fee(
            name in canonical_name(),
            management_fee_bps in 0u16..=MAX_FEE_BPS,
            performance_fee_bps in (MAX_FEE_BPS + 1)..=u16::MAX,
        ) {
            let params = CreateFundParams {
                name,
                management_fee_bps,
                performance_fee_bps,
                capacity: 0,
                withdrawal_delay_days: 0,
            };
            prop_assert_eq!(validate_params(&params), Err(FundError::FeeTooHigh.into()));
        }

        /// A name carrying a non-zero byte after a zero byte is never canonical
        /// and is always rejected with `NonCanonicalName`, even with valid fees.
        #[test]
        fn validate_rejects_non_canonical_names(
            name in non_canonical_name(),
            management_fee_bps in 0u16..=MAX_FEE_BPS,
            performance_fee_bps in 0u16..=MAX_FEE_BPS,
        ) {
            prop_assert!(!is_canonically_padded(&name));
            let params = CreateFundParams {
                name,
                management_fee_bps,
                performance_fee_bps,
                capacity: 0,
                withdrawal_delay_days: 0,
            };
            prop_assert_eq!(
                validate_params(&params),
                Err(FundError::NonCanonicalName.into())
            );
        }

        /// `is_canonically_padded` matches the structural definition computed
        /// independently: a name is canonical iff every byte from the first
        /// zero onward is also zero.
        #[test]
        fn canonical_padding_matches_definition(name in any::<[u8; 32]>()) {
            let expected = match name.iter().position(|&byte| byte == 0) {
                None => true,
                Some(first_zero) => name[first_zero..].iter().all(|&byte| byte == 0),
            };
            prop_assert_eq!(is_canonically_padded(&name), expected);
        }
    }
}
