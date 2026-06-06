use anchor_lang::prelude::*;

#[error_code]
pub enum FundError {
    #[msg("fee exceeds the maximum of 10000 bps (100%)")]
    FeeTooHigh,
    #[msg("name must be non-empty and canonically zero-padded")]
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
