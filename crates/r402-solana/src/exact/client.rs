//! Client-side payment signing for the Solana "exact" scheme.
//!
//! This module provides [`SolanaExactClient`] for building and signing
//! SPL Token transfer transactions on Solana.
//!
//! # Features
//!
//! - Automatic compute unit estimation via simulation
//! - Priority fee calculation from recent fees
//! - SPL Token and Token-2022 support
//! - Transaction building with proper instruction ordering

use std::future::Future;
use std::pin::Pin;

use r402_client::{DefaultAssetInfo, PaymentCandidate, PaymentCandidateSigner, SchemeClient};
use r402_protocol::scheme::SchemeId;
use r402_protocol::{Base64Bytes, ChainId, ClientError, PaymentRequired, ResourceInfo};
use solana_client::rpc_config::RpcSimulateTransactionConfig;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_message::v0::Message as MessageV0;
use solana_message::{Hash, VersionedMessage};
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_signer::Signer;
use solana_transaction::Instruction;
use solana_transaction::versioned::VersionedTransaction;
use spl_token::solana_program::program_pack::Pack;

use crate::chain::Address;
use crate::chain::rpc::RpcClientLike;
use crate::exact::{
    ATA_PROGRAM_PUBKEY, ExactSolanaPayload, SPL_MEMO_PROGRAM, SolanaExact, TransactionInt, payload,
};

/// Mint information for SPL tokens.
#[derive(Debug, Clone, Copy)]
pub enum Mint {
    /// Standard SPL Token mint.
    Token {
        /// Number of decimal places.
        decimals: u8,
        /// SPL Token program ID.
        token_program: Pubkey,
    },
    /// SPL Token-2022 mint.
    Token2022 {
        /// Number of decimal places.
        decimals: u8,
        /// SPL Token-2022 program ID.
        token_program: Pubkey,
    },
}

impl Mint {
    /// Returns the SPL Token program ID for this mint.
    #[must_use]
    pub const fn token_program(&self) -> &Pubkey {
        match self {
            Self::Token { token_program, .. } | Self::Token2022 { token_program, .. } => {
                token_program
            }
        }
    }
}

/// Fetch mint information from the blockchain.
///
/// # Errors
///
/// Returns [`ClientError`] if the mint account cannot be fetched or parsed.
pub async fn fetch_mint<R: RpcClientLike>(
    mint_address: &Address,
    rpc_client: &R,
) -> Result<Mint, ClientError> {
    let mint_pubkey = mint_address.pubkey();
    let account = rpc_client
        .get_account(mint_pubkey)
        .await
        .map_err(|e| ClientError::Signing(format!("failed to fetch mint {mint_pubkey}: {e}")))?;
    if account.owner == spl_token::id() {
        let mint = spl_token::state::Mint::unpack(&account.data).map_err(|e| {
            ClientError::Signing(format!("failed to unpack mint {mint_pubkey}: {e}"))
        })?;
        Ok(Mint::Token {
            decimals: mint.decimals,
            token_program: spl_token::id(),
        })
    } else if account.owner == spl_token_2022_interface::id() {
        let mint = spl_token_2022_interface::state::Mint::unpack(&account.data).map_err(|e| {
            ClientError::Signing(format!("failed to unpack mint {mint_pubkey}: {e}"))
        })?;
        Ok(Mint::Token2022 {
            decimals: mint.decimals,
            token_program: spl_token_2022_interface::id(),
        })
    } else {
        Err(ClientError::Signing(format!(
            "failed to unpack mint {mint_pubkey}: unknown owner"
        )))
    }
}

/// Build the message we want to simulate (priority fee + transfer Ixs).
///
/// # Errors
///
/// Returns [`ClientError`] if message compilation fails.
pub fn build_message_to_simulate(
    fee_payer: Pubkey,
    transfer_instructions: &[Instruction],
    priority_micro_lamports: u64,
    recent_blockhash: Hash,
) -> Result<(MessageV0, Vec<Instruction>), ClientError> {
    let set_price = ComputeBudgetInstruction::set_compute_unit_price(priority_micro_lamports);

    let mut ixs = Vec::with_capacity(1 + transfer_instructions.len());
    ixs.push(set_price);
    ixs.extend(transfer_instructions.to_owned());

    let with_cu_limit = {
        let mut ixs_mod = ixs.clone();
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "1e5 fits in u32"
        )]
        update_or_append_set_compute_unit_limit(&mut ixs_mod, 1e5 as u32);
        ixs_mod
    };
    let message = MessageV0::try_compile(&fee_payer, &with_cu_limit, &[], recent_blockhash)
        .map_err(|e| ClientError::Signing(format!("{e:?}")))?;
    Ok((message, ixs))
}

