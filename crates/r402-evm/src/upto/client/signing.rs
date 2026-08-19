//! EIP-712 signing for the upto scheme's Permit2 `PermitWitnessTransferFrom`.

use alloy_primitives::{Address, U256};
use alloy_sol_types::{SolStruct, eip712_domain};
use r402_core::error::ClientError;
use r402_core::wire::UnixTimestamp;
use rand::RngExt;
use rand::rng;

use crate::chain::TokenAmount;
use crate::permit2::{PERMIT2_ADDRESS, Permit2TokenPermissions};
use crate::signer::SignerLike;
use crate::upto::{
    PermitWitnessTransferFrom as SolPermitWitnessTransferFrom,
    TokenPermissions as SolTokenPermissions, UptoPermit2Authorization, UptoPermit2Payload,
    UptoPermit2Witness, Witness as SolWitness, X402_UPTO_PERMIT2_PROXY,
};

/// Signing parameters for a Permit2 upto authorization.
#[derive(Debug, Clone, Copy)]
pub struct Permit2UptoSigningParams {
    /// Numeric EIP-155 chain ID (not CAIP-2).
    pub chain_id: u64,
    /// Token contract address.
    pub asset_address: Address,
    /// Recipient address for the transfer.
    pub pay_to: Address,
    /// Facilitator address authorised in the witness. The on-chain proxy
    /// enforces `msg.sender == witness.facilitator`, so this MUST equal an
    /// address listed in the facilitator's `/supported` response and MUST
    /// be the same address that ultimately submits the settle transaction.
    pub facilitator_address: Address,
    /// **Maximum** amount the buyer authorises. The facilitator may settle
    /// for any value in `[0, max_amount]`.
    pub max_amount: U256,
    /// Maximum time in seconds the authorization remains valid.
    pub max_timeout_seconds: u64,
}

/// Signs a Permit2 `PermitWitnessTransferFrom` for the upto scheme using EIP-712.
///
/// Constructs the Permit2 domain (`name = "Permit2"`,
/// `verifyingContract = PERMIT2_ADDRESS`), embeds the facilitator address
/// into `witness.facilitator` (the on-chain proxy reverts with
/// `UnauthorizedFacilitator` when the settle caller doesn't match), and
/// returns a payload ready to serialise into the `PAYMENT-SIGNATURE`
/// header.
///
/// # Errors
///
/// Returns [`ClientError::Signing`] when hash signing fails.
pub async fn sign_permit2_upto_authorization<S: SignerLike + Sync>(
    signer: &S,
    params: &Permit2UptoSigningParams,
) -> Result<UptoPermit2Payload, ClientError> {
    let domain = eip712_domain! {
        name: "Permit2",
        chain_id: params.chain_id,
        verifying_contract: PERMIT2_ADDRESS,
    };

    let now = UnixTimestamp::now();
    // `validAfter` is shifted 10 minutes into the past to absorb mild clock
    // skew between the buyer's host and the facilitator chain node
    // (matches the official TypeScript / Go reference clients).
    let valid_after_secs = now.as_secs().saturating_sub(10 * 60);
    let deadline_secs = now.as_secs() + params.max_timeout_seconds;

    // Permit2 nonce is a fresh uint256; we draw 32 random bytes.
    let nonce_bytes: [u8; 32] = rng().random();
    let nonce = U256::from_be_bytes(nonce_bytes);

    let permit_witness = SolPermitWitnessTransferFrom {
        permitted: SolTokenPermissions {
            token: params.asset_address,
            amount: params.max_amount,
        },
        spender: X402_UPTO_PERMIT2_PROXY,
        nonce,
        deadline: U256::from(deadline_secs),
        witness: SolWitness {
            to: params.pay_to,
            facilitator: params.facilitator_address,
            validAfter: U256::from(valid_after_secs),
        },
    };

    let eip712_hash = permit_witness.eip712_signing_hash(&domain);
    let signature = signer
        .sign_hash(&eip712_hash)
        .await
        .map_err(|e| ClientError::Signing(format!("{e:?}")))?;

    let authorization = UptoPermit2Authorization {
        from: signer.address(),
        permitted: Permit2TokenPermissions {
            token: params.asset_address,
            amount: TokenAmount::from(params.max_amount),
        },
        spender: X402_UPTO_PERMIT2_PROXY,
        nonce: TokenAmount::from(nonce),
        deadline: TokenAmount::from(U256::from(deadline_secs)),
        witness: UptoPermit2Witness {
            to: params.pay_to,
            facilitator: params.facilitator_address,
            valid_after: TokenAmount::from(U256::from(valid_after_secs)),
        },
    };

    Ok(UptoPermit2Payload {
        signature: signature.as_bytes().into(),
        permit2_authorization: authorization,
    })
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;
    use alloy_signer_local::PrivateKeySigner;

    use super::*;

    #[tokio::test]
    async fn sign_produces_payload_with_expected_shape() {
        let signer = PrivateKeySigner::random();
        let facilitator = Address::repeat_byte(0xFA);
        let params = Permit2UptoSigningParams {
            chain_id: 8453,
            asset_address: Address::repeat_byte(0xAA),
            pay_to: Address::repeat_byte(0xBB),
            facilitator_address: facilitator,
            max_amount: U256::from(5_000_000_u64),
            max_timeout_seconds: 300,
        };
        let payload = sign_permit2_upto_authorization(&signer, &params)
            .await
            .unwrap();
        let auth = &payload.permit2_authorization;
        assert_eq!(auth.from, signer.address());
        assert_eq!(auth.spender, X402_UPTO_PERMIT2_PROXY);
        assert_eq!(auth.permitted.token, params.asset_address);
        assert_eq!(auth.permitted.amount.0, params.max_amount);
        assert_eq!(auth.witness.to, params.pay_to);
        assert_eq!(auth.witness.facilitator, facilitator);
        // 65-byte EOA signature.
        assert_eq!(payload.signature.len(), 65);
    }

    #[tokio::test]
    async fn two_signs_use_distinct_nonces() {
        let signer = PrivateKeySigner::random();
        let params = Permit2UptoSigningParams {
            chain_id: 1,
            asset_address: Address::ZERO,
            pay_to: Address::ZERO,
            facilitator_address: Address::ZERO,
            max_amount: U256::from(1_u64),
            max_timeout_seconds: 60,
        };
        let a = sign_permit2_upto_authorization(&signer, &params)
            .await
            .unwrap();
        let b = sign_permit2_upto_authorization(&signer, &params)
            .await
            .unwrap();
        assert_ne!(
            a.permit2_authorization.nonce, b.permit2_authorization.nonce,
            "Permit2 nonces MUST be unique across signatures"
        );
    }
}
