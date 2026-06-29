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
    #[msg("deposit amount must be greater than zero")]
    ZeroDeposit,
    #[msg("deposit would exceed the fund capacity")]
    CapacityExceeded,
    #[msg("deposit would mint zero shares")]
    ZeroShares,
    #[msg("arithmetic overflow")]
    MathOverflow,
    #[msg("vault is empty while shares are outstanding")]
    EmptyVaultWithShares,
}
