//! EVM facilitator-side "exact" scheme implementation.
//!
//! Implements [`SchemeFacilitator`] for the `exact` scheme. Verifies and
//! settles EIP-3009 `transferWithAuthorization` payments on EVM networks
//! using an alloy [`Provider`].
//!
//! Corresponds to Python SDK's `mechanisms/evm/exact/facilitator.py`.

use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_provider::Provider;
use alloy_sol_types::{Eip712Domain, SolCall, SolStruct, SolType, sol_data};
use r402::proto::{PaymentPayload, PaymentRequirements, SettleResponse, VerifyResponse};
use r402::scheme::{BoxFuture, SchemeFacilitator};
use serde_json::Value;

use crate::chain::{NetworkConfig, parse_caip2};
use crate::exact::types::{
    ExactPayload, ExactRequirementsExtra, SCHEME_EXACT, TransferWithAuthorization,
    authorizationStateCall, balanceOfCall, isValidSignatureCall, transferWithAuthorizationCall,
    transferWithAuthorizationVRSCall,
};
use crate::networks::known_networks;

/// Configuration options for the EVM exact scheme facilitator.
///
/// Corresponds to Python SDK's `ExactEvmSchemeConfig` in `exact/facilitator.py`.
#[derive(Debug, Clone, Copy)]
pub struct ExactEvmConfig {
    /// Whether to deploy ERC-4337 smart wallets via ERC-6492 factory calls
    /// during settlement. When `false`, payments from undeployed smart wallets
    /// will be rejected at settlement time.
    ///
    /// Default: `false`.
    pub deploy_erc4337_with_eip6492: bool,
}

