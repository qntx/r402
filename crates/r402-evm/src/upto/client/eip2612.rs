//! Client-side EIP-712 signing for the EIP-2612 gas-sponsoring extension.

use std::sync::Arc;

use alloy_consensus::{SignableTransaction, TxEip1559, TxEnvelope};
use alloy_eips::eip2718::Encodable2718;
use alloy_eips::eip2930::AccessList;
use alloy_primitives::{Address, B256, Bytes, TxKind, U256, hex};
use alloy_sol_types::{Eip712Domain, SolStruct, eip712_domain, sol};
use r402_protocol::error::ClientError;
use r402_protocol::payment::Extensions;

use crate::chain::TokenAmount;
use crate::eip2612::{
    EIP2612_GAS_SPONSORING_KEY, EIP2612_GAS_SPONSORING_VERSION, Eip2612SignedPermit,
};
use crate::erc20_approval::{
    DEFAULT_MAX_FEE_PER_GAS, DEFAULT_MAX_PRIORITY_FEE_PER_GAS, ERC20_APPROVAL_GAS_SPONSORING_KEY,
    ERC20_APPROVAL_GAS_SPONSORING_VERSION, ERC20_APPROVE_GAS_LIMIT, Erc20ApprovalGasSponsoringInfo,
};
use crate::permit2::{PERMIT2_ADDRESS, Permit2Approver, permit2_approval_calldata};
use crate::signer::SignerLike;

