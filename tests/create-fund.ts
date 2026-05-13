import anchor from "@anchor-lang/core";
import { createMint, getMint, TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { PublicKey, SystemProgram, SYSVAR_RENT_PUBKEY } from "@solana/web3.js";
import { assert } from "chai";

import type { Fund } from "../target/types/fund";

const { BN } = anchor;

/** Pad a UTF-8 name to the 32-byte fixed-size `name` field. */
function padName(name: string): Buffer {
  const buf = Buffer.alloc(32);
  buf.write(name);
  return buf;
}

describe("create_fund", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.Fund as anchor.Program<Fund>;
  const connection = provider.connection;
  const manager = (provider.wallet as anchor.Wallet).payer;

  // USDC stand-in: a fresh 6-decimal mint we own for the duration of the
  // test run. The fund's `quote_mint` parameter points at this.
  let usdcMint: PublicKey;

  before("create a USDC-like mint", async () => {
    usdcMint = await createMint(
      connection,
      manager,
      manager.publicKey,
      null,
      6,
    );
  });

  it("initializes a fund with the supplied parameters", async () => {
    const name = padName("test-fund-1");

    const [fund] = PublicKey.findProgramAddressSync(
      [Buffer.from("fund"), manager.publicKey.toBuffer(), name],
      program.programId,
    );
    const [vault] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), fund.toBuffer()],
      program.programId,
    );
    const [sharesMint] = PublicKey.findProgramAddressSync(
      [Buffer.from("shares"), fund.toBuffer()],
      program.programId,
    );

    const params = {
      name: Array.from(name),
      managementFeeBps: 200, // 2%
      performanceFeeBps: 2000, // 20%
      capacity: new BN(1_000_000_000_000), // 1M USDC (6 decimals)
      withdrawalDelayDays: 7,
    };

    await program.methods
      .createFund(params)
      .accounts({
        manager: manager.publicKey,
        fund,
        quoteMint: usdcMint,
        vault,
        sharesMint,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .signers([manager])
      .rpc();

    // Fund parameters round-trip through on-chain state.
    const fundAccount = await program.account.fund.fetch(fund);
    assert(fundAccount.manager.equals(manager.publicKey), "manager mismatch");
    assert(fundAccount.quoteMint.equals(usdcMint), "quote_mint mismatch");
    assert.deepEqual(
      Array.from(fundAccount.name as Uint8Array),
      Array.from(name),
      "name mismatch",
    );
    assert.equal(
      fundAccount.managementFeeBps,
      params.managementFeeBps,
      "management_fee_bps mismatch",
    );
    assert.equal(
      fundAccount.performanceFeeBps,
      params.performanceFeeBps,
      "performance_fee_bps mismatch",
    );
    assert(fundAccount.capacity.eq(params.capacity), "capacity mismatch");
    assert.equal(
      fundAccount.withdrawalDelayDays,
      params.withdrawalDelayDays,
      "withdrawal_delay_days mismatch",
    );

    // Vault is a fresh quote-token account with balance 0.
    const vaultBalance = await connection.getTokenAccountBalance(vault);
    assert.equal(vaultBalance.value.amount, "0", "vault should start empty");

    // Shares mint exists with supply 0 and decimals matching quote.
    const sharesMintAccount = await getMint(connection, sharesMint);
    assert.equal(
      sharesMintAccount.supply,
      BigInt(0),
      "shares mint should start with 0 supply",
    );
    assert.equal(
      sharesMintAccount.decimals,
      6,
      "shares mint should match quote decimals",
    );
  });
});
