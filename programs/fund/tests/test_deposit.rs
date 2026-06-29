use {
    anchor_lang::{
        solana_program::{instruction::Instruction, program_option::COption},
        InstructionData, ToAccountMetas,
    },
    anchor_spl::{associated_token::get_associated_token_address, token::spl_token},
    fund::state::CreateFundParams,
    litesvm::LiteSVM,
    solana_sdk::{
        account::Account,
        instruction::InstructionError,
        message::{Message, VersionedMessage},
        program_pack::Pack,
        pubkey::Pubkey,
        signature::{Keypair, Signer},
        transaction::{TransactionError, VersionedTransaction},
    },
};

const QUOTE_DECIMALS: u8 = 6;
const INVESTOR_QUOTE_BALANCE: u64 = 1_000_000_000;

// Anchor custom error codes: 6000 + variant index in FundError.
const ZERO_DEPOSIT: u32 = 6004;
const CAPACITY_EXCEEDED: u32 = 6005;
const ZERO_SHARES: u32 = 6006;
const EMPTY_VAULT_WITH_SHARES: u32 = 6008;
// Anchor framework error (anchor-lang 0.31 error.rs):
// AccountNotAssociatedTokenAccount, raised by the associated_token
// constraints when the passed account is not the canonical ATA.
const ACCOUNT_NOT_ASSOCIATED_TOKEN_ACCOUNT: u32 = 3014;

#[test]
fn deposit_transfers_quote_and_mints_shares() {
    let mut ctx = setup(1_000_000_000_000);
    let amount = 250_000_000;

    let result = send_deposit(&mut ctx, amount);
    assert!(result.is_ok(), "deposit failed: {result:?}");

    let vault_state = unpack_token(&ctx.svm, &ctx.vault);
    assert_eq!(vault_state.amount, amount);

    let shares_mint_state = unpack_mint(&ctx.svm, &ctx.shares_mint);
    assert_eq!(shares_mint_state.supply, amount); // first deposit is 1:1

    let investor_shares = get_associated_token_address(&ctx.investor.pubkey(), &ctx.shares_mint);
    let shares_state = unpack_token(&ctx.svm, &investor_shares);
    assert_eq!(shares_state.amount, amount);
    assert_eq!(shares_state.owner, ctx.investor.pubkey());

    let investor_quote = get_associated_token_address(&ctx.investor.pubkey(), &ctx.quote_mint);
    let quote_state = unpack_token(&ctx.svm, &investor_quote);
    assert_eq!(quote_state.amount, INVESTOR_QUOTE_BALANCE - amount);
}

#[test]
fn second_deposit_mints_pro_rata_into_the_existing_ata() {
    let mut ctx = setup(1_000_000_000_000);

    let first = send_deposit(&mut ctx, 100_000_000);
    assert!(first.is_ok(), "first deposit failed: {first:?}");

    // Reuses the now-existing shares ATA (the init_if_needed exists-path).
    ctx.svm.expire_blockhash();
    let second = send_deposit(&mut ctx, 50_000_000);
    assert!(second.is_ok(), "second deposit failed: {second:?}");

    // Equal price both times: 150M quote against 150M shares.
    let shares_mint_state = unpack_mint(&ctx.svm, &ctx.shares_mint);
    assert_eq!(shares_mint_state.supply, 150_000_000);
    let vault_state = unpack_token(&ctx.svm, &ctx.vault);
    assert_eq!(vault_state.amount, 150_000_000);

    // The shares must land in the investor's pre-existing ATA (the
    // init_if_needed exists-path), not just inflate the aggregate supply -- a
    // wrong-ATA-derivation bug would leave supply correct but this assertion
    // would catch it.
    let investor_shares = get_associated_token_address(&ctx.investor.pubkey(), &ctx.shares_mint);
    assert_eq!(unpack_token(&ctx.svm, &investor_shares).amount, 150_000_000);
}

#[test]
fn deposit_rejects_zero_amount() {
    let mut ctx = setup(1_000_000_000_000);

    let result = send_deposit(&mut ctx, 0);
    let err = result.expect_err("zero deposit must fail");
    assert_custom_error(&err, ZERO_DEPOSIT);
}

#[test]
fn deposit_rejects_outstanding_shares_against_an_empty_vault() {
    let mut ctx = setup(1_000_000_000_000);

    let first = send_deposit(&mut ctx, 100_000_000);
    assert!(first.is_ok(), "first deposit failed: {first:?}");

    // Drain the vault out-of-band while shares remain outstanding -- the
    // invariant break EmptyVaultWithShares must reject instead of letting
    // the next depositor buy in at a fake 1:1 price.
    set_token_account(&mut ctx.svm, ctx.vault, ctx.quote_mint, ctx.fund_pda, 0);

    ctx.svm.expire_blockhash();
    let second = send_deposit(&mut ctx, 100_000_000);
    let err = second.expect_err("deposit into drained vault must fail");
    assert_custom_error(&err, EMPTY_VAULT_WITH_SHARES);
}

