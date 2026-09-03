//! CEP-3009 / Casper EIP-712 digest construction.
//!
//! Domain and message encoding match `@casper-ecosystem/casper-eip-712`:
//! Casper `address` fields are `keccak256(tag ‖ hash33)`, not 20-byte
//! Ethereum addresses.

use sha3::{Digest as _, Keccak256};

use crate::chain::motes::Motes;
use crate::chain::{Address, ContractPackageHash};
use crate::exact::payload::ExactCasperAuthorization;

/// EIP-712 primary type name.
pub const PRIMARY_TYPE: &str = "TransferWithAuthorization";

/// The EIP-712 domain a Casper CEP-18 contract binds its authorisations to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Eip712Domain {
    /// Token contract name, from `requirements.extra.name`.
    pub name: String,
    /// Token contract version, from `requirements.extra.version`.
    pub version: String,
    /// CAIP-2 network identifier the payment targets (`chain_name`).
    pub network: String,
    /// CEP-18 contract package hash (`contract_package_hash`).
    pub verifying_contract: ContractPackageHash,
}

/// Type string for the Casper CEP-3009 domain (order is significant).
pub const CASPER_DOMAIN_TYPE: &str =
    "EIP712Domain(string name,string version,string chain_name,bytes32 contract_package_hash)";

/// Type string for `TransferWithAuthorization`.
pub const TRANSFER_WITH_AUTHORIZATION_TYPE: &str = "TransferWithAuthorization(address from,address to,uint256 value,uint256 validAfter,uint256 validBefore,bytes32 nonce)";

/// Computes the CEP-3009 EIP-712 digest for an authorization under `domain`.
#[must_use]
pub fn transfer_with_authorization_digest(
    domain: &Eip712Domain,
    authorization: &ExactCasperAuthorization,
) -> [u8; 32] {
    let domain_sep = hash_domain_separator(domain);
    let struct_hash = hash_transfer_with_authorization(authorization);
    hash_typed_data(&domain_sep, &struct_hash)
}

/// Hashes the Casper EIP-712 domain separator.
#[must_use]
pub fn hash_domain_separator(domain: &Eip712Domain) -> [u8; 32] {
    keccak256_concat(&[
        &keccak256(CASPER_DOMAIN_TYPE.as_bytes()),
        &encode_string(&domain.name),
        &encode_string(&domain.version),
        &encode_string(&domain.network),
        domain.verifying_contract.as_bytes(),
    ])
}

/// Hashes the `TransferWithAuthorization` struct.
#[must_use]
pub fn hash_transfer_with_authorization(auth: &ExactCasperAuthorization) -> [u8; 32] {
    // Nonce is already 32 bytes when well-formed; fall back to zero on
    // malformed input — callers validate before signing.
    let nonce = auth.nonce_bytes().unwrap_or([0u8; 32]);
    keccak256_concat(&[
        &keccak256(TRANSFER_WITH_AUTHORIZATION_TYPE.as_bytes()),
        &encode_casper_address(auth.from),
        &encode_casper_address(auth.to),
        &encode_motes(auth.value),
        &encode_u64(auth.valid_after.as_secs()),
        &encode_u64(auth.valid_before.as_secs()),
        &nonce,
    ])
}

/// Final EIP-712 digest: `\x19\x01 ‖ domainSeparator ‖ hashStruct`.
#[must_use]
pub fn hash_typed_data(domain_separator: &[u8; 32], struct_hash: &[u8; 32]) -> [u8; 32] {
    keccak256_concat(&[
        &[0x19, 0x01],
        domain_separator.as_slice(),
        struct_hash.as_slice(),
    ])
}

/// Builds the domain descriptor from requirements fields.
#[must_use]
pub fn domain_from_parts(
    name: impl Into<String>,
    version: impl Into<String>,
    network: impl Into<String>,
    package: ContractPackageHash,
) -> Eip712Domain {
    Eip712Domain {
        name: name.into(),
        version: version.into(),
        network: network.into(),
        verifying_contract: package,
    }
}

