//! Client-side payment signing for the Solana "exact" scheme.
//!
//! This module provides [`V1SolanaExactClient`] and [`V2SolanaExactClient`] for
//! building and signing SPL Token transfer transactions on Solana.
//! Both share the core transaction building logic via [`build_signed_transfer_transaction`].
//!
//! # Features
//!
//! - Automatic compute unit estimation via simulation
//! - Priority fee calculation from recent fees
//! - SPL Token and Token-2022 support
//! - Transaction building with proper instruction ordering

use r402::encoding::Base64Bytes;
use r402::proto::PaymentRequired;
use r402::proto::v1;
use r402::proto::v2::{self, ResourceInfo};
use r402::scheme::X402SchemeId;
use r402::scheme::{PaymentCandidate, PaymentCandidateSigner, X402Error, X402SchemeClient};
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
use std::future::Future;
use std::pin::Pin;

use crate::chain::Address;
use crate::chain::rpc::RpcClientLike;
use crate::exact::types;
use crate::exact::{
    ATA_PROGRAM_PUBKEY, ExactScheme, ExactSolanaPayload, TransactionInt, V1SolanaExact,
    V2SolanaExact,
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
/// Returns [`X402Error`] if the mint account cannot be fetched or parsed.
pub async fn fetch_mint<R: RpcClientLike>(
    mint_address: &Address,
    rpc_client: &R,
) -> Result<Mint, X402Error> {
    let mint_pubkey = mint_address.pubkey();
    let account = rpc_client
        .get_account(mint_pubkey)
        .await
        .map_err(|e| X402Error::SigningError(format!("failed to fetch mint {mint_pubkey}: {e}")))?;
    if account.owner == spl_token::id() {
        let mint = spl_token::state::Mint::unpack(&account.data).map_err(|e| {
            X402Error::SigningError(format!("failed to unpack mint {mint_pubkey}: {e}"))
        })?;
        Ok(Mint::Token {
            decimals: mint.decimals,
            token_program: spl_token::id(),
        })
    } else if account.owner == spl_token_2022::id() {
        let mint = spl_token_2022::state::Mint::unpack(&account.data).map_err(|e| {
            X402Error::SigningError(format!("failed to unpack mint {mint_pubkey}: {e}",))
        })?;
        Ok(Mint::Token2022 {
            decimals: mint.decimals,
            token_program: spl_token_2022::id(),
        })
    } else {
        Err(X402Error::SigningError(format!(
            "failed to unpack mint {mint_pubkey}: unknown owner"
        )))
    }
}

/// Build the message we want to simulate (priority fee + transfer Ixs).
///
/// # Errors
///
/// Returns [`X402Error`] if message compilation fails.
pub fn build_message_to_simulate(
    fee_payer: Pubkey,
    transfer_instructions: &[Instruction],
    priority_micro_lamports: u64,
    recent_blockhash: Hash,
) -> Result<(MessageV0, Vec<Instruction>), X402Error> {
    let set_price = ComputeBudgetInstruction::set_compute_unit_price(priority_micro_lamports);

    let mut ixs = Vec::with_capacity(1 + transfer_instructions.len());
    ixs.push(set_price);
    ixs.extend(transfer_instructions.to_owned());

    let with_cu_limit = {
        let mut ixs_mod = ixs.clone();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        update_or_append_set_compute_unit_limit(&mut ixs_mod, 1e5 as u32);
        ixs_mod
    };
    let message = MessageV0::try_compile(&fee_payer, &with_cu_limit, &[], recent_blockhash)
        .map_err(|e| X402Error::SigningError(format!("{e:?}")))?;
    Ok((message, ixs))
}

/// Estimate compute units by simulating the unsigned/signed tx.
///
/// # Errors
///
/// Returns [`X402Error`] if simulation fails.
pub async fn estimate_compute_units<S: RpcClientLike>(
    rpc_client: &S,
    message: &MessageV0,
) -> Result<u32, X402Error> {
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
        .map_err(|e| X402Error::SigningError(format!("{e:?}")))?;
    let units = sim.value.units_consumed.ok_or_else(|| {
        X402Error::SigningError("simulation returned no units_consumed".to_string())
    })?;
    #[allow(clippy::cast_possible_truncation)]
    Ok(units as u32)
}

/// Get the priority fee in micro-lamports.
///
/// # Errors
///
/// Returns [`X402Error`] if fee retrieval fails.
pub async fn get_priority_fee_micro_lamports<S: RpcClientLike>(
    rpc_client: &S,
    writeable_accounts: &[Pubkey],
) -> Result<u64, X402Error> {
    let recent_fees = rpc_client
        .get_recent_prioritization_fees(writeable_accounts)
        .await
        .map_err(|e| X402Error::SigningError(format!("{e:?}")))?;
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
/// Returns [`X402Error`] if transaction building or signing fails.
pub async fn build_signed_transfer_transaction<S: Signer + Sync, R: RpcClientLike>(
    signer: &S,
    rpc_client: &R,
    fee_payer: &Pubkey,
    pay_to: &Address,
    asset: &Address,
    amount: u64,
) -> Result<String, X402Error> {
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
        .map_err(|e| X402Error::SigningError(format!("{e}")))?,
        Mint::Token2022 {
            decimals,
            token_program,
        } => spl_token_2022::instruction::transfer_checked(
            &token_program,
            &source_ata,
            asset.pubkey(),
            &destination_ata,
            &client_pubkey,
            &[],
            amount,
            decimals,
        )
        .map_err(|e| X402Error::SigningError(format!("{e}")))?,
    };

    let recent_blockhash = rpc_client
        .get_latest_blockhash()
        .await
        .map_err(|e| X402Error::SigningError(format!("{e:?}")))?;

    let fee =
        get_priority_fee_micro_lamports(rpc_client, &[*fee_payer, destination_ata, source_ata])
            .await?;

    let (msg_to_sim, instructions) =
        build_message_to_simulate(*fee_payer, &[transfer_instruction], fee, recent_blockhash)?;

    let estimated_cu = estimate_compute_units(rpc_client, &msg_to_sim).await?;

    let cu_ix = ComputeBudgetInstruction::set_compute_unit_limit(estimated_cu);
    let msg = {
        let mut final_instructions = Vec::with_capacity(instructions.len() + 1);
        final_instructions.push(cu_ix);
        final_instructions.extend(instructions);
        MessageV0::try_compile(fee_payer, &final_instructions, &[], recent_blockhash)
            .map_err(|e| X402Error::SigningError(format!("{e:?}")))?
    };

    let tx = VersionedTransaction {
        signatures: vec![],
        message: VersionedMessage::V0(msg),
    };

    let tx = TransactionInt::new(tx);
    let signed = tx
        .sign_with_keypair(signer)
        .map_err(|e| X402Error::SigningError(format!("{e:?}")))?;
    let tx_b64 = signed
        .as_base64()
        .map_err(|e| X402Error::SigningError(format!("{e:?}")))?;

    Ok(tx_b64)
}

/// V1 Solana exact scheme client for building and signing payment transactions.
pub struct V1SolanaExactClient<S, R> {
    signer: S,
    rpc_client: R,
}

impl<S, R> std::fmt::Debug for V1SolanaExactClient<S, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V1SolanaExactClient")
            .finish_non_exhaustive()
    }
}

