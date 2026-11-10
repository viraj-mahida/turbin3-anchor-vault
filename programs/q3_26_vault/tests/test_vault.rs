use {
    anchor_lang::{
        prelude::Pubkey,
        solana_program::{instruction::Instruction, system_program},
        InstructionData, ToAccountMetas,
    },
    litesvm::{types::FailedTransactionMetadata, LiteSVM},
    q3_26_vault::{STATE, VAULT_SEED},
    solana_keypair::Keypair,
    solana_message::{Message, VersionedMessage},
    solana_signer::Signer,
    solana_transaction::versioned::VersionedTransaction,
};

const DEPOSIT_LAMPORTS: u64 = 500_000_000;
const WITHDRAW_LAMPORTS: u64 = 100_000_000;
const AIRDROP_LAMPORTS: u64 = 2_000_000_000;

struct Ctx {
    svm: LiteSVM,
    user: Keypair,
    vault_state: Pubkey,
    vault: Pubkey,
}

fn setup() -> Ctx {
    let program_id = q3_26_vault::id();
    let user = Keypair::new();
    let (vault_state, _) =
        Pubkey::find_program_address(&[STATE, user.pubkey().as_ref()], &program_id);
    let (vault, _) =
        Pubkey::find_program_address(&[VAULT_SEED, user.pubkey().as_ref()], &program_id);

    let mut svm = LiteSVM::new();
    let bytes = include_bytes!(concat!(
        env!("CARGO_TARGET_TMPDIR"),
        "/../deploy/q3_26_vault.so"
    ));
    svm.add_program(program_id, bytes).unwrap();
    svm.airdrop(&user.pubkey(), AIRDROP_LAMPORTS).unwrap();

    Ctx {
        svm,
        user,
        vault_state,
        vault,
    }
}

fn try_send(
    svm: &mut LiteSVM,
    payer: &Keypair,
    ix: Instruction,
) -> Result<(), FailedTransactionMetadata> {
    svm.expire_blockhash();
    let blockhash = svm.latest_blockhash();
    let msg = Message::new_with_blockhash(&[ix], Some(&payer.pubkey()), &blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[payer]).unwrap();
    svm.send_transaction(tx).map(|_| ())
}

fn send(svm: &mut LiteSVM, payer: &Keypair, ix: Instruction) {
    try_send(svm, payer, ix).unwrap_or_else(|err| {
        panic!("tx failed: {err:?}\nlogs: {logs:#?}", logs = err.meta.logs);
    });
}

fn assert_fails_with(result: Result<(), FailedTransactionMetadata>, needle: &str) {
    let err = result.expect_err("transaction should have failed");
    let logs = err.meta.logs.join("\n");
    assert!(
        logs.contains(needle),
        "expected logs to contain `{needle}`, got:\n{logs}"
    );
}

fn initialize_ix(user: Pubkey, vault_state: Pubkey, vault: Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        q3_26_vault::id(),
        &q3_26_vault::instruction::Initialize {}.data(),
        q3_26_vault::accounts::Initialize {
            user,
            vault_state,
            vault,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
    )
}

