use anchor_lang::prelude::*;

/// Parameters supplied by the manager to `create_fund`.
///
/// `name` is part of the Fund PDA seeds, so it must be a fixed-size byte
/// slice (PDA seeds are limited to 32 bytes each). Clients pad shorter
/// human-readable names with trailing zeros.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
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
    /// Running internal accounting of the fund's quote-denominated assets under
    /// management: the quote the program recognizes as fund-owned. Initialized
    /// to 0 at creation, raised by `deposit` (`+= amount`) and lowered by
    /// withdrawal. This -- never the manipulable `vault.amount` -- is the
    /// donation-resistant basis for share pricing: a direct transfer into the
    /// vault raises `vault.amount` but not `total_assets`, so it cannot move the
    /// price (ADR 0001; see the SPEC deposit Share math).
    pub total_assets: u64,
}
