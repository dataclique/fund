use anchor_lang::prelude::*;

/// Parameters supplied by the manager to `create_fund`.
///
/// `name` is part of the Fund PDA seeds, so it must be a fixed-size byte
/// slice (PDA seeds are limited to 32 bytes each). Clients pad shorter
/// human-readable names with trailing zeros.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, InitSpace)]
pub struct CreateFundParams {
    pub name: [u8; 32],
    pub management_fee_bps: u16,
    pub performance_fee_bps: u16,
    pub capacity: u64,
    pub withdrawal_delay_days: u16,
}

/// On-chain state for a single fund. One per manager + name pair.
///
/// The bumps for the Vault and SharesMint PDAs are cached on the Fund so
/// subsequent instructions don't have to re-derive them.
#[account]
#[derive(InitSpace)]
pub struct Fund {
    pub manager: Pubkey,
    pub name: [u8; 32],
    pub quote_mint: Pubkey,
    pub management_fee_bps: u16,
    pub performance_fee_bps: u16,
    pub capacity: u64,
    pub withdrawal_delay_days: u16,
    pub fund_bump: u8,
    pub vault_bump: u8,
    pub shares_mint_bump: u8,
}
