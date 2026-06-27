use {
    anchor_lang::{
        solana_program::{instruction::Instruction, program_option::COption},
        AccountDeserialize, InstructionData, ToAccountMetas,
    },
    anchor_spl::token::spl_token,
    fund::state::CreateFundParams,
    litesvm::LiteSVM,
    solana_sdk::{
        account::Account,
        message::{Message, VersionedMessage},
        program_pack::Pack,
        pubkey::Pubkey,
        signature::{Keypair, Signer},
        transaction::{TransactionError, VersionedTransaction},
    },
};

const QUOTE_DECIMALS: u8 = 6;

#[test]
fn create_fund_initializes_fund_vault_and_shares_mint() {
    let (mut svm, manager, quote_mint) = setup();
    let params = sample_params(b"momentum-fund");
    let (fund_pda, vault_pda, shares_pda) = fund_pdas(&manager.pubkey(), &params.name);

    let result = send_create_fund(&mut svm, &manager, quote_mint, &params);
    assert!(result.is_ok(), "create_fund failed: {result:?}");

    let fund_account = svm.get_account(&fund_pda).unwrap();
    assert_eq!(fund_account.owner, fund::id());
    let fund_state = fund::state::Fund::try_deserialize(&mut fund_account.data.as_slice()).unwrap();
    assert_eq!(fund_state.manager, manager.pubkey());
    assert_eq!(fund_state.name, params.name);
    assert_eq!(fund_state.quote_mint, quote_mint);
    assert_eq!(fund_state.management_fee_bps, params.management_fee_bps);
    assert_eq!(fund_state.performance_fee_bps, params.performance_fee_bps);
    assert_eq!(fund_state.capacity, params.capacity);
    assert_eq!(
        fund_state.withdrawal_delay_days,
        params.withdrawal_delay_days
    );

    // The stored bumps must reproduce the canonical PDAs, so the Fund can
    // re-sign as vault/shares-mint authority without re-searching.
    assert_eq!(
        Pubkey::create_program_address(
            &[
                b"fund",
                manager.pubkey().as_ref(),
                &params.name,
                &[fund_state.fund_bump],
            ],
            &fund::id(),
        )
        .unwrap(),
        fund_pda
    );
    assert_eq!(
        Pubkey::create_program_address(
            &[b"vault", fund_pda.as_ref(), &[fund_state.vault_bump]],
            &fund::id(),
        )
        .unwrap(),
        vault_pda
    );
    assert_eq!(
        Pubkey::create_program_address(
            &[b"shares", fund_pda.as_ref(), &[fund_state.shares_mint_bump]],
            &fund::id(),
        )
        .unwrap(),
        shares_pda
    );

    let vault_account = svm.get_account(&vault_pda).unwrap();
    assert_eq!(vault_account.owner, spl_token::id());
    let vault_state = spl_token::state::Account::unpack(&vault_account.data).unwrap();
    assert_eq!(vault_state.mint, quote_mint);
    assert_eq!(vault_state.owner, fund_pda);
    assert_eq!(vault_state.amount, 0);

    let shares_account = svm.get_account(&shares_pda).unwrap();
    assert_eq!(shares_account.owner, spl_token::id());
    let shares_state = spl_token::state::Mint::unpack(&shares_account.data).unwrap();
    assert_eq!(shares_state.mint_authority, COption::Some(fund_pda));
    assert_eq!(shares_state.decimals, QUOTE_DECIMALS);
    assert_eq!(shares_state.supply, 0);
}

#[test]
fn create_fund_rejects_duplicate_creation() {
    let (mut svm, manager, quote_mint) = setup();
    let params = sample_params(b"once-only-fund");
    let (fund_pda, _, _) = fund_pdas(&manager.pubkey(), &params.name);

    let first = send_create_fund(&mut svm, &manager, quote_mint, &params);
    assert!(first.is_ok(), "first create_fund failed: {first:?}");
    let fund_before = svm
        .get_account(&fund_pda)
        .expect("fund must exist after the first create")
        .data;

    // Anchor's `init` constraint must reject re-initialization of the same
    // manager + name pair — a re-initializable fund could have its state
    // silently replaced (sealevel-attacks: Initialization). The blockhash
    // expiry is load-bearing: without it litesvm short-circuits the identical
    // transaction with AlreadyProcessed before the program ever runs.
    svm.expire_blockhash();
    let second = send_create_fund(&mut svm, &manager, quote_mint, &params);
    let err = second.expect_err("duplicate create_fund must fail");
    assert!(
        matches!(err.err, TransactionError::InstructionError(..)),
        "expected an on-chain init rejection, got {:?}",
        err.err
    );

    // Rejecting the re-init must leave the original fund byte-identical, not
    // merely fail: a permissive `is_err` would also pass if the second attempt
    // silently overwrote state.
    let fund_after = svm
        .get_account(&fund_pda)
        .expect("fund must remain after the duplicate is rejected")
        .data;
    assert_eq!(fund_after, fund_before, "duplicate path mutated fund state");
}

#[test]
fn create_fund_accepts_fees_at_exactly_one_hundred_percent() {
    let (mut svm, manager, quote_mint) = setup();
    let mut params = sample_params(b"max-fee-fund");
    params.management_fee_bps = 10_000;
    params.performance_fee_bps = 10_000;

    let result = send_create_fund(&mut svm, &manager, quote_mint, &params);
    assert!(result.is_ok(), "boundary fees rejected: {result:?}");
}