/// Estimate compute units by simulating the unsigned/signed tx.
///
/// # Errors
///
/// Returns [`ClientError`] if simulation fails.
pub async fn estimate_compute_units<S: RpcClientLike>(
    rpc_client: &S,
    message: &MessageV0,
) -> Result<u32, ClientError> {
    let message = VersionedMessage::V0(message.clone());
    let num_required_signatures = message.header().num_required_signatures;
    let tx = VersionedTransaction {
        signatures: vec![Signature::default(); num_required_signatures as usize],
        message,
    };

    let sim = rpc_client
        .simulate_transaction_with_config(
            &tx,
            RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: true,
                ..RpcSimulateTransactionConfig::default()
            },
        )
        .await
        .map_err(|e| ClientError::Signing(format!("{e:?}")))?;
    let units = sim
        .value
        .units_consumed
        .ok_or_else(|| ClientError::Signing("simulation returned no units_consumed".to_owned()))?;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "compute units always fit in u32"
    )]
    Ok(units as u32)
}

/// Get the priority fee in micro-lamports.
///
/// # Errors
///
/// Returns [`ClientError`] if fee retrieval fails.
pub async fn get_priority_fee_micro_lamports<S: RpcClientLike>(
    rpc_client: &S,
    writeable_accounts: &[Pubkey],
) -> Result<u64, ClientError> {
    let recent_fees = rpc_client
        .get_recent_prioritization_fees(writeable_accounts)
        .await
        .map_err(|e| ClientError::Signing(format!("{e:?}")))?;
    let fee = recent_fees
        .iter()
        .filter_map(|e| {
            if e.prioritization_fee > 0 {
                Some(e.prioritization_fee)
            } else {
                None
            }
        })
        .min_by(Ord::cmp)
        .unwrap_or(1);
    Ok(fee)
}

/// SPL Memo instruction whose data equals `memo` (spec ≤ 256 bytes).
///
/// # Errors
///
/// Returns [`ClientError::Signing`] when `memo` exceeds 256 bytes.
pub fn memo_instruction(memo: &str) -> Result<Instruction, ClientError> {
    if memo.len() > 256 {
        return Err(ClientError::Signing(format!(
            "memo exceeds 256 bytes ({})",
            memo.len()
        )));
    }
    Ok(Instruction {
        program_id: SPL_MEMO_PROGRAM,
        accounts: Vec::new(),
        data: memo.as_bytes().to_vec(),
    })
}

/// Transfer instruction plus a Memo instruction (spec §3.1).
///
/// Uses `extra.memo` when set and non-empty (≤256 bytes). Empty/absent memo
/// becomes a 16-byte random nonce encoded as 32 lowercase hex characters
/// (official TS `if (sellerMemo)`).
///
/// # Errors
///
/// Returns [`ClientError::Signing`] when a declared memo exceeds 256 bytes.
pub fn with_memo(
    transfer: Instruction,
    extra_memo: Option<&str>,
) -> Result<Vec<Instruction>, ClientError> {
    let memo_ix = if let Some(memo) = extra_memo.filter(|m| !m.is_empty()) {
        memo_instruction(memo)?
    } else {
        let mut nonce = [0u8; 16];
        rand::fill(&mut nonce);
        Instruction {
            program_id: SPL_MEMO_PROGRAM,
            accounts: Vec::new(),
            data: hex::encode(nonce).into_bytes(),
        }
    };
    Ok(vec![transfer, memo_ix])
}

/// Update the first `set_compute_unit_limit` ix if it exists, else append a new one.
pub fn update_or_append_set_compute_unit_limit(ixs: &mut Vec<Instruction>, units: u32) {
    let target_program = solana_compute_budget_interface::ID;
    let new_ix = ComputeBudgetInstruction::set_compute_unit_limit(units);

    // SetComputeUnitLimit discriminator byte is 2
    let ix = ixs
        .iter_mut()
        .find(|ix| ix.program_id == target_program && ix.data.first().copied() == Some(2));
    if let Some(ix) = ix {
        *ix = new_ix;
    } else {
        ixs.push(new_ix);
    }
}