impl Default for ExactEvmConfig {
    fn default() -> Self {
        Self {
            deploy_erc4337_with_eip6492: false,
        }
    }
}

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
    config: ExactEvmConfig,
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
    /// settlement transactions. Uses default configuration.
    pub fn new(provider: P, signer_address: Address) -> Self {
        Self {
            provider,
            signer_address,
            config: ExactEvmConfig::default(),
            networks: known_networks(),
        }
    }

    /// Creates a facilitator with custom configuration.
    pub fn with_config(provider: P, signer_address: Address, config: ExactEvmConfig) -> Self {
        Self {
            provider,
            signer_address,
            config,
            networks: known_networks(),
        }
    }

    /// Creates a facilitator with custom network configurations and config.
    pub fn with_networks(
        provider: P,
        signer_address: Address,
        config: ExactEvmConfig,
        networks: Vec<NetworkConfig>,
    ) -> Self {
        Self {
            provider,
            signer_address,
            config,
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

        let extra: ExactRequirementsExtra = match serde_json::from_value(requirements.extra.clone())
        {
            Ok(e) => e,
            Err(_) => {
                return VerifyResponse::invalid_with_payer(
                    "missing_eip712_domain",
                    "EIP-712 domain params missing in extra",
                    &payer_str,
                );
            }
        };

        // Resolve chain ID from CAIP-2 network identifier
        let chain_id = match parse_caip2(&requirements.network) {
            Some(id) => id,
            None => {
                return VerifyResponse::invalid_with_payer(
                    "invalid_network",
                    format!("Cannot parse CAIP-2 network: {}", requirements.network),
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

        // Verify EIP-712 signature (EOA ecrecover / EIP-1271 / ERC-6492)
        let raw_signature = match parse_hex_bytes(&evm_payload.signature) {
            Ok(b) => b,
            Err(e) => {
                return VerifyResponse::invalid_with_payer("invalid_signature", e, &payer_str);
            }
        };

        if raw_signature.is_empty() {
            return VerifyResponse::invalid_with_payer(
                "invalid_signature",
                "Empty signature",
                &payer_str,
            );
        }

        let domain = Self::build_eip712_domain(&extra, chain_id, asset_address);
        let eip712_hash = match Self::compute_eip712_hash(&evm_payload.authorization, &domain) {
            Ok(h) => h,
            Err(e) => {
                return VerifyResponse::invalid_with_payer("failed_to_compute_hash", e, &payer_str);
            }
        };

        match self
            .verify_signature(payer, eip712_hash, &raw_signature)
            .await
        {
            Ok((true, _)) => VerifyResponse::valid(&payer_str),
            Ok((false, _)) => VerifyResponse::invalid_with_payer(
                "invalid_signature",
                "Signature verification failed",
                &payer_str,
            ),
            Err(e) => {
                VerifyResponse::invalid_with_payer("failed_to_verify_signature", e, &payer_str)
            }
        }
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

        let raw_signature = match parse_hex_bytes(&evm_payload.signature) {
            Ok(b) => b,
            Err(e) => return SettleResponse::error("invalid_signature", e, &network),
        };

        // Parse ERC-6492 wrapper to detect smart wallet deployment needs
        let sig_data = parse_erc6492_signature(&raw_signature);

        // Deploy smart wallet if needed (ERC-6492)
        if has_deployment_info(&sig_data) {
            let code = self.get_code(from).await.unwrap_or_default();
            if code.is_empty() {
                if !self.config.deploy_erc4337_with_eip6492 {
                    return SettleResponse::error(
                        "undeployed_smart_wallet",
                        "Smart wallet deployment is disabled by configuration",
                        &network,
                    );
                }
                // Smart wallet not deployed — attempt factory deployment
                let deploy_tx = alloy_rpc_types_eth::TransactionRequest::default()
                    .from(self.signer_address)
                    .to(sig_data.factory)
                    .input(sig_data.factory_calldata.clone().into());

                match self.provider.send_transaction(deploy_tx).await {
                    Ok(pending) => match pending.get_receipt().await {
                        Ok(receipt) => {
                            if !receipt.status() {
                                return SettleResponse::error(
                                    "smart_wallet_deployment_failed",
                                    "Factory call reverted",
                                    &network,
                                );
                            }
                        }
                        Err(e) => {
                            return SettleResponse::error(
                                "smart_wallet_deployment_failed",
                                e.to_string(),
                                &network,
                            );
                        }
                    },
                    Err(e) => {
                        return SettleResponse::error(
                            "smart_wallet_deployment_failed",
                            e.to_string(),
                            &network,
                        );
                    }
                }
            }
        }

        // Use inner signature (stripped of ERC-6492 wrapper) for settlement
        let inner_sig = &sig_data.inner_signature;

        // Build transferWithAuthorization calldata.
        // EOA (65-byte or 64-byte ERC-2098): decode to v,r,s and use the
        // VRS overload for maximum on-chain compatibility.
        // Smart wallet (other lengths): use bytes overload.
        let calldata = if let Ok((r_val, s_val, parity)) = decode_ecdsa_signature(inner_sig) {
            let r = B256::from(r_val.to_be_bytes::<32>());
            let s = B256::from(s_val.to_be_bytes::<32>());
            let v = if parity { 28u8 } else { 27u8 };
            let call = transferWithAuthorizationVRSCall {
                from,
                to,
                value,
                validAfter: valid_after,
                validBefore: valid_before,
                nonce: nonce_bytes.into(),
                v,
                r,
                s,
            };
            call.abi_encode()
        } else {
            let call = transferWithAuthorizationCall {
                from,
                to,
                value,
                validAfter: valid_after,
                validBefore: valid_before,
                nonce: nonce_bytes.into(),
                signature: Bytes::from(inner_sig.clone()),
            };
            call.abi_encode()
        };

        // Submit settlement transaction
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

        let result: Bytes = self.provider.call(tx).await.map_err(|e| e.to_string())?;

        Ok(result.len() >= 32 && result[31] != 0)
    }

    /// Gets the ERC-20 balance of an address.
    async fn get_balance(&self, account: Address, token: Address) -> Result<U256, String> {
        let call = balanceOfCall { account };
        let calldata = Bytes::from(call.abi_encode());

        let tx = alloy_rpc_types_eth::TransactionRequest::default()
            .to(token)
            .input(calldata.into());

        let result: Bytes = self.provider.call(tx).await.map_err(|e| e.to_string())?;

        if result.len() >= 32 {
            Ok(U256::from_be_slice(&result[..32]))
        } else {
            Err("Invalid balance response".to_owned())
        }
    }

    /// Gets the deployed bytecode at an address.
    ///
    /// Returns an empty `Bytes` if the address has no code (EOA).
    async fn get_code(&self, address: Address) -> Result<Bytes, String> {
        self.provider
            .get_code_at(address)
            .await
            .map_err(|e| e.to_string())
    }

    /// Verifies an ECDSA signature by recovering the signer address.
    ///
    /// Supports both standard 65-byte `(r, s, v)` and ERC-2098 compact
    /// 64-byte `(r, yParityAndS)` formats. Handles Ethereum v-value
    /// adjustment (27/28 → 0/1 parity).
    ///
    /// Corresponds to Python SDK's `verify_eoa_signature` in `verify.py`.
    fn verify_eoa_signature(
        hash: &B256,
        signature: &[u8],
        expected: Address,
    ) -> Result<bool, String> {
        let (r, s, parity) = decode_ecdsa_signature(signature)?;
        let sig = alloy_primitives::Signature::new(r, s, parity);
        match sig.recover_address_from_prehash(hash) {
            Ok(recovered) => Ok(recovered == expected),
            Err(_) => Ok(false),
        }
    }

    /// Verifies an EIP-1271 smart contract wallet signature.
    ///
    /// Calls `isValidSignature(bytes32,bytes)` on the wallet contract and
    /// checks the return value against the EIP-1271 magic value `0x1626ba7e`.
    ///
    /// Corresponds to Python SDK's `verify_eip1271_signature` in `verify.py`.
    async fn verify_eip1271_signature(
        &self,
        wallet: Address,
        hash: B256,
        signature: &[u8],
    ) -> Result<bool, String> {
        let call = isValidSignatureCall {
            hash,
            signature: Bytes::from(signature.to_vec()),
        };
        let calldata = Bytes::from(call.abi_encode());
        let tx = alloy_rpc_types_eth::TransactionRequest::default()
            .to(wallet)
            .input(calldata.into());

        let result: Bytes = self.provider.call(tx).await.map_err(|e| e.to_string())?;

        Ok(result.len() >= 4 && result[..4] == EIP1271_MAGIC_VALUE)
    }

    /// Builds the EIP-712 domain separator from requirements extra and chain info.
    fn build_eip712_domain(
        extra: &ExactRequirementsExtra,
        chain_id: u64,
        verifying_contract: Address,
    ) -> Eip712Domain {
        Eip712Domain {
            name: Some(extra.name.clone().into()),
            version: Some(extra.version.clone().into()),
            chain_id: Some(U256::from(chain_id)),
            verifying_contract: Some(verifying_contract),
            salt: None,
        }
    }

    /// Computes the EIP-712 signing hash for an ERC-3009 authorization.
    ///
    /// Corresponds to Python SDK's `hash_eip3009_authorization` in `eip712.py`.
    fn compute_eip712_hash(
        auth: &crate::exact::types::ExactAuthorization,
        domain: &Eip712Domain,
    ) -> Result<B256, String> {
        let from: Address = auth
            .from
            .parse()
            .map_err(|e| format!("Invalid from: {e}"))?;
        let to: Address = auth.to.parse().map_err(|e| format!("Invalid to: {e}"))?;
        let value =
            U256::from_str_radix(&auth.value, 10).map_err(|e| format!("Invalid value: {e}"))?;
        let valid_after = U256::from_str_radix(&auth.valid_after, 10)
            .map_err(|e| format!("Invalid validAfter: {e}"))?;
        let valid_before = U256::from_str_radix(&auth.valid_before, 10)
            .map_err(|e| format!("Invalid validBefore: {e}"))?;
        let nonce_bytes = parse_hex_bytes32(&auth.nonce)?;

        let typed_data = TransferWithAuthorization {
            from,
            to,
            value,
            validAfter: valid_after,
            validBefore: valid_before,
            nonce: B256::from(nonce_bytes),
        };

        Ok(typed_data.eip712_signing_hash(domain))
    }

    /// Unified signature verification supporting EOA, EIP-1271, and ERC-6492.
    ///
    /// Follows the same logic as Python SDK's `verify_universal_signature`:
    /// 1. Parse ERC-6492 wrapper if present.
    /// 2. If inner sig is 65 bytes with no factory → EOA ecrecover.
    /// 3. Check if contract is deployed on-chain.
    /// 4. If undeployed with factory info → accept (deployment in settle).
    /// 5. If undeployed without factory → fallback to EOA ecrecover.
    /// 6. If deployed → EIP-1271 `isValidSignature`.
    ///
    /// Corresponds to Python SDK's `verify_universal_signature` in `verify.py`.
    async fn verify_signature(
        &self,
        payer: Address,
        hash: B256,
        raw_signature: &[u8],
    ) -> Result<(bool, Erc6492SignatureData), String> {
        let sig_data = parse_erc6492_signature(raw_signature);

        // Fast path: plain 65-byte ECDSA with no factory → skip get_code
        if is_eoa_signature(&sig_data) {
            let valid = Self::verify_eoa_signature(&hash, &sig_data.inner_signature, payer)?;
            return Ok((valid, sig_data));
        }

        // Check if the payer contract is deployed
        let code = self.get_code(payer).await.unwrap_or_default();
        let is_deployed = !code.is_empty();

        if !is_deployed {
            if has_deployment_info(&sig_data) {
                // ERC-6492: undeployed smart wallet with factory — accept now,
                // deployment happens during settle.
                return Ok((true, sig_data));
            }
            // No factory info — try EOA verification as fallback (65-byte or 64-byte ERC-2098)
            let len = sig_data.inner_signature.len();
            if len == 65 || len == 64 {
                let valid = Self::verify_eoa_signature(&hash, &sig_data.inner_signature, payer)?;
                return Ok((valid, sig_data));
            }
            return Ok((false, sig_data));
        }

        // Deployed contract → EIP-1271 verification
        let valid = self
            .verify_eip1271_signature(payer, hash, &sig_data.inner_signature)
            .await
            .unwrap_or(false);
        Ok((valid, sig_data))
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

/// Decodes an ECDSA signature into `(r, s, parity)` components.
///
/// Supports two formats:
/// - **65-byte standard**: `r (32) || s (32) || v (1)` where v ∈ {0,1,27,28}
/// - **64-byte ERC-2098 compact**: `r (32) || yParityAndS (32)` where
///   `yParity` is encoded in the highest bit of `s`
///
/// Returns an error for any other length.
fn decode_ecdsa_signature(sig: &[u8]) -> Result<(U256, U256, bool), String> {
    match sig.len() {
        65 => {
            let r = U256::from_be_slice(&sig[..32]);
            let s = U256::from_be_slice(&sig[32..64]);
            let v = sig[64];
            let parity = if v >= 27 { v - 27 } else { v } != 0;
            Ok((r, s, parity))
        }
        64 => {
            let r = U256::from_be_slice(&sig[..32]);
            let y_parity = sig[32] >> 7;
            let mut s_bytes = [0u8; 32];
            s_bytes.copy_from_slice(&sig[32..64]);
            s_bytes[0] &= 0x7f; // clear the highest bit
            let s = U256::from_be_bytes(s_bytes);
            Ok((r, s, y_parity != 0))
        }
        other => Err(format!(
            "Invalid ECDSA signature length: expected 64 or 65, got {other}"
        )),
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

/// ERC-6492 magic suffix (32 bytes).
///
/// `bytes32(uint256(keccak256("erc6492.invalid.signature")) - 1)`
///
/// Corresponds to Python SDK's `ERC6492_MAGIC_VALUE` in `constants.py`.
const ERC6492_MAGIC_SUFFIX: [u8; 32] = [
    0x64, 0x92, 0x64, 0x92, 0x64, 0x92, 0x64, 0x92, 0x64, 0x92, 0x64, 0x92, 0x64, 0x92, 0x64, 0x92,
    0x64, 0x92, 0x64, 0x92, 0x64, 0x92, 0x64, 0x92, 0x64, 0x92, 0x64, 0x92, 0x64, 0x92, 0x64, 0x92,
];

/// EIP-1271 `isValidSignature` success magic value (`0x1626ba7e`).
const EIP1271_MAGIC_VALUE: [u8; 4] = [0x16, 0x26, 0xba, 0x7e];

/// Parsed ERC-6492 signature data.
///
/// If the original signature is not ERC-6492 wrapped, `factory` will be
/// the zero address and `factory_calldata` will be empty.
///
/// Corresponds to Python SDK's `ERC6492SignatureData` in `types.py`.
struct Erc6492SignatureData {
    factory: Address,
    factory_calldata: Vec<u8>,
    inner_signature: Vec<u8>,
}

/// Parses a potentially ERC-6492 wrapped signature.
///
/// ERC-6492 format: `abi.encode(address, bytes, bytes) + magicSuffix`.
/// If the signature does not end with the magic suffix, it is returned
/// as-is in `inner_signature`.
fn parse_erc6492_signature(signature: &[u8]) -> Erc6492SignatureData {
    if signature.len() > 32 && signature[signature.len() - 32..] == ERC6492_MAGIC_SUFFIX {
        let payload = &signature[..signature.len() - 32];
        type Erc6492Tuple = (sol_data::Address, sol_data::Bytes, sol_data::Bytes);
        if let Ok((factory, factory_calldata, inner_sig)) =
            <Erc6492Tuple as SolType>::abi_decode(payload)
        {
            return Erc6492SignatureData {
                factory,
                factory_calldata: factory_calldata.to_vec(),
                inner_signature: inner_sig.to_vec(),
            };
        }
    }
    Erc6492SignatureData {
        factory: Address::ZERO,
        factory_calldata: Vec::new(),
        inner_signature: signature.to_vec(),
    }
}

/// Returns `true` if the signature is a plain ECDSA signature from an EOA
/// (no ERC-6492 factory). Accepts both 65-byte standard and 64-byte
/// ERC-2098 compact formats.
fn is_eoa_signature(sig_data: &Erc6492SignatureData) -> bool {
    let len = sig_data.inner_signature.len();
    (len == 65 || len == 64) && sig_data.factory == Address::ZERO
}

/// Returns `true` if the signature contains smart wallet deployment info.
fn has_deployment_info(sig_data: &Erc6492SignatureData) -> bool {
    sig_data.factory != Address::ZERO && !sig_data.factory_calldata.is_empty()
}