#[test]
fn create_fund_rejects_a_fee_above_one_hundred_percent() {
    let (mut svm, manager, quote_mint) = setup();
    let mut params = sample_params(b"greedy-fund");
    params.management_fee_bps = 10_001;

    let result = send_create_fund(&mut svm, &manager, quote_mint, &params);
    assert!(result.is_err());
    let (fund_pda, vault_pda, shares_pda) = fund_pdas(&manager.pubkey(), &params.name);
    assert!(svm.get_account(&fund_pda).is_none());
    assert!(svm.get_account(&vault_pda).is_none());
    assert!(svm.get_account(&shares_pda).is_none());
}

#[test]
fn create_fund_rejects_a_non_canonical_name() {
    let (mut svm, manager, quote_mint) = setup();
    let mut params = sample_params(b"gap");
    params.name[5] = b'x';

    let result = send_create_fund(&mut svm, &manager, quote_mint, &params);
    assert!(result.is_err());

    // Validation runs after Anchor's `init` CPIs, so a rejection must revert
    // the whole transaction atomically — no fund, vault, or shares account is
    // left behind.
    let (fund_pda, vault_pda, shares_pda) = fund_pdas(&manager.pubkey(), &params.name);
    assert!(svm.get_account(&fund_pda).is_none());
    assert!(svm.get_account(&vault_pda).is_none());
    assert!(svm.get_account(&shares_pda).is_none());
}

#[test]
fn create_fund_rejects_an_all_zero_name() {
    let (mut svm, manager, quote_mint) = setup();
    let mut params = sample_params(b"ignored");
    params.name = [0u8; 32];

    let result = send_create_fund(&mut svm, &manager, quote_mint, &params);
    assert!(result.is_err());

    let (fund_pda, vault_pda, shares_pda) = fund_pdas(&manager.pubkey(), &params.name);
    assert!(svm.get_account(&fund_pda).is_none());
    assert!(svm.get_account(&vault_pda).is_none());
    assert!(svm.get_account(&shares_pda).is_none());
}

/// Fresh SVM with the compiled program, a funded manager, and an initialized
/// SPL quote mint (USDC-like, 6 decimals).
fn setup() -> (LiteSVM, Keypair, Pubkey) {
    let mut svm = LiteSVM::new();
    svm.add_program(fund::id(), include_bytes!("../../../target/deploy/fund.so"));

    let manager = Keypair::new();
    svm.airdrop(&manager.pubkey(), 10_000_000_000).unwrap();

    let quote_mint = Pubkey::new_unique();
    let mut mint_data = vec![0u8; spl_token::state::Mint::LEN];
    spl_token::state::Mint::pack(
        spl_token::state::Mint {
            mint_authority: COption::None,
            supply: 0,
            decimals: QUOTE_DECIMALS,
            is_initialized: true,
            freeze_authority: COption::None,
        },
        &mut mint_data,
    )
    .unwrap();
    svm.set_account(
        quote_mint,
        Account {
            lamports: 1_000_000_000,
            data: mint_data,
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();

    (svm, manager, quote_mint)
}

/// Builds the params struct with a right-zero-padded `name`.
fn sample_params(name_text: &[u8]) -> CreateFundParams {
    let mut name = [0u8; 32];
    name[..name_text.len()].copy_from_slice(name_text);
    CreateFundParams {
        name,
        management_fee_bps: 200,
        performance_fee_bps: 2_000,
        capacity: 1_000_000_000_000,
        withdrawal_delay_days: 7,
    }
}

/// Canonical Fund/Vault/SharesMint PDAs for a manager + name pair.
fn fund_pdas(manager: &Pubkey, name: &[u8; 32]) -> (Pubkey, Pubkey, Pubkey) {
    let (fund_pda, _) =
        Pubkey::find_program_address(&[b"fund", manager.as_ref(), name], &fund::id());
    let (vault_pda, _) = Pubkey::find_program_address(&[b"vault", fund_pda.as_ref()], &fund::id());
    let (shares_pda, _) =
        Pubkey::find_program_address(&[b"shares", fund_pda.as_ref()], &fund::id());
    (fund_pda, vault_pda, shares_pda)
}

/// Sends a `create_fund` transaction signed by `manager`.
fn send_create_fund(
    svm: &mut LiteSVM,
    manager: &Keypair,
    quote_mint: Pubkey,
    params: &CreateFundParams,
) -> Result<litesvm::types::TransactionMetadata, Box<litesvm::types::FailedTransactionMetadata>> {
    let (fund_pda, vault_pda, shares_pda) = fund_pdas(&manager.pubkey(), &params.name);

    let instruction = Instruction::new_with_bytes(
        fund::id(),
        &fund::instruction::CreateFund {
            params: params.clone(),
        }
        .data(),
        fund::accounts::CreateFund {
            manager: manager.pubkey(),
            fund: fund_pda,
            quote_mint,
            vault: vault_pda,
            shares_mint: shares_pda,
            system_program: solana_sdk::system_program::id(),
            token_program: spl_token::id(),
        }
        .to_account_metas(None),
    );

    let blockhash = svm.latest_blockhash();
    let message = Message::new_with_blockhash(&[instruction], Some(&manager.pubkey()), &blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(message), &[manager]).unwrap();
    svm.send_transaction(tx).map_err(Box::new)
}
