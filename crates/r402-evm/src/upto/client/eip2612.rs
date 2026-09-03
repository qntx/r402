//! Client-side EIP-712 signing for the EIP-2612 gas-sponsoring extension.

use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_sol_types::{Eip712Domain, SolStruct, eip712_domain, sol};
use r402_protocol::error::ClientError;

use crate::chain::TokenAmount;
use crate::eip2612::{EIP2612_GAS_SPONSORING_VERSION, Eip2612SignedPermit};
use crate::permit2::PERMIT2_ADDRESS;
use crate::signer::SignerLike;

sol! {
    /// EIP-2612 typed-data layout (`Permit(...)`).
    #[derive(Debug)]
    struct Eip2612TypedData {
        address owner;
        address spender;
        uint256 value;
        uint256 nonce;
        uint256 deadline;
    }
}

/// Inputs required to sign an EIP-2612 permit for the upto scheme.
///
/// `spender` is always [`PERMIT2_ADDRESS`]. `value` MUST equal the Permit2-signed maximum.
#[derive(Debug, Clone)]
pub struct Eip2612SigningParams {
    /// Numeric EIP-155 chain ID (not CAIP-2).
    pub chain_id: u64,
    /// ERC-20 token granting the allowance.
    pub asset_address: Address,
    /// Token's EIP-712 domain `name` field.
    pub token_name: String,
    /// Token's EIP-712 domain `version` field.
    pub token_version: String,
    /// Permit owner address (== buyer).
    pub owner: Address,
    /// Approved allowance — MUST equal the Permit2-signed maximum.
    pub value: U256,
    /// Owner's permit nonce on the token.
    pub nonce: U256,
    /// Permit deadline (Unix seconds).
    pub deadline: U256,
}

/// Signs an EIP-2612 permit using the buyer's wallet.
///
/// # Errors
///
/// Returns [`ClientError::Signing`] when the wallet refuses to sign.
pub async fn sign_eip2612_permit<S: SignerLike + Sync>(
    signer: &S,
    params: &Eip2612SigningParams,
) -> Result<Eip2612SignedPermit, ClientError> {
    let domain = build_token_domain(params);
    let typed = Eip2612TypedData {
        owner: params.owner,
        spender: PERMIT2_ADDRESS,
        value: params.value,
        nonce: params.nonce,
        deadline: params.deadline,
    };
    let hash: B256 = typed.eip712_signing_hash(&domain);
    let signature = signer
        .sign_hash(&hash)
        .await
        .map_err(|e| ClientError::Signing(format!("{e:?}")))?;

    Ok(Eip2612SignedPermit {
        from: params.owner,
        asset: params.asset_address,
        spender: PERMIT2_ADDRESS,
        amount: TokenAmount::from(params.value),
        nonce: TokenAmount::from(params.nonce),
        deadline: TokenAmount::from(params.deadline),
        signature: Bytes::from(signature.as_bytes().to_vec()),
        version: EIP2612_GAS_SPONSORING_VERSION.into(),
    })
}

fn build_token_domain(params: &Eip2612SigningParams) -> Eip712Domain {
    eip712_domain! {
        name: params.token_name.clone(),
        version: params.token_version.clone(),
        chain_id: params.chain_id,
        verifying_contract: params.asset_address,
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::address;
    use alloy_signer_local::PrivateKeySigner;

    use super::*;

    fn fixture() -> Eip2612SigningParams {
        Eip2612SigningParams {
            chain_id: 8453,
            asset_address: address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
            token_name: "USD Coin".into(),
            token_version: "2".into(),
            owner: address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            value: U256::from(1_000_000_u64),
            nonce: U256::from(0u64),
            deadline: U256::from(1_700_000_000_u64),
        }
    }

    #[tokio::test]
    async fn sign_returns_canonical_65_byte_signature() {
        let signer = PrivateKeySigner::random();
        let mut params = fixture();
        params.owner = signer.address();
        let permit = sign_eip2612_permit(&signer, &params)
            .await
            .expect("signing succeeds");
        assert_eq!(permit.signature.len(), Eip2612SignedPermit::SIGNATURE_LEN);
        assert_eq!(permit.from, signer.address());
        assert_eq!(permit.asset, params.asset_address);
        assert_eq!(permit.spender, PERMIT2_ADDRESS);
        assert_eq!(permit.amount.0, params.value);
        assert_eq!(permit.nonce.0, params.nonce);
        assert_eq!(permit.version, EIP2612_GAS_SPONSORING_VERSION);
        assert_eq!(permit.deadline.0, params.deadline);
    }

    #[tokio::test]
    async fn signing_is_deterministic_for_fixed_inputs() {
        let signer = PrivateKeySigner::random();
        let mut params = fixture();
        params.owner = signer.address();
        let a = sign_eip2612_permit(&signer, &params).await.unwrap();
        let b = sign_eip2612_permit(&signer, &params).await.unwrap();
        assert_eq!(a.signature, b.signature);
    }

    #[tokio::test]
    async fn signing_uses_canonical_permit2_spender() {
        let signer = PrivateKeySigner::random();
        let mut params = fixture();
        params.owner = signer.address();
        let permit = sign_eip2612_permit(&signer, &params).await.unwrap();
        assert_eq!(permit.spender, PERMIT2_ADDRESS);
    }
}
