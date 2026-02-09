//! EVM facilitator-side "exact" scheme implementation.
//!
//! Implements [`SchemeFacilitator`] for the `exact` scheme. Verifies and
//! settles EIP-3009 `transferWithAuthorization` payments on EVM networks
//! using an alloy [`Provider`].
//!
//! Corresponds to Python SDK's `mechanisms/evm/exact/facilitator.py`.

use alloy_primitives::{Address, Bytes, U256};
use alloy_provider::Provider;
use alloy_sol_types::SolCall;
use r402::proto::{PaymentPayload, PaymentRequirements, SettleResponse, VerifyResponse};
use r402::scheme::{BoxFuture, SchemeFacilitator};
use serde_json::Value;

use crate::chain::NetworkConfig;
use crate::exact::types::{
    ExactPayload, ExactRequirementsExtra, SCHEME_EXACT, authorizationStateCall, balanceOfCall,
    transferWithAuthorizationCall,
};
use crate::networks::known_networks;

/// EVM facilitator implementation for the "exact" payment scheme.
///
/// Verifies EIP-3009 authorization signatures and settles payments by
/// calling `transferWithAuthorization` on-chain via an alloy [`Provider`].
///
/// # Type Parameters
///
/// - `P`: Provider type (e.g., `RootProvider`). Must implement
///   `Provider` for the default Ethereum network.
///
/// Corresponds to Python SDK's `ExactEvmScheme` in `exact/facilitator.py`.
pub struct ExactEvmFacilitator<P> {
    provider: P,
    signer_address: Address,
    #[allow(dead_code)]
    networks: Vec<NetworkConfig>,
}