fn encode_casper_address(address: Address) -> [u8; 32] {
    keccak256(&address.to_tagged_bytes())
}

fn encode_motes(value: Motes) -> [u8; 32] {
    encode_u128(value.inner())
}

fn encode_u128(value: u128) -> [u8; 32] {
    let mut out = [0u8; 32];
    let be = value.to_be_bytes();
    for (dst, byte) in out.iter_mut().skip(16).zip(be) {
        *dst = byte;
    }
    out
}

fn encode_u64(value: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    let be = value.to_be_bytes();
    for (dst, byte) in out.iter_mut().skip(24).zip(be) {
        *dst = byte;
    }
    out
}

fn encode_string(value: &str) -> [u8; 32] {
    keccak256(value.as_bytes())
}

fn keccak256(input: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(input);
    hasher.finalize().into()
}

fn keccak256_concat(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use r402_protocol::UnixTimestamp;

    use super::*;
    use crate::chain::codec;
    use crate::exact::payload::ExactCasperAuthorization;

    fn spec_authorization() -> ExactCasperAuthorization {
        ExactCasperAuthorization {
            from: "0076d080b4e769f0b29c77fc6472d6e425710840c2f46a4506e5544d2ce34f43a3"
                .parse()
                .unwrap(),
            to: "007a9f9948cb7b258d18f3c5e85780372971b5b40096e724c9e596c284a01445fa"
                .parse()
                .unwrap(),
            value: Motes::new(7_500_000_000),
            valid_after: UnixTimestamp::from_secs(1_782_725_469),
            valid_before: UnixTimestamp::from_secs(1_782_729_069),
            nonce: "6505daf8ee30b4bf90db8e4ef3849ea869945ba0638853f6194704e8c9001115".to_owned(),
        }
    }

    fn spec_domain() -> Eip712Domain {
        domain_from_parts(
            "Casper X402 Token",
            "1",
            "casper:casper-test",
            "17be3c3dc67ddf193b8f64bfc2421826407470f88b3dab68184ebffebdd57f59"
                .parse()
                .unwrap(),
        )
    }

    #[test]
    fn domain_type_string_is_casper_specific() {
        assert!(CASPER_DOMAIN_TYPE.contains("chain_name"));
        assert!(CASPER_DOMAIN_TYPE.contains("contract_package_hash"));
        assert!(!CASPER_DOMAIN_TYPE.contains("chainId"));
    }

    #[test]
    fn digest_is_deterministic() {
        let a = transfer_with_authorization_digest(&spec_domain(), &spec_authorization());
        let b = transfer_with_authorization_digest(&spec_domain(), &spec_authorization());
        assert_eq!(a, b);
        assert_ne!(a, [0u8; 32]);
    }

    #[test]
    fn digest_changes_when_amount_changes() {
        let mut auth = spec_authorization();
        let baseline = transfer_with_authorization_digest(&spec_domain(), &auth);
        auth.value = Motes::new(1);
        let mutated = transfer_with_authorization_digest(&spec_domain(), &auth);
        assert_ne!(baseline, mutated);
    }

    #[test]
    fn casper_address_encoding_is_keccak_of_tagged_bytes() {
        let addr: Address = "0076d080b4e769f0b29c77fc6472d6e425710840c2f46a4506e5544d2ce34f43a3"
            .parse()
            .unwrap();
        assert_eq!(
            encode_casper_address(addr),
            keccak256(&addr.to_tagged_bytes())
        );
    }

    #[test]
    fn spec_example_digest_snapshot() {
        let digest = transfer_with_authorization_digest(&spec_domain(), &spec_authorization());
        assert_eq!(
            codec::encode(&digest),
            "bde9e6f18cef29a20bc794094e610cfe21de36c83a5e3ae156353a033fed8bd8"
        );
    }
}
