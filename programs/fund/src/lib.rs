pub mod constants;
pub mod error;
pub mod instructions;
pub mod state;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("8FUTArRdDgGDTYbnBMrzB7mBTwLrYpJDTzWY4d62GpCD");

#[program]
pub mod fund {
    use super::*;

    pub fn create_fund(ctx: Context<CreateFund>, params: CreateFundParams) -> Result<()> {
        create_fund::handler(ctx, params)
    }
}