impl<S, R> V1SolanaExactClient<S, R> {
    /// Creates a new V1 Solana exact client.
    pub const fn new(signer: S, rpc_client: R) -> Self {
        Self { signer, rpc_client }
    }
}

impl<S, R> X402SchemeId for V1SolanaExactClient<S, R> {
    fn x402_version(&self) -> u8 {
        V1SolanaExact.x402_version()
    }

    fn namespace(&self) -> &str {
        V1SolanaExact.namespace()
    }

    fn scheme(&self) -> &str {
        V1SolanaExact.scheme()
    }
}

impl<S, R> X402SchemeClient for V1SolanaExactClient<S, R>
where
    S: Signer + Send + Sync + Clone + 'static,
    R: RpcClientLike + Send + Sync + Clone + 'static,
{
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        let PaymentRequired::V1(payment_required) = payment_required else {
            return vec![];
        };
        payment_required
            .accepts
            .iter()
            .filter_map(|v| {
                let requirements: types::v1::PaymentRequirements = v.as_concrete()?;
                let chain_id = crate::networks::solana_network_registry()
                    .chain_id_by_name(&requirements.network)?
                    .clone();
                if chain_id.namespace() != "solana" {
                    return None;
                }
                let candidate = PaymentCandidate {
                    chain_id,
                    asset: requirements.asset.to_string(),
                    amount: requirements.max_amount_required.inner().to_string(),
                    scheme: self.scheme().to_string(),
                    x402_version: self.x402_version(),
                    pay_to: requirements.pay_to.to_string(),
                    signer: Box::new(V1PayloadSigner {
                        signer: self.signer.clone(),
                        rpc_client: self.rpc_client.clone(),
                        requirements,
                    }),
                };
                Some(candidate)
            })
            .collect::<Vec<_>>()
    }
}

struct V1PayloadSigner<S, R> {
    signer: S,
    rpc_client: R,
    requirements: types::v1::PaymentRequirements,
}

