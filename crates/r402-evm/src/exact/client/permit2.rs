//! Permit2 signing for the EIP-155 exact scheme.

use alloy_primitives::{Address, U256};
use alloy_sol_types::{SolStruct, eip712_domain};
use r402_protocol::error::ClientError;
use r402_protocol::payment::UnixTimestamp;
use rand::{RngExt, rng};

use super::{SolTokenPermissions, SolWitness};
use crate::chain::TokenAmount;
use crate::exact::{
    Permit2Authorization, Permit2Payload, Permit2Witness, PermitWitnessTransferFrom,
    X402_EXACT_PERMIT2_PROXY,
};
use crate::permit2::{PERMIT2_ADDRESS, Permit2TokenPermissions};
use crate::signer::SignerLike;

/// Shared signing parameters for Permit2 authorization.
#[derive(Debug, Clone, Copy)]
pub struct Permit2SigningParams {
    /// The EIP-155 chain ID (numeric)
    pub chain_id: u64,
    /// The token contract address
    pub asset_address: Address,
    /// The recipient address for the transfer
    pub pay_to: Address,
    /// The amount to transfer (in token units)
    pub amount: U256,
    /// Maximum timeout in seconds for the authorization validity window
    pub max_timeout_seconds: u64,
}

/// Signs a Permit2 `PermitWitnessTransferFrom` using EIP-712.
///
/// Constructs the Permit2 EIP-712 domain (name = "Permit2", no version,
/// verifying contract = canonical Permit2 address), builds the authorization
/// with timing parameters, and signs the resulting hash.
///
/// # Errors
///
/// Returns [`ClientError`] if EIP-712 signing fails.
pub async fn sign_permit2_authorization<S: SignerLike + Sync>(
    signer: &S,
    params: &Permit2SigningParams,
) -> Result<Permit2Payload, ClientError> {
    let domain = eip712_domain! {
        name: "Permit2",
        chain_id: params.chain_id,
        verifying_contract: PERMIT2_ADDRESS,
    };

    let now = UnixTimestamp::now();
    let valid_after_secs = now.as_secs().saturating_sub(10 * 60);
    let deadline_secs = now.as_secs() + params.max_timeout_seconds;

    // Permit2 uses uint256 nonce (random 32 bytes interpreted as uint256)
    let nonce_bytes: [u8; 32] = rng().random();
    let nonce = U256::from_be_bytes(nonce_bytes);

    let permit_witness = PermitWitnessTransferFrom {
        permitted: SolTokenPermissions {
            token: params.asset_address,
            amount: params.amount,
        },
        spender: X402_EXACT_PERMIT2_PROXY,
        nonce,
        deadline: U256::from(deadline_secs),
        witness: SolWitness {
            to: params.pay_to,
            validAfter: U256::from(valid_after_secs),
        },
    };

    let eip712_hash = permit_witness.eip712_signing_hash(&domain);
    let signature = signer
        .sign_hash(&eip712_hash)
        .await
        .map_err(|e| ClientError::Signing(format!("{e:?}")))?;

    let authorization = Permit2Authorization {
        from: signer.address(),
        permitted: Permit2TokenPermissions {
            token: params.asset_address,
            amount: TokenAmount::from(params.amount),
        },
        spender: X402_EXACT_PERMIT2_PROXY,
        nonce: TokenAmount::from(nonce),
        deadline: TokenAmount::from(U256::from(deadline_secs)),
        witness: Permit2Witness {
            to: params.pay_to,
            valid_after: TokenAmount::from(U256::from(valid_after_secs)),
        },
    };

    Ok(Permit2Payload {
        signature: signature.as_bytes().into(),
        permit2_authorization: authorization,
    })
}
