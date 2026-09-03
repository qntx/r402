//! Canonical program constants and PDA derivation.
#![allow(
    unreachable_pub,
    dead_code,
    reason = "some constants are only used by client/facilitator instruction builders"
)]

use solana_pubkey::{Pubkey, pubkey};

/// Canonical payment-channels program id (mainnet / Surfnet deployment).
pub const PAYMENT_CHANNELS_PROGRAM_ID: Pubkey =
    pubkey!("CHNLxYvVA28MJP9PrFuDXccuoGXAx7jBacfLEkahyGsX");

/// Ed25519 signature-verification precompile.
pub const ED25519_PROGRAM_ID: Pubkey = pubkey!("Ed25519SigVerify111111111111111111111111111");

/// Instructions sysvar (`Sysvar1nstructions…`).
pub const INSTRUCTIONS_SYSVAR: Pubkey = pubkey!("Sysvar1nstructions1111111111111111111111111");

/// Rent sysvar.
pub const RENT_SYSVAR: Pubkey = pubkey!("SysvarRent111111111111111111111111111111111");

/// System program.
pub const SYSTEM_PROGRAM_ID: Pubkey = pubkey!("11111111111111111111111111111111");

/// Associated Token program (Token and Token-2022 share this id).
pub const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey =
    pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

/// `open` instruction discriminator.
pub const OPEN_DISCRIMINATOR: u8 = 1;
/// `settle_and_seal` instruction discriminator.
pub const SETTLE_AND_SEAL_DISCRIMINATOR: u8 = 4;
/// `distribute` instruction discriminator.
pub const DISTRIBUTE_DISCRIMINATOR: u8 = 7;
/// `reclaim` instruction discriminator.
pub const RECLAIM_DISCRIMINATOR: u8 = 9;

/// Canonical account count of an `open` instruction.
pub const OPEN_ACCOUNT_COUNT: usize = 14;

/// Slot freshness / reclaim gate window.
pub const OPEN_SLOT_WINDOW: u64 = 1_500;

/// Default forced-close grace period (seconds).
pub const DEFAULT_GRACE_PERIOD_SECONDS: u32 = 900;

/// Spec ceiling for `SetComputeUnitLimit` on an open transaction.
pub const OPEN_MAX_COMPUTE_UNIT_LIMIT: u32 = 400_000;

/// Default `SetComputeUnitLimit` for a built open transaction.
pub const OPEN_DEFAULT_COMPUTE_UNIT_LIMIT: u32 = 90_000;

/// Default `SetComputeUnitLimit` for facilitator-submitted settlement txs.
pub const DEFAULT_SETTLE_COMPUTE_UNIT_LIMIT: u32 = 100_000;

/// Default `SetComputeUnitPrice` (microlamports per CU).
pub const DEFAULT_COMPUTE_UNIT_PRICE_MICROLAMPORTS: u64 = 1;

/// Full distribution share in basis points.
pub const BASIS_POINTS_DENOMINATOR: u16 = 10_000;

/// Max optional suffix instructions after `open` (3 Lighthouse + 1 Memo).
pub const OPEN_MAX_OPTIONAL_SUFFIX: usize = 4;

/// Max Phantom/Solflare Lighthouse assertions after `open`.
pub const OPEN_MAX_LIGHTHOUSE_INSTRUCTIONS: usize = 3;

/// Onchain `Channel.status` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ChannelStatus {
    /// Channel is open and can be settled.
    Open = 0,
    /// Cooperative close in progress.
    Closing = 1,
    /// Sealed; awaiting distribute.
    Sealed = 2,
    /// Distributed; awaiting reclaim.
    Distributed = 3,
}

const MAINNET_TREASURY_OWNER: Pubkey = pubkey!("Cs2zdfUNonRdRGsiZUQQLdTxzxVvJZmgiX2mpLYKuEqP");
const DEVNET_TREASURY_OWNER: Pubkey = pubkey!("4zTeC5mVqWLruDexgU2mV66p9t5vCA9JyiZqdGDUspap");

/// Treasury owner baked into the program binary for `network`.
#[must_use]
pub fn treasury_owner(network: &str) -> Pubkey {
    if network == "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1" || network == "solana-devnet" {
        DEVNET_TREASURY_OWNER
    } else {
        MAINNET_TREASURY_OWNER
    }
}

/// Channel PDA seeds: `"channel"`, payer, payee, mint, authorized signer, salt, open slot.
#[cfg(any(feature = "client", feature = "facilitator"))]
#[must_use]
pub fn find_channel_pda(
    payer: &Pubkey,
    payee: &Pubkey,
    mint: &Pubkey,
    authorized_signer: &Pubkey,
    salt: u64,
    open_slot: u64,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            b"channel",
            payer.as_ref(),
            payee.as_ref(),
            mint.as_ref(),
            authorized_signer.as_ref(),
            &salt.to_le_bytes(),
            &open_slot.to_le_bytes(),
        ],
        &PAYMENT_CHANNELS_PROGRAM_ID,
    )
}

/// Event-authority PDA (`"event_authority"`).
#[cfg(any(feature = "client", feature = "facilitator"))]
#[must_use]
pub fn find_event_authority_pda() -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"event_authority"], &PAYMENT_CHANNELS_PROGRAM_ID)
}

/// Associated token account for `owner`/`mint` under `token_program`.
#[cfg(any(feature = "client", feature = "facilitator"))]
#[must_use]
pub fn find_ata(owner: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[owner.as_ref(), token_program.as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    )
}