fn deposit_ix(user: Pubkey, vault_state: Pubkey, vault: Pubkey, amount: u64) -> Instruction {
    Instruction::new_with_bytes(
        q3_26_vault::id(),
        &q3_26_vault::instruction::Deposit { amount }.data(),
        q3_26_vault::accounts::Deposit {
            user,
            vault_state,
            vault,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
    )
}

fn withdraw_ix(user: Pubkey, vault_state: Pubkey, vault: Pubkey, amount: u64) -> Instruction {
    Instruction::new_with_bytes(
        q3_26_vault::id(),
        &q3_26_vault::instruction::Withdraw { amount }.data(),
        q3_26_vault::accounts::Withdraw {
            user,
            vault_state,
            vault,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
    )
}

fn close_ix(user: Pubkey, vault_state: Pubkey, vault: Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        q3_26_vault::id(),
        &q3_26_vault::instruction::Close {}.data(),
        q3_26_vault::accounts::Close {
            user,
            vault_state,
            vault,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
    )
}

fn initialized() -> Ctx {
    let mut c = setup();
    let ix = initialize_ix(c.user.pubkey(), c.vault_state, c.vault);
    send(&mut c.svm, &c.user, ix);
    c
}

fn funded() -> Ctx {
    let mut c = initialized();
    let ix = deposit_ix(c.user.pubkey(), c.vault_state, c.vault, DEPOSIT_LAMPORTS);
    send(&mut c.svm, &c.user, ix);
    c
}

#[test]
fn initialize_creates_state_and_rent_exempt_vault() {
    let c = initialized();
    let rent_exempt = c.svm.minimum_balance_for_rent_exemption(0);

    let state = c
        .svm
        .get_account(&c.vault_state)
        .expect("vault_state should exist");
    assert_eq!(
        state.owner,
        q3_26_vault::id(),
        "state is owned by the program"
    );
    assert_eq!(
        c.svm.get_balance(&c.vault).unwrap(),
        rent_exempt,
        "vault should be funded with exactly the rent-exempt minimum"
    );
}

#[test]
fn initialize_twice_fails() {
    let mut c = initialized();
    let ix = initialize_ix(c.user.pubkey(), c.vault_state, c.vault);
    assert!(try_send(&mut c.svm, &c.user, ix).is_err());
}

#[test]
fn deposit_moves_lamports_into_vault() {
    let c = funded();
    let rent_exempt = c.svm.minimum_balance_for_rent_exemption(0);
    assert_eq!(
        c.svm.get_balance(&c.vault).unwrap(),
        rent_exempt + DEPOSIT_LAMPORTS
    );
}

#[test]
fn deposit_zero_fails() {
    let mut c = initialized();
    let ix = deposit_ix(c.user.pubkey(), c.vault_state, c.vault, 0);
    assert_fails_with(try_send(&mut c.svm, &c.user, ix), "InvalidAmount");
}

#[test]
fn withdraw_moves_lamports_back_to_user() {
    let mut c = funded();
    let rent_exempt = c.svm.minimum_balance_for_rent_exemption(0);
    let user_before = c.svm.get_balance(&c.user.pubkey()).unwrap();

    let ix = withdraw_ix(c.user.pubkey(), c.vault_state, c.vault, WITHDRAW_LAMPORTS);
    send(&mut c.svm, &c.user, ix);

    assert_eq!(
        c.svm.get_balance(&c.vault).unwrap(),
        rent_exempt + DEPOSIT_LAMPORTS - WITHDRAW_LAMPORTS,
        "vault should decrease by the withdraw"
    );
    // The user pays a 5000 lamport signature fee for the withdraw tx.
    assert_eq!(
        c.svm.get_balance(&c.user.pubkey()).unwrap(),
        user_before + WITHDRAW_LAMPORTS - 5_000,
        "user should receive the withdrawn lamports"
    );
}

#[test]
fn withdraw_full_deposit_keeps_vault_rent_exempt() {
    let mut c = funded();
    let rent_exempt = c.svm.minimum_balance_for_rent_exemption(0);
    let ix = withdraw_ix(c.user.pubkey(), c.vault_state, c.vault, DEPOSIT_LAMPORTS);
    send(&mut c.svm, &c.user, ix);
    assert_eq!(c.svm.get_balance(&c.vault).unwrap(), rent_exempt);
}

#[test]
fn withdraw_more_than_deposited_fails() {
    let mut c = funded();
    let ix = withdraw_ix(
        c.user.pubkey(),
        c.vault_state,
        c.vault,
        DEPOSIT_LAMPORTS + 1,
    );
    assert_fails_with(try_send(&mut c.svm, &c.user, ix), "InsufficientFunds");
}

#[test]
fn withdraw_zero_fails() {
    let mut c = funded();
    let ix = withdraw_ix(c.user.pubkey(), c.vault_state, c.vault, 0);
    assert_fails_with(try_send(&mut c.svm, &c.user, ix), "InvalidAmount");
}

#[test]
fn withdraw_from_someone_elses_vault_fails() {
    let mut c = funded();
    let attacker = Keypair::new();
    c.svm.airdrop(&attacker.pubkey(), AIRDROP_LAMPORTS).unwrap();

    // The PDAs are derived from the signer, so the victim's accounts do not
    // match the attacker's seeds.
    let ix = withdraw_ix(attacker.pubkey(), c.vault_state, c.vault, WITHDRAW_LAMPORTS);
    assert_fails_with(try_send(&mut c.svm, &attacker, ix), "ConstraintSeeds");

    let rent_exempt = c.svm.minimum_balance_for_rent_exemption(0);
    assert_eq!(
        c.svm.get_balance(&c.vault).unwrap(),
        rent_exempt + DEPOSIT_LAMPORTS,
        "victim vault is untouched"
    );
}

#[test]
fn close_returns_everything_and_closes_accounts() {
    let mut c = funded();
    let rent_exempt = c.svm.minimum_balance_for_rent_exemption(0);
    let state_rent = c.svm.get_balance(&c.vault_state).unwrap();
    let user_before = c.svm.get_balance(&c.user.pubkey()).unwrap();

    let ix = close_ix(c.user.pubkey(), c.vault_state, c.vault);
    send(&mut c.svm, &c.user, ix);

    assert!(
        c.svm
            .get_account(&c.vault_state)
            .map_or(true, |a| a.lamports == 0),
        "vault_state should be closed"
    );
    assert_eq!(
        c.svm.get_balance(&c.vault).unwrap_or(0),
        0,
        "vault should be empty"
    );
    assert_eq!(
        c.svm.get_balance(&c.user.pubkey()).unwrap(),
        user_before + rent_exempt + DEPOSIT_LAMPORTS + state_rent - 5_000,
        "user gets the deposit plus both rent reserves back"
    );
}

#[test]
fn close_by_someone_else_fails() {
    let mut c = funded();
    let attacker = Keypair::new();
    c.svm.airdrop(&attacker.pubkey(), AIRDROP_LAMPORTS).unwrap();

    let ix = close_ix(attacker.pubkey(), c.vault_state, c.vault);
    assert_fails_with(try_send(&mut c.svm, &attacker, ix), "ConstraintSeeds");
    assert!(c.svm.get_account(&c.vault_state).is_some());
}

#[test]
fn vault_can_be_reopened_after_close() {
    let mut c = funded();
    let ix = close_ix(c.user.pubkey(), c.vault_state, c.vault);
    send(&mut c.svm, &c.user, ix);

    let ix = initialize_ix(c.user.pubkey(), c.vault_state, c.vault);
    send(&mut c.svm, &c.user, ix);
    let ix = deposit_ix(c.user.pubkey(), c.vault_state, c.vault, WITHDRAW_LAMPORTS);
    send(&mut c.svm, &c.user, ix);

    let rent_exempt = c.svm.minimum_balance_for_rent_exemption(0);
    assert_eq!(
        c.svm.get_balance(&c.vault).unwrap(),
        rent_exempt + WITHDRAW_LAMPORTS
    );
}
