use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct Initialize {}

pub fn handler(ctx: Context<Initialize>) -> Result<()> {
    msg!("Yo, is {:?} ready to make some shmoney?", ctx.program_id);

    Ok(())
}