impl<P> ExactEvmFacilitator<P>
where
    P: Provider + Send + Sync + 'static,
{
    /// Creates a new facilitator with the given provider and signer address.
    ///
    /// The `signer_address` is the facilitator's wallet that will submit
    /// settlement transactions.
    pub fn new(provider: P, signer_address: Address) -> Self {
        Self {
            provider,
            signer_address,
            networks: known_networks(),
        }
    }

    /// Creates a facilitator with custom network configurations.
    pub fn with_networks(
        provider: P,
        signer_address: Address,
        networks: Vec<NetworkConfig>,
    ) -> Self {
        Self {
            provider,
            signer_address,
            networks,
        }
    }

    /// Parses the inner [`ExactPayload`] from a [`PaymentPayload`].
    fn parse_exact_payload(payload: &PaymentPayload) -> Result<ExactPayload, String> {
        serde_json::from_value(payload.payload.clone())
            .map_err(|e| format!("Invalid exact payload: {e}"))
    }

    /// Verifies the payment payload (inner logic).
    async fn verify_inner(
        &self,
        payload: &PaymentPayload,
        requirements: &PaymentRequirements,
    ) -> VerifyResponse {
        let evm_payload = match Self::parse_exact_payload(payload) {
            Ok(p) => p,
            Err(e) => return VerifyResponse::invalid("invalid_payload", e),
        };

        let payer_str = evm_payload.authorization.from.clone();
        let payer: Address = match payer_str.parse() {
            Ok(a) => a,
            Err(e) => {
                return VerifyResponse::invalid_with_payer(
                    "invalid_payer_address",
                    e.to_string(),
                    &payer_str,
                );
            }
        };
        let _ = payer;

        if payload.accepted.scheme != SCHEME_EXACT {
            return VerifyResponse::invalid_with_payer(
                "unsupported_scheme",
                "Expected exact scheme",
                &payer_str,
            );
        }

        if payload.accepted.network != requirements.network {
            return VerifyResponse::invalid_with_payer(
                "network_mismatch",
                "Payload network does not match requirements",
                &payer_str,
            );
        }

        let _extra: ExactRequirementsExtra =
            match serde_json::from_value(requirements.extra.clone()) {
                Ok(e) => e,
                Err(_) => {
                    return VerifyResponse::invalid_with_payer(
                        "missing_eip712_domain",
                        "EIP-712 domain params missing in extra",
                        &payer_str,
                    );
                }
            };

        if !evm_payload
            .authorization
            .to
            .eq_ignore_ascii_case(&requirements.pay_to)
        {
            return VerifyResponse::invalid_with_payer(
                "recipient_mismatch",
                "Authorization recipient does not match payTo",
                &payer_str,
            );
        }

        let auth_amount: u128 = match evm_payload.authorization.value.parse() {
            Ok(a) => a,
            Err(_) => {
                return VerifyResponse::invalid_with_payer(
                    "invalid_amount",
                    "Cannot parse authorization value",
                    &payer_str,
                );
            }
        };
        let req_amount: u128 = match requirements.amount.parse() {
            Ok(a) => a,
            Err(_) => {
                return VerifyResponse::invalid_with_payer(
                    "invalid_required_amount",
                    "Cannot parse required amount",
                    &payer_str,
                );
            }
        };
        if auth_amount < req_amount {
            return VerifyResponse::invalid_with_payer(
                "insufficient_amount",
                "Authorization amount less than required",
                &payer_str,
            );
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let valid_before: u64 = evm_payload.authorization.valid_before.parse().unwrap_or(0);
        let valid_after: u64 = evm_payload
            .authorization
            .valid_after
            .parse()
            .unwrap_or(u64::MAX);

        if valid_before < now + 6 {
            return VerifyResponse::invalid_with_payer(
                "valid_before_expired",
                "Authorization has expired",
                &payer_str,
            );
        }

        if valid_after > now {
            return VerifyResponse::invalid_with_payer(
                "valid_after_future",
                "Authorization not yet valid",
                &payer_str,
            );
        }

        let asset_address: Address = match requirements.asset.parse() {
            Ok(a) => a,
            Err(_) => {
                return VerifyResponse::invalid_with_payer(
                    "invalid_asset_address",
                    "Cannot parse asset address",
                    &payer_str,
                );
            }
        };

        let nonce_bytes: [u8; 32] = match parse_hex_bytes32(&evm_payload.authorization.nonce) {
            Ok(b) => b,
            Err(_) => {
                return VerifyResponse::invalid_with_payer(
                    "invalid_nonce",
                    "Cannot parse nonce",
                    &payer_str,
                );
            }
        };

        // On-chain nonce check (best effort)
        if let Ok(true) = self
            .check_nonce_used(payer, nonce_bytes, asset_address)
            .await
        {
            return VerifyResponse::invalid_with_payer(
                "nonce_already_used",
                "EIP-3009 nonce has already been consumed",
                &payer_str,
            );
        }

        // On-chain balance check (best effort)
        if let Ok(balance) = self.get_balance(payer, asset_address).await {
            if balance < U256::from(auth_amount) {
                return VerifyResponse::invalid_with_payer(
                    "insufficient_balance",
                    "Payer balance is less than authorization value",
                    &payer_str,
                );
            }
        }

        VerifyResponse::valid(&payer_str)
    }

    /// Settles the payment on-chain (inner logic).
    async fn settle_inner(
        &self,
        payload: &PaymentPayload,
        requirements: &PaymentRequirements,
    ) -> SettleResponse {
        let network = payload.accepted.network.clone();

        // Re-verify
        let verify = self.verify_inner(payload, requirements).await;
        if !verify.is_valid {
            return SettleResponse::error(
                verify.invalid_reason.unwrap_or_default(),
                verify.invalid_message.unwrap_or_default(),
                &network,
            );
        }

        let evm_payload = match Self::parse_exact_payload(payload) {
            Ok(p) => p,
            Err(e) => return SettleResponse::error("invalid_payload", e, &network),
        };

        let payer_str = evm_payload.authorization.from.clone();
        let asset_address: Address = match requirements.asset.parse() {
            Ok(a) => a,
            Err(e) => {
                return SettleResponse::error("invalid_asset_address", e.to_string(), &network);
            }
        };

        let from: Address = match evm_payload.authorization.from.parse() {
            Ok(a) => a,
            Err(e) => {
                return SettleResponse::error("invalid_from_address", e.to_string(), &network);
            }
        };
        let to: Address = match evm_payload.authorization.to.parse() {
            Ok(a) => a,
            Err(e) => return SettleResponse::error("invalid_to_address", e.to_string(), &network),
        };
        let value = U256::from_str_radix(&evm_payload.authorization.value, 10).unwrap_or_default();
        let valid_after =
            U256::from_str_radix(&evm_payload.authorization.valid_after, 10).unwrap_or_default();
        let valid_before =
            U256::from_str_radix(&evm_payload.authorization.valid_before, 10).unwrap_or_default();

        let nonce_bytes: [u8; 32] = match parse_hex_bytes32(&evm_payload.authorization.nonce) {
            Ok(b) => b,
            Err(e) => return SettleResponse::error("invalid_nonce", e, &network),
        };

        let signature = match parse_hex_bytes(&evm_payload.signature) {
            Ok(b) => b,
            Err(e) => return SettleResponse::error("invalid_signature", e, &network),
        };

        // Build transferWithAuthorization calldata (bytes overload)
        let call = transferWithAuthorizationCall {
            from,
            to,
            value,
            validAfter: valid_after,
            validBefore: valid_before,
            nonce: nonce_bytes.into(),
            signature: Bytes::from(signature),
        };
        let calldata = call.abi_encode();

        // Submit transaction
        let tx = alloy_rpc_types_eth::TransactionRequest::default()
            .from(self.signer_address)
            .to(asset_address)
            .input(calldata.into());

        match self.provider.send_transaction(tx).await {
            Ok(pending) => match pending.get_receipt().await {
                Ok(receipt) => {
                    let tx_hash = format!("{:?}", receipt.transaction_hash);
                    if receipt.status() {
                        SettleResponse::success(&tx_hash, &network, &payer_str)
                    } else {
                        SettleResponse::error("transaction_failed", "On-chain revert", &network)
                    }
                }
                Err(e) => {
                    SettleResponse::error("transaction_receipt_failed", e.to_string(), &network)
                }
            },
            Err(e) => SettleResponse::error("transaction_send_failed", e.to_string(), &network),
        }
    }

    /// Checks if an EIP-3009 nonce has been used on-chain.
    async fn check_nonce_used(
        &self,
        authorizer: Address,
        nonce: [u8; 32],
        token: Address,
    ) -> Result<bool, String> {
        let call = authorizationStateCall {
            authorizer,
            nonce: nonce.into(),
        };
        let calldata = Bytes::from(call.abi_encode());

        let tx = alloy_rpc_types_eth::TransactionRequest::default()
            .to(token)
            .input(calldata.into());

        let result: Bytes = self
            .provider
            .call(tx)
            .await
            .map_err(|e| e.to_string())?;

        Ok(result.len() >= 32 && result[31] != 0)
    }

    /// Gets the ERC-20 balance of an address.
    async fn get_balance(&self, account: Address, token: Address) -> Result<U256, String> {
        let call = balanceOfCall { account };
        let calldata = Bytes::from(call.abi_encode());

        let tx = alloy_rpc_types_eth::TransactionRequest::default()
            .to(token)
            .input(calldata.into());

        let result: Bytes = self
            .provider
            .call(tx)
            .await
            .map_err(|e| e.to_string())?;

        if result.len() >= 32 {
            Ok(U256::from_be_slice(&result[..32]))
        } else {
            Err("Invalid balance response".to_owned())
        }
    }
}

impl<P> std::fmt::Debug for ExactEvmFacilitator<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExactEvmFacilitator")
            .field("signer_address", &self.signer_address)
            .field("networks_count", &self.networks.len())
            .finish_non_exhaustive()
    }
}