#[test]
fn deposit_rejects_dust_that_rounds_to_zero_shares() {
    let mut ctx = setup(1_000_000_000_000);

    let first = send_deposit(&mut ctx, 1_000);
    assert!(first.is_ok(), "first deposit failed: {first:?}");

    // Inflate AUM far past the share supply so a 1-unit deposit computes
    // floor(1 * 1_000 / 1_000_000_000) == 0 shares.
    set_token_account(
        &mut ctx.svm,
        ctx.vault,
        ctx.quote_mint,
        ctx.fund_pda,
        1_000_000_000,
    );

    ctx.svm.expire_blockhash();
    let dust = send_deposit(&mut ctx, 1);
    let err = dust.expect_err("dust deposit must fail");
    assert_custom_error(&err, ZERO_SHARES);
}

#[test]
fn deposit_rejects_a_substituted_non_canonical_shares_account() {
    let mut ctx = setup(1_000_000_000_000);

    // A valid-looking spl-token account (right mint, right owner) at a
    // NON-canonical address. The associated_token constraints must reject it
    // on the init_if_needed exists-path -- this pins the safety claim the
    // Deposit accounts struct documents.
    let fake_shares_account = Pubkey::new_unique();
    set_token_account(
        &mut ctx.svm,
        fake_shares_account,
        ctx.shares_mint,
        ctx.investor.pubkey(),
        0,
    );

    let result = send_deposit_with_shares_account(&mut ctx, 100_000_000, fake_shares_account);
    let err = result.expect_err("substituted shares account must be rejected");
    assert_custom_error(&err, ACCOUNT_NOT_ASSOCIATED_TOKEN_ACCOUNT);
}

#[test]
fn deposit_rejects_amount_exceeding_capacity() {
    let mut ctx = setup(100_000_000); // tiny capacity

    let result = send_deposit(&mut ctx, 100_000_001);
    let err = result.expect_err("over-capacity deposit must fail");
    assert_custom_error(&err, CAPACITY_EXCEEDED);
    assert_eq!(unpack_token(&ctx.svm, &ctx.vault).amount, 0);
}

#[test]
fn deposit_at_exactly_capacity_succeeds() {
    // The capacity check is `projected_aum <= capacity`, so a deposit that lands
    // AUM *exactly* on the ceiling must be accepted -- capacity is the inclusive
    // limit the fund promises LPs, not an exclusive one. This pins the allowed
    // side of the fence-post: a future tightening of `<=` to `<` would silently
    // break the promise, and only this green boundary test would catch it (the
    // rejection test above fires one unit over and would stay green).
    let mut ctx = setup(100_000_000);

    let result = send_deposit(&mut ctx, 100_000_000);
    assert!(
        result.is_ok(),
        "deposit at exactly capacity failed: {result:?}"
    );

    let vault_state = unpack_token(&ctx.svm, &ctx.vault);
    assert_eq!(vault_state.amount, 100_000_000);
    let shares_mint_state = unpack_mint(&ctx.svm, &ctx.shares_mint);
    assert_eq!(shares_mint_state.supply, 100_000_000); // first deposit is 1:1
}

struct TestContext {
    svm: LiteSVM,
    investor: Keypair,
    quote_mint: Pubkey,
    fund_pda: Pubkey,
    vault: Pubkey,
    shares_mint: Pubkey,
}

/// Boots an SVM, creates a fund with the given capacity via `create_fund`,
/// and funds an investor with quote tokens in their ATA.
fn setup(capacity: u64) -> TestContext {
    let mut svm = LiteSVM::new();
    svm.add_program(fund::id(), include_bytes!("../../../target/deploy/fund.so"));

    let manager = Keypair::new();
    svm.airdrop(&manager.pubkey(), 10_000_000_000).unwrap();
    let investor = Keypair::new();
    svm.airdrop(&investor.pubkey(), 10_000_000_000).unwrap();

    let quote_mint = Pubkey::new_unique();
    set_mint(&mut svm, quote_mint, 0);

    let mut name = [0u8; 32];
    name[..b"deposit-fund".len()].copy_from_slice(b"deposit-fund");
    let params = CreateFundParams {
        name,
        management_fee_bps: 200,
        performance_fee_bps: 2_000,
        capacity,
        withdrawal_delay_days: 7,
    };

    let (fund_pda, _) =
        Pubkey::find_program_address(&[b"fund", manager.pubkey().as_ref(), &name], &fund::id());
    let (vault, _) = Pubkey::find_program_address(&[b"vault", fund_pda.as_ref()], &fund::id());
    let (shares_mint, _) =
        Pubkey::find_program_address(&[b"shares", fund_pda.as_ref()], &fund::id());

    let create = Instruction::new_with_bytes(
        fund::id(),
        &fund::instruction::CreateFund { params }.data(),
        fund::accounts::CreateFund {
            manager: manager.pubkey(),
            fund: fund_pda,
            quote_mint,
            vault,
            shares_mint,
            system_program: solana_sdk::system_program::id(),
            token_program: spl_token::id(),
        }
        .to_account_metas(None),
    );
    send(&mut svm, &[create], &manager).expect("create_fund setup failed");

    // Materialize the investor's quote ATA with a balance, owned by the
    // SPL token program at the canonical ATA address.
    let investor_quote_ata = get_associated_token_address(&investor.pubkey(), &quote_mint);
    set_token_account(
        &mut svm,
        investor_quote_ata,
        quote_mint,
        investor.pubkey(),
        INVESTOR_QUOTE_BALANCE,
    );
    set_mint(&mut svm, quote_mint, INVESTOR_QUOTE_BALANCE);

    TestContext {
        svm,
        investor,
        quote_mint,
        fund_pda,
        vault,
        shares_mint,
    }
}