sol! {
    /// EIP-2612 typed-data layout. Identifier MUST be `Permit` so alloy
    /// `SolStruct::NAME` matches the token's on-chain typehash.
    #[derive(Debug)]
    struct Permit {
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
    let typed = Permit {
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

/// Official `trySignEip2612PermitExtension`: no-op unless advertised, then name/version, then allowance.
#[allow(
    clippy::too_many_arguments,
    reason = "mirrors official trySignEip2612PermitExtension argument list"
)]
pub(crate) async fn try_sign_eip2612_permit_extension<S: SignerLike + Sync>(
    signer: &S,
    approver: Option<&Arc<dyn Permit2Approver>>,
    advertised: &Extensions,
    token_name: &str,
    token_version: &str,
    chain_id: u64,
    asset: Address,
    required_allowance: U256,
    deadline: U256,
) -> Result<Option<Eip2612SignedPermit>, ClientError> {
    let Some(approver) = approver else {
        return Ok(None);
    };
    if !approver.supports_gas_sponsoring_rpc() {
        return Ok(None);
    }
    if advertised.get(EIP2612_GAS_SPONSORING_KEY).is_none() {
        return Ok(None);
    }
    if token_name.is_empty() || token_version.is_empty() {
        return Ok(None);
    }

    let owner = signer.address();
    match approver.check_permit2_allowance(asset, owner).await {
        Ok(allowance) if allowance >= required_allowance => return Ok(None),
        Ok(_) | Err(_) => {}
    }

    let nonce = approver.eip2612_nonce(asset, owner).await?;
    let params = Eip2612SigningParams {
        chain_id,
        asset_address: asset,
        token_name: token_name.to_owned(),
        token_version: token_version.to_owned(),
        owner,
        value: required_allowance,
        nonce,
        deadline,
    };
    let permit = sign_eip2612_permit(signer, &params).await?;
    Ok(Some(permit))
}

/// Official `trySignErc20ApprovalExtension` fallback after EIP-2612 `None`.
pub(crate) async fn try_sign_erc20_approval_extension<S: SignerLike + Sync>(
    signer: &S,
    approver: Option<&Arc<dyn Permit2Approver>>,
    advertised: &Extensions,
    chain_id: u64,
    asset: Address,
    required_allowance: U256,
) -> Result<Option<Erc20ApprovalGasSponsoringInfo>, ClientError> {
    let Some(approver) = approver else {
        return Ok(None);
    };
    if !approver.supports_gas_sponsoring_rpc() {
        return Ok(None);
    }
    if advertised.get(ERC20_APPROVAL_GAS_SPONSORING_KEY).is_none() {
        return Ok(None);
    }
    if !signer.signs_eip1559() {
        return Ok(None);
    }

    let owner = signer.address();
    match approver.check_permit2_allowance(asset, owner).await {
        Ok(allowance) if allowance >= required_allowance => return Ok(None),
        Ok(_) | Err(_) => {}
    }

    let nonce = approver.transaction_count(owner).await?;
    let (max_fee_per_gas, max_priority_fee_per_gas) = approver
        .estimate_fees_per_gas()
        .await
        .unwrap_or((DEFAULT_MAX_FEE_PER_GAS, DEFAULT_MAX_PRIORITY_FEE_PER_GAS));
    let (_token, calldata) = permit2_approval_calldata(asset);
    let mut tx = TxEip1559 {
        chain_id,
        nonce,
        gas_limit: ERC20_APPROVE_GAS_LIMIT,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        to: TxKind::Call(asset),
        value: U256::ZERO,
        access_list: AccessList::default(),
        input: calldata,
    };
    let Some(sig) = signer
        .sign_eip1559(&mut tx)
        .await
        .map_err(|e| ClientError::Signing(format!("{e:?}")))?
    else {
        return Ok(None);
    };
    let signed = TxEnvelope::Eip1559(tx.into_signed(sig)).encoded_2718();
    Ok(Some(Erc20ApprovalGasSponsoringInfo {
        from: owner,
        asset,
        spender: PERMIT2_ADDRESS,
        amount: U256::MAX.to_string(),
        signed_transaction: format!("0x{}", hex::encode(signed)),
        version: ERC20_APPROVAL_GAS_SPONSORING_VERSION.into(),
    }))
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
    use std::future::Future;

    use alloy_primitives::{address, keccak256};
    use alloy_signer_local::PrivateKeySigner;

    use super::*;

    #[test]
    fn eip2612_encode_type_is_official_permit() {
        let typed = Permit {
            owner: Address::ZERO,
            spender: Address::ZERO,
            value: U256::ZERO,
            nonce: U256::ZERO,
            deadline: U256::ZERO,
        };
        assert_eq!(
            Permit::eip712_encode_type(),
            "Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)"
        );
        assert_eq!(
            typed.eip712_type_hash(),
            keccak256(
                b"Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)"
            )
        );
    }

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

    struct MockApprover {
        allowance: Result<U256, String>,
        nonce: U256,
        tx_count: u64,
    }

    impl Permit2Approver for MockApprover {
        fn check_permit2_allowance(
            &self,
            _token: Address,
            _owner: Address,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<U256, ClientError>> + Send + '_>>
        {
            let result = self
                .allowance
                .clone()
                .map_err(ClientError::PreConditionFailed);
            Box::pin(async move { result })
        }

        fn approve_permit2(
            &self,
            _token: Address,
            _owner: Address,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), ClientError>> + Send + '_>> {
            Box::pin(async { Ok(()) })
        }

        fn supports_gas_sponsoring_rpc(&self) -> bool {
            true
        }

        fn eip2612_nonce(
            &self,
            _token: Address,
            _owner: Address,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<U256, ClientError>> + Send + '_>>
        {
            let nonce = self.nonce;
            Box::pin(async move { Ok(nonce) })
        }

        fn transaction_count(
            &self,
            _owner: Address,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<u64, ClientError>> + Send + '_>> {
            let n = self.tx_count;
            Box::pin(async move { Ok(n) })
        }

        fn estimate_fees_per_gas(
            &self,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<(u128, u128), ClientError>> + Send + '_>>
        {
            Box::pin(async { Ok((1_000_000_000, 100_000_000)) })
        }
    }

    fn advertised(key: &str) -> Extensions {
        let mut ext = Extensions::new();
        ext.insert(
            key,
            r402_protocol::payment::ExtensionEntry::info(serde_json::json!({})),
        );
        ext
    }

    #[tokio::test]
    async fn try_sign_eip2612_noops_without_advertisement() {
        let signer = PrivateKeySigner::random();
        let approver: Arc<dyn Permit2Approver> = Arc::new(MockApprover {
            allowance: Ok(U256::ZERO),
            nonce: U256::ZERO,
            tx_count: 0,
        });
        let out = try_sign_eip2612_permit_extension(
            &signer,
            Some(&approver),
            &Extensions::new(),
            "USD Coin",
            "2",
            8453,
            Address::repeat_byte(0xAA),
            U256::from(1_000_000u64),
            U256::from(1_700_000_000u64),
        )
        .await
        .unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn try_sign_eip2612_noops_without_name_version() {
        let signer = PrivateKeySigner::random();
        let approver: Arc<dyn Permit2Approver> = Arc::new(MockApprover {
            allowance: Ok(U256::ZERO),
            nonce: U256::ZERO,
            tx_count: 0,
        });
        let out = try_sign_eip2612_permit_extension(
            &signer,
            Some(&approver),
            &advertised(EIP2612_GAS_SPONSORING_KEY),
            "",
            "2",
            8453,
            Address::repeat_byte(0xAA),
            U256::from(1_000_000u64),
            U256::from(1_700_000_000u64),
        )
        .await
        .unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn try_sign_eip2612_noops_when_allowance_sufficient() {
        let signer = PrivateKeySigner::random();
        let required = U256::from(1_000_000u64);
        let approver: Arc<dyn Permit2Approver> = Arc::new(MockApprover {
            allowance: Ok(required),
            nonce: U256::ZERO,
            tx_count: 0,
        });
        let out = try_sign_eip2612_permit_extension(
            &signer,
            Some(&approver),
            &advertised(EIP2612_GAS_SPONSORING_KEY),
            "USD Coin",
            "2",
            8453,
            Address::repeat_byte(0xAA),
            required,
            U256::from(1_700_000_000u64),
        )
        .await
        .unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn try_sign_eip2612_attaches_when_advertised_and_allowance_low() {
        let signer = PrivateKeySigner::random();
        let approver: Arc<dyn Permit2Approver> = Arc::new(MockApprover {
            allowance: Ok(U256::ZERO),
            nonce: U256::from(7u64),
            tx_count: 0,
        });
        let permit = try_sign_eip2612_permit_extension(
            &signer,
            Some(&approver),
            &advertised(EIP2612_GAS_SPONSORING_KEY),
            "USD Coin",
            "2",
            8453,
            Address::repeat_byte(0xAA),
            U256::from(1_000_000u64),
            U256::from(1_700_000_000u64),
        )
        .await
        .unwrap()
        .expect("signed permit");
        assert_eq!(permit.from, signer.address());
        assert_eq!(permit.nonce.0, U256::from(7u64));
        assert_eq!(permit.spender, PERMIT2_ADDRESS);
        assert_eq!(permit.signature.len(), Eip2612SignedPermit::SIGNATURE_LEN);
    }

    #[tokio::test]
    async fn try_sign_eip2612_proceeds_when_allowance_rpc_fails() {
        let signer = PrivateKeySigner::random();
        let approver: Arc<dyn Permit2Approver> = Arc::new(MockApprover {
            allowance: Err("rpc down".into()),
            nonce: U256::ZERO,
            tx_count: 0,
        });
        let permit = try_sign_eip2612_permit_extension(
            &signer,
            Some(&approver),
            &advertised(EIP2612_GAS_SPONSORING_KEY),
            "USD Coin",
            "2",
            8453,
            Address::repeat_byte(0xAA),
            U256::from(1_000_000u64),
            U256::from(1_700_000_000u64),
        )
        .await
        .unwrap()
        .expect("official catch proceeds to sign");
        assert_eq!(permit.from, signer.address());
    }

    #[tokio::test]
    async fn try_sign_erc20_noops_without_advertisement() {
        let signer = PrivateKeySigner::random();
        let approver: Arc<dyn Permit2Approver> = Arc::new(MockApprover {
            allowance: Ok(U256::ZERO),
            nonce: U256::ZERO,
            tx_count: 0,
        });
        let out = try_sign_erc20_approval_extension(
            &signer,
            Some(&approver),
            &Extensions::new(),
            8453,
            Address::repeat_byte(0xAA),
            U256::from(1_000_000u64),
        )
        .await
        .unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn try_sign_erc20_attaches_when_advertised_and_allowance_low() {
        let signer = PrivateKeySigner::random();
        let approver: Arc<dyn Permit2Approver> = Arc::new(MockApprover {
            allowance: Ok(U256::ZERO),
            nonce: U256::ZERO,
            tx_count: 3,
        });
        let info = try_sign_erc20_approval_extension(
            &signer,
            Some(&approver),
            &advertised(ERC20_APPROVAL_GAS_SPONSORING_KEY),
            8453,
            Address::repeat_byte(0xAA),
            U256::from(1_000_000u64),
        )
        .await
        .unwrap()
        .expect("signed approve tx");
        assert_eq!(info.from, signer.address());
        assert_eq!(info.spender, PERMIT2_ADDRESS);
        assert_eq!(info.amount, U256::MAX.to_string());
        assert!(info.signed_transaction.starts_with("0x"));
    }
}