impl<S: Signer + Sync, R: RpcClientLike + Sync> PaymentCandidateSigner for V1PayloadSigner<S, R> {
    fn sign_payment(&self) -> Pin<Box<dyn Future<Output = Result<String, X402Error>> + Send + '_>> {
        Box::pin(async move {
            let fee_payer = self
                .requirements
                .extra
                .as_ref()
                .map(|extra| extra.fee_payer)
                .ok_or_else(|| X402Error::SigningError("missing fee_payer in extra".to_string()))?;
            let fee_payer_pubkey: Pubkey = fee_payer.into();

            let amount = self.requirements.max_amount_required.inner();
            let tx_b64 = build_signed_transfer_transaction(
                &self.signer,
                &self.rpc_client,
                &fee_payer_pubkey,
                &self.requirements.pay_to,
                &self.requirements.asset,
                amount,
            )
            .await?;

            let payload = types::v1::PaymentPayload {
                x402_version: v1::V1,
                scheme: ExactScheme,
                network: self.requirements.network.clone(),
                payload: ExactSolanaPayload {
                    transaction: tx_b64,
                },
            };
            let json = serde_json::to_vec(&payload)?;
            let b64 = Base64Bytes::encode(&json);

            Ok(b64.to_string())
        })
    }
}

/// V2 Solana exact scheme client for building and signing payment transactions.
#[derive(Clone)]
pub struct V2SolanaExactClient<S, R> {
    signer: S,
    rpc_client: R,
}

impl<S, R> std::fmt::Debug for V2SolanaExactClient<S, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2SolanaExactClient")
            .finish_non_exhaustive()
    }
}

impl<S, R> V2SolanaExactClient<S, R> {
    /// Creates a new V2 Solana exact client.
    pub const fn new(signer: S, rpc_client: R) -> Self {
        Self { signer, rpc_client }
    }
}

impl<S, R> X402SchemeId for V2SolanaExactClient<S, R> {
    fn x402_version(&self) -> u8 {
        V2SolanaExact.x402_version()
    }

    fn namespace(&self) -> &str {
        V2SolanaExact.namespace()
    }

    fn scheme(&self) -> &str {
        V2SolanaExact.scheme()
    }
}

impl<S, R> X402SchemeClient for V2SolanaExactClient<S, R>
where
    S: Signer + Send + Sync + Clone + 'static,
    R: RpcClientLike + Send + Sync + Clone + 'static,
{
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        let PaymentRequired::V2(payment_required) = payment_required else {
            return vec![];
        };
        payment_required
            .accepts
            .iter()
            .filter_map(|v| {
                let requirements: types::v2::PaymentRequirements = v.as_concrete()?;
                let chain_id = requirements.network.clone();
                if chain_id.namespace() != "solana" {
                    return None;
                }
                let candidate = PaymentCandidate {
                    chain_id,
                    asset: requirements.asset.to_string(),
                    amount: requirements.amount.inner().to_string(),
                    scheme: self.scheme().to_string(),
                    x402_version: self.x402_version(),
                    pay_to: requirements.pay_to.to_string(),
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
}

struct V2PayloadSigner<S, R> {
    signer: S,
    rpc_client: R,
    requirements: types::v2::PaymentRequirements,
    resource: ResourceInfo,
}

impl<S: Signer + Sync, R: RpcClientLike + Sync> PaymentCandidateSigner for V2PayloadSigner<S, R> {
    fn sign_payment(&self) -> Pin<Box<dyn Future<Output = Result<String, X402Error>> + Send + '_>> {
        Box::pin(async move {
            let fee_payer = self
                .requirements
                .extra
                .as_ref()
                .map(|extra| extra.fee_payer)
                .ok_or_else(|| X402Error::SigningError("missing fee_payer in extra".to_string()))?;
            let fee_payer_pubkey: Pubkey = fee_payer.into();

            let amount = self.requirements.amount.inner();
            let tx_b64 = build_signed_transfer_transaction(
                &self.signer,
                &self.rpc_client,
                &fee_payer_pubkey,
                &self.requirements.pay_to,
                &self.requirements.asset,
                amount,
            )
            .await?;

            let payload = types::v2::PaymentPayload {
                x402_version: v2::V2,
                accepted: self.requirements.clone(),
                resource: Some(self.resource.clone()),
                payload: ExactSolanaPayload {
                    transaction: tx_b64,
                },
                extensions: None,
            };
            let json = serde_json::to_vec(&payload)?;
            let b64 = Base64Bytes::encode(&json);

            Ok(b64.to_string())
        })
    }
}