fn send_deposit(
    ctx: &mut TestContext,
    amount: u64,
) -> Result<litesvm::types::TransactionMetadata, Box<litesvm::types::FailedTransactionMetadata>> {
    let shares_ata = get_associated_token_address(&ctx.investor.pubkey(), &ctx.shares_mint);
    send_deposit_with_shares_account(ctx, amount, shares_ata)
}

fn send_deposit_with_shares_account(
    ctx: &mut TestContext,
    amount: u64,
    investor_shares_ata: Pubkey,
) -> Result<litesvm::types::TransactionMetadata, Box<litesvm::types::FailedTransactionMetadata>> {
    let instruction = Instruction::new_with_bytes(
        fund::id(),
        &fund::instruction::Deposit { amount }.data(),
        fund::accounts::Deposit {
            investor: ctx.investor.pubkey(),
            fund: ctx.fund_pda,
            vault: ctx.vault,
            shares_mint: ctx.shares_mint,
            investor_quote_ata: get_associated_token_address(
                &ctx.investor.pubkey(),
                &ctx.quote_mint,
            ),
            investor_shares_ata,
            token_program: spl_token::id(),
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: solana_sdk::system_program::id(),
        }
        .to_account_metas(None),
    );
    let TestContext { svm, investor, .. } = ctx;
    send(svm, &[instruction], investor)
}

fn send(
    svm: &mut LiteSVM,
    instructions: &[Instruction],
    payer: &Keypair,
) -> Result<litesvm::types::TransactionMetadata, Box<litesvm::types::FailedTransactionMetadata>> {
    let blockhash = svm.latest_blockhash();
    let message = Message::new_with_blockhash(instructions, Some(&payer.pubkey()), &blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(message), &[payer]).unwrap();
    svm.send_transaction(tx).map_err(Box::new)
}

fn set_mint(svm: &mut LiteSVM, mint: Pubkey, supply: u64) {
    let mut data = vec![0u8; spl_token::state::Mint::LEN];
    spl_token::state::Mint::pack(
        spl_token::state::Mint {
            mint_authority: COption::None,
            supply,
            decimals: QUOTE_DECIMALS,
            is_initialized: true,
            freeze_authority: COption::None,
        },
        &mut data,
    )
    .unwrap();
    svm.set_account(
        mint,
        Account {
            lamports: 1_000_000_000,
            data,
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
}

fn set_token_account(svm: &mut LiteSVM, address: Pubkey, mint: Pubkey, owner: Pubkey, amount: u64) {
    let mut data = vec![0u8; spl_token::state::Account::LEN];
    spl_token::state::Account::pack(
        spl_token::state::Account {
            mint,
            owner,
            amount,
            delegate: COption::None,
            state: spl_token::state::AccountState::Initialized,
            is_native: COption::None,
            delegated_amount: 0,
            close_authority: COption::None,
        },
        &mut data,
    )
    .unwrap();
    svm.set_account(
        address,
        Account {
            lamports: 1_000_000_000,
            data,
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
}

/// Pins a rejection to the exact custom error code, so a test cannot pass
/// because some unrelated failure also produced an instruction error.
fn assert_custom_error(err: &litesvm::types::FailedTransactionMetadata, expected: u32) {
    match err.err {
        TransactionError::InstructionError(_, InstructionError::Custom(code)) => assert_eq!(
            code, expected,
            "wrong rejection code; logs: {:?}",
            err.meta.logs
        ),
        ref other => panic!("expected Custom({expected}), got {other:?}"),
    }
}

fn unpack_token(svm: &LiteSVM, address: &Pubkey) -> spl_token::state::Account {
    spl_token::state::Account::unpack(&svm.get_account(address).unwrap().data).unwrap()
}

fn unpack_mint(svm: &LiteSVM, address: &Pubkey) -> spl_token::state::Mint {
    spl_token::state::Mint::unpack(&svm.get_account(address).unwrap().data).unwrap()
}
