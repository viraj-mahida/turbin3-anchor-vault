use crate::{
    constants::{STATE, VAULT_SEED},
    error::ErrorCode,
    state::VaultState,
};
use anchor_lang::{
    prelude::*,
    system_program::{transfer, Transfer},
};

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        seeds = [STATE, user.key().as_ref()],
        bump = vault_state.state_bump
    )]
    pub vault_state: Account<'info, VaultState>,

    #[account(
        mut,
        seeds = [VAULT_SEED, user.key().as_ref()],
        bump = vault_state.vault_bump
    )]
    pub vault: SystemAccount<'info>,
    pub system_program: Program<'info, System>,
}

impl<'info> Withdraw<'info> {
    pub fn withdraw(&mut self, amount: u64) -> Result<()> {
        require!(amount > 0, ErrorCode::InvalidAmount);

        // The vault must always keep enough lamports to stay rent-exempt,
        // so only the balance above that floor can be withdrawn.
        let rent_exempt = Rent::get()?.minimum_balance(self.vault.data_len());
        let available = self
            .vault
            .lamports()
            .checked_sub(rent_exempt)
            .ok_or(ErrorCode::InsufficientFunds)?;
        require!(amount <= available, ErrorCode::InsufficientFunds);

        let cpi_program = self.system_program.key();

        let cpi_accounts = Transfer {
            from: self.vault.to_account_info(),
            to: self.user.to_account_info(),
        };

        let user_key = self.user.key();
        let seeds: &[&[u8]] = &[
            VAULT_SEED,
            user_key.as_ref(),
            &[self.vault_state.vault_bump],
        ];
        let signer_seeds = &[seeds];

        let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer_seeds);

        transfer(cpi_ctx, amount)
    }
}