impl<P> SchemeFacilitator for ExactEvmFacilitator<P>
where
    P: Provider + Send + Sync + 'static,
{
    fn scheme(&self) -> &str {
        SCHEME_EXACT
    }

    fn caip_family(&self) -> &str {
        "eip155:*"
    }

    fn get_extra(&self, _network: &str) -> Option<Value> {
        None
    }

    fn get_signers(&self, _network: &str) -> Vec<String> {
        vec![format!("{:?}", self.signer_address)]
    }

    fn verify<'a>(
        &'a self,
        payload: &'a PaymentPayload,
        requirements: &'a PaymentRequirements,
    ) -> BoxFuture<'a, VerifyResponse> {
        Box::pin(self.verify_inner(payload, requirements))
    }

    fn settle<'a>(
        &'a self,
        payload: &'a PaymentPayload,
        requirements: &'a PaymentRequirements,
    ) -> BoxFuture<'a, SettleResponse> {
        Box::pin(self.settle_inner(payload, requirements))
    }
}

/// Parses a `0x`-prefixed hex string into a 32-byte array.
fn parse_hex_bytes32(hex: &str) -> Result<[u8; 32], String> {
    let clean = hex.strip_prefix("0x").unwrap_or(hex);
    let bytes = alloy_primitives::hex::decode(clean).map_err(|e| format!("Invalid hex: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!("Expected 32 bytes, got {}", bytes.len()));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

/// Parses a `0x`-prefixed hex string into a byte vector.
fn parse_hex_bytes(hex: &str) -> Result<Vec<u8>, String> {
    let clean = hex.strip_prefix("0x").unwrap_or(hex);
    alloy_primitives::hex::decode(clean).map_err(|e| format!("Invalid hex: {e}"))
}
