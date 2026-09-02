//! Bindings for the canonical payment-channels program used by SVM `upto`.
//!
//! Program id is a network/SDK constant and is never negotiated over the wire.
#![allow(
    unreachable_pub,
    reason = "layout constants are crate-visible through this module"
)]

mod codec;
mod program;
mod voucher;

#[cfg(any(feature = "client", feature = "facilitator"))]
mod instructions;
#[cfg(any(feature = "client", feature = "facilitator"))]
mod open;

#[cfg(feature = "facilitator")]
mod channel;

pub use codec::{ChannelSplit, OpenArgs, decode_open_args, encode_open_args};
pub use program::{
    ASSOCIATED_TOKEN_PROGRAM_ID, ChannelStatus, DEFAULT_COMPUTE_UNIT_PRICE_MICROLAMPORTS,
    DEFAULT_GRACE_PERIOD_SECONDS, DEFAULT_SETTLE_COMPUTE_UNIT_LIMIT, ED25519_PROGRAM_ID,
    INSTRUCTIONS_SYSVAR, OPEN_ACCOUNT_COUNT, OPEN_DEFAULT_COMPUTE_UNIT_LIMIT,
    OPEN_MAX_COMPUTE_UNIT_LIMIT, OPEN_SLOT_WINDOW, PAYMENT_CHANNELS_PROGRAM_ID, RENT_SYSVAR,
    SYSTEM_PROGRAM_ID, treasury_owner,
};
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use program::{find_ata, find_channel_pda, find_event_authority_pda};
pub use voucher::{VOUCHER_MAGIC, VOUCHER_MESSAGE_SIZE, encode_voucher_message};
#[cfg(any(feature = "server", feature = "client", feature = "facilitator"))]
pub use voucher::{sign_voucher, verify_voucher_signature};

#[cfg(any(feature = "client", feature = "facilitator"))]
pub use instructions::{
    DistributeInstructionArgs, build_distribute_instruction, build_ed25519_verify_instruction,
    build_open_instruction, build_reclaim_instruction, build_settle_and_seal_instruction,
};
#[cfg(any(feature = "client", feature = "facilitator"))]
pub use open::{
    BuildOpenArgs, BuiltOpen, OpenError, VerifyOpenExpected, VerifyOpenResult,
    build_open_transaction, verify_open_transaction,
};

#[cfg(feature = "facilitator")]
pub use channel::{
    CHANNEL_ACCOUNT_SIZE, Channel, DecodeChannelError, decode_channel, distribution_hash,
};
