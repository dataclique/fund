use anchor_lang::prelude::*;

#[error_code]
pub enum FundError {
    #[msg("management fee exceeds the maximum of 10000 bps (100%)")]
    ManagementFeeTooHigh,
    #[msg("performance fee exceeds the maximum of 10000 bps (100%)")]
    PerformanceFeeTooHigh,
    #[msg("name must be non-empty")]
    EmptyName,
    #[msg("name must be canonically zero-padded")]
    NonCanonicalName,
}
