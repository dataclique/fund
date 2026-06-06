use anchor_lang::prelude::*;

#[error_code]
pub enum FundError {
    #[msg("fee exceeds the maximum of 10000 bps (100%)")]
    FeeTooHigh,
    #[msg("name must be non-empty and canonically zero-padded")]
    NonCanonicalName,
}
