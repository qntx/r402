//! EVM client-side "exact" scheme implementation.
//!
//! Implements [`SchemeClient`] for the `exact` scheme. Creates signed
//! EIP-3009 `transferWithAuthorization` payloads using EIP-712 typed data
//! signing via an [`alloy_signer::Signer`].
//!
//! Corresponds to Python SDK's `mechanisms/evm/exact/client.py`.

use alloy_primitives::{Address, B256, U256};
use alloy_signer::Signer;
use alloy_sol_types::{Eip712Domain, SolStruct};
use r402::proto::PaymentRequirements;
use r402::scheme::{BoxFuture, SchemeClient, SchemeError};
use serde_json::Value;

use crate::chain::parse_caip2;
use crate::exact::types::{
    ExactAuthorization, ExactPayload, ExactRequirementsExtra, SCHEME_EXACT,
    TransferWithAuthorization,
};

/// EVM client implementation for the "exact" payment scheme.
///
/// Takes a generic [`Signer`] to create signed EIP-3009 payment
/// payloads. The signer provides the payer's address and EIP-712
/// signing capability.
///
/// # Type Parameters
///
/// - `S`: An alloy [`Signer`] (e.g., `PrivateKeySigner`, `LedgerSigner`).
///
/// # Example
///
/// ```no_run
/// use alloy_signer_local::PrivateKeySigner;
/// use r402_evm::exact::client::ExactEvmClient;
///
/// let signer = PrivateKeySigner::random();
/// let client = ExactEvmClient::new(signer);
/// ```
///
/// Corresponds to Python SDK's `ExactEvmScheme` in `exact/client.py`.
pub struct ExactEvmClient<S> {
    signer: S,
}

impl<S: Signer + Send + Sync> ExactEvmClient<S> {
    /// Creates a new exact scheme client with the given signer.
    pub fn new(signer: S) -> Self {
        Self { signer }
    }

    /// Returns the signer's address.
    ///
    /// # Errors
    ///
    /// Returns an error if the signer cannot provide its address.
    fn payer_address(&self) -> Address {
        self.signer.address()
    }

    /// Creates a random 32-byte nonce.
    fn create_nonce() -> B256 {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).expect("getrandom failed");
        B256::from(bytes)
    }

    /// Computes the validity window (validAfter, validBefore) as unix timestamps.
    fn create_validity_window(max_timeout_seconds: u64) -> (u64, u64) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before UNIX epoch")
            .as_secs();

        let valid_after = now.saturating_sub(60);
        let valid_before = now + max_timeout_seconds;

        (valid_after, valid_before)
    }

    /// Builds the EIP-712 domain from requirements.
    fn build_domain(
        chain_id: u64,
        verifying_contract: Address,
        extra: &ExactRequirementsExtra,
    ) -> Eip712Domain {
        Eip712Domain {
            name: Some(extra.name.clone().into()),
            version: Some(extra.version.clone().into()),
            chain_id: Some(U256::from(chain_id)),
            verifying_contract: Some(verifying_contract),
            salt: None,
        }
    }
}

impl<S> std::fmt::Debug for ExactEvmClient<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExactEvmClient").finish_non_exhaustive()
    }
}

impl<S> SchemeClient for ExactEvmClient<S>
where
    S: Signer + Send + Sync + 'static,
{
    fn scheme(&self) -> &str {
        SCHEME_EXACT
    }

    fn create_payment_payload<'a>(
        &'a self,
        requirements: &'a PaymentRequirements,
    ) -> BoxFuture<'a, Result<Value, SchemeError>> {
        Box::pin(async move {
            // Parse chain ID from CAIP-2 network
            let chain_id = parse_caip2(&requirements.network).ok_or_else(|| -> SchemeError {
                format!("Invalid EVM network: {}", requirements.network).into()
            })?;

            // Parse verifying contract (asset address)
            let verifying_contract: Address =
                requirements.asset.parse().map_err(|e| -> SchemeError {
                    format!("Invalid asset address '{}': {e}", requirements.asset).into()
                })?;

            // Extract EIP-712 domain params from extra
            let extra: ExactRequirementsExtra = serde_json::from_value(requirements.extra.clone())
                .map_err(|e| -> SchemeError {
                    format!("Missing EIP-712 domain params in extra: {e}").into()
                })?;

            // Create nonce and validity window
            let nonce = Self::create_nonce();
            let max_timeout = requirements.max_timeout_seconds.max(60);
            let (valid_after, valid_before) = Self::create_validity_window(max_timeout);

            let payer = self.payer_address();

            // Parse payTo address
            let pay_to: Address = requirements.pay_to.parse().map_err(|e| -> SchemeError {
                format!("Invalid payTo address '{}': {e}", requirements.pay_to).into()
            })?;

            // Parse amount
            let value =
                U256::from_str_radix(&requirements.amount, 10).map_err(|e| -> SchemeError {
                    format!("Invalid amount '{}': {e}", requirements.amount).into()
                })?;

            // Build EIP-712 struct
            let typed_data = TransferWithAuthorization {
                from: payer,
                to: pay_to,
                value,
                validAfter: U256::from(valid_after),
                validBefore: U256::from(valid_before),
                nonce,
            };

            // Build domain
            let domain = Self::build_domain(chain_id, verifying_contract, &extra);

            // Sign EIP-712 typed data
            let signing_hash = typed_data.eip712_signing_hash(&domain);
            let signature = self
                .signer
                .sign_hash(&signing_hash)
                .await
                .map_err(|e| -> SchemeError { format!("EIP-712 signing failed: {e}").into() })?;

            let sig_bytes = signature.as_bytes();
            let sig_hex = format!("0x{}", alloy_primitives::hex::encode(sig_bytes));

            // Build authorization
            let authorization = ExactAuthorization {
                from: format!("{payer:?}"),
                to: format!("{pay_to:?}"),
                value: requirements.amount.clone(),
                valid_after: valid_after.to_string(),
                valid_before: valid_before.to_string(),
                nonce: format!("{nonce:?}"),
            };

            // Build payload
            let payload = ExactPayload {
                authorization,
                signature: sig_hex,
            };

            serde_json::to_value(&payload)
                .map_err(|e| -> SchemeError { format!("Serialize payload: {e}").into() })
        })
    }
}