/// Build and sign a Solana token transfer transaction.
///
/// Returns the base64-encoded signed transaction.
///
/// # Errors
///
/// Returns [`ClientError`] if transaction building or signing fails.
pub async fn build_signed_transfer_transaction<S: Signer + Sync, R: RpcClientLike>(
    signer: &S,
    rpc_client: &R,
    fee_payer: &Pubkey,
    pay_to: &Address,
    asset: &Address,
    amount: u64,
    memo: Option<&str>,
) -> Result<String, ClientError> {
    let mint = fetch_mint(asset, rpc_client).await?;

    let (ata, _) = Pubkey::find_program_address(
        &[
            pay_to.as_ref(),
            mint.token_program().as_ref(),
            asset.as_ref(),
        ],
        &ATA_PROGRAM_PUBKEY,
    );

    let client_pubkey = signer.pubkey();
    let (source_ata, _) = Pubkey::find_program_address(
        &[
            client_pubkey.as_ref(),
            mint.token_program().as_ref(),
            asset.as_ref(),
        ],
        &ATA_PROGRAM_PUBKEY,
    );
    let destination_ata = ata;

    let transfer_instruction = match mint {
        Mint::Token {
            decimals,
            token_program,
        } => spl_token::instruction::transfer_checked(
            &token_program,
            &source_ata,
            asset.pubkey(),
            &destination_ata,
            &client_pubkey,
            &[],
            amount,
            decimals,
        )
        .map_err(|e| ClientError::Signing(format!("{e}")))?,
        Mint::Token2022 {
            decimals,
            token_program,
        } => spl_token_2022_interface::instruction::transfer_checked(
            &token_program,
            &source_ata,
            asset.pubkey(),
            &destination_ata,
            &client_pubkey,
            &[],
            amount,
            decimals,
        )
        .map_err(|e| ClientError::Signing(format!("{e}")))?,
    };

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .await
        .map_err(|e| ClientError::Signing(format!("{e:?}")))?;

    let fee =
        get_priority_fee_micro_lamports(rpc_client, &[*fee_payer, destination_ata, source_ata])
            .await?;

    let transfer_instructions = with_memo(transfer_instruction, memo)?;
    let (msg_to_sim, instructions) =
        build_message_to_simulate(*fee_payer, &transfer_instructions, fee, recent_blockhash)?;

    let estimated_cu = estimate_compute_units(rpc_client, &msg_to_sim).await?;

    let cu_ix = ComputeBudgetInstruction::set_compute_unit_limit(estimated_cu);
    let msg = {
        let mut final_instructions = Vec::with_capacity(instructions.len() + 1);
        final_instructions.push(cu_ix);
        final_instructions.extend(instructions);
        MessageV0::try_compile(fee_payer, &final_instructions, &[], recent_blockhash)
            .map_err(|e| ClientError::Signing(format!("{e:?}")))?
    };

    let tx = VersionedTransaction {
        signatures: vec![],
        message: VersionedMessage::V0(msg),
    };

    let tx = TransactionInt::new(tx);
    let signed = tx
        .sign_with_keypair(signer)
        .map_err(|e| ClientError::Signing(format!("{e:?}")))?;
    let tx_b64 = signed
        .as_base64()
        .map_err(|e| ClientError::Signing(format!("{e:?}")))?;

    Ok(tx_b64)
}

/// Solana exact scheme client for building and signing payment transactions.
#[derive(Clone)]
pub struct SolanaExactClient<S, R> {
    signer: S,
    rpc_client: R,
}

impl<S, R> std::fmt::Debug for SolanaExactClient<S, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SolanaExactClient").finish_non_exhaustive()
    }
}

impl<S, R> SolanaExactClient<S, R> {
    /// Creates a new Solana exact client.
    pub const fn new(signer: S, rpc_client: R) -> Self {
        Self { signer, rpc_client }
    }
}

impl<S, R> SchemeId for SolanaExactClient<S, R> {
    fn namespace(&self) -> &str {
        SolanaExact.namespace()
    }

    fn scheme(&self) -> &str {
        SolanaExact.scheme()
    }
}

impl<S, R> SchemeClient for SolanaExactClient<S, R>
where
    S: Signer + Send + Sync + Clone + 'static,
    R: RpcClientLike + Send + Sync + Clone + 'static,
{
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        payment_required
            .accepts
            .iter()
            .filter_map(|v| {
                let requirements: payload::v2::PaymentRequirements = v.as_concrete()?;
                let chain_id = requirements.network.clone();
                if chain_id.namespace() != "solana" {
                    return None;
                }
                let candidate = PaymentCandidate {
                    chain_id,
                    asset: requirements.asset.to_string().into(),
                    amount: requirements.amount.inner().to_string().into(),
                    scheme: self.scheme().into(),
                    pay_to: requirements.pay_to.to_string().into(),
                    requirements: v.clone(),
                    signer: Box::new(V2PayloadSigner {
                        signer: self.signer.clone(),
                        rpc_client: self.rpc_client.clone(),
                        requirements,
                        resource: payment_required.resource.clone(),
                    }),
                };
                Some(candidate)
            })
            .collect::<Vec<_>>()
    }

    fn find_default_asset(&self, asset: &str, network: &ChainId) -> Option<DefaultAssetInfo> {
        crate::find_default_solana_asset(asset, network)
    }
}

struct V2PayloadSigner<S, R> {
    signer: S,
    rpc_client: R,
    requirements: payload::v2::PaymentRequirements,
    resource: ResourceInfo,
}

impl<S: Signer + Send + Sync, R: RpcClientLike + Send + Sync> PaymentCandidateSigner
    for V2PayloadSigner<S, R>
{
    fn sign_payment(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<String, ClientError>> + Send + '_>> {
        Box::pin(async move {
            let fee_payer = self
                .requirements
                .extra
                .as_ref()
                .map(|extra| extra.fee_payer)
                .ok_or_else(|| ClientError::Signing("missing fee_payer in extra".to_owned()))?;
            let fee_payer_pubkey: Pubkey = fee_payer.into();

            let amount = self.requirements.amount.inner();
            let memo = self
                .requirements
                .extra
                .as_ref()
                .and_then(|extra| extra.memo.as_deref());
            let tx_b64 = build_signed_transfer_transaction(
                &self.signer,
                &self.rpc_client,
                &fee_payer_pubkey,
                &self.requirements.pay_to,
                &self.requirements.asset,
                amount,
                memo,
            )
            .await?;

            let payload = payload::v2::PaymentPayload::new(
                self.requirements.clone(),
                ExactSolanaPayload {
                    transaction: tx_b64,
                },
            )
            .with_resource(self.resource.clone());
            let json = serde_json::to_vec(&payload)?;
            let b64 = Base64Bytes::encode(&json);

            Ok(b64.to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_transfer() -> Instruction {
        Instruction {
            program_id: spl_token::id(),
            accounts: Vec::new(),
            data: vec![12],
        }
    }

    #[test]
    fn memo_instruction_matches_declared_bytes() {
        let ix = memo_instruction("order-123").expect("memo");
        assert_eq!(ix.program_id, SPL_MEMO_PROGRAM);
        assert!(ix.accounts.is_empty());
        assert_eq!(ix.data, b"order-123");
    }

    #[test]
    fn memo_instruction_rejects_over_256_bytes() {
        assert!(memo_instruction(&"a".repeat(257)).is_err());
        assert!(memo_instruction(&"a".repeat(256)).is_ok());
    }

    #[test]
    fn declared_memo_is_appended_after_transfer() {
        let ixs = with_memo(dummy_transfer(), Some("order-123")).expect("ixs");
        assert_eq!(ixs.len(), 2);
        assert_eq!(ixs[0].program_id, spl_token::id());
        assert_eq!(ixs[1].program_id, SPL_MEMO_PROGRAM);
        assert_eq!(ixs[1].data, b"order-123");
    }

    #[test]
    fn absent_memo_appends_32_byte_hex_nonce() {
        let ixs = with_memo(dummy_transfer(), None).expect("ixs");
        assert_eq!(ixs.len(), 2);
        assert_eq!(ixs[0].program_id, spl_token::id());
        assert_eq!(ixs[1].program_id, SPL_MEMO_PROGRAM);
        assert_eq!(ixs[1].data.len(), 32);
        assert!(ixs[1].data.iter().all(u8::is_ascii_hexdigit));
        let again = with_memo(dummy_transfer(), None).expect("ixs");
        assert_ne!(ixs[1].data, again[1].data);
    }

    #[test]
    fn with_memo_rejects_over_256_bytes() {
        assert!(with_memo(dummy_transfer(), Some(&"a".repeat(257))).is_err());
        assert!(with_memo(dummy_transfer(), Some(&"a".repeat(256))).is_ok());
    }

    #[test]
    fn empty_extra_memo_is_treated_as_absent() {
        let ixs = with_memo(dummy_transfer(), Some("")).expect("ixs");
        assert_eq!(ixs.len(), 2);
        assert_eq!(ixs[1].program_id, SPL_MEMO_PROGRAM);
        assert_eq!(ixs[1].data.len(), 32);
        assert!(ixs[1].data.iter().all(u8::is_ascii_hexdigit));
    }
}
