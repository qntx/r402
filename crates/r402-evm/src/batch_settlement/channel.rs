//! Channel id derivation (EIP-712 `ChannelConfig` hash).

use alloy_primitives::{Address, B256};
use alloy_sol_types::{SolStruct, eip712_domain, sol};

use super::payload::{
    BATCH_SETTLEMENT_ADDRESS, BATCH_SETTLEMENT_DOMAIN_NAME, BATCH_SETTLEMENT_DOMAIN_VERSION,
    ChannelConfig,
};

sol! {
    /// EIP-712 primary type for channel identity.
    struct ChannelConfigSol {
        address payer;
        address payerAuthorizer;
        address receiver;
        address receiverAuthorizer;
        address token;
        uint40 withdrawDelay;
        bytes32 salt;
    }
}

/// `withdrawDelay` does not fit `uint40` (ruint `Uint::from` would panic).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("withdrawDelay exceeds uint40")]
pub struct ChannelIdError;

/// Computes the chain-bound channel id from config + chain id.
///
/// Matches TS `computeChannelId` / Go `ComputeChannelId`.
///
/// # Errors
///
/// [`ChannelIdError`] when `withdrawDelay >= 2^40`.
pub fn compute_channel_id(config: &ChannelConfig, chain_id: u64) -> Result<B256, ChannelIdError> {
    let withdraw_delay = alloy_primitives::Uint::<40, 1>::try_from(config.withdraw_delay)
        .map_err(|_| ChannelIdError)?;
    let domain = eip712_domain! {
        name: BATCH_SETTLEMENT_DOMAIN_NAME,
        version: BATCH_SETTLEMENT_DOMAIN_VERSION,
        chain_id: chain_id,
        verifying_contract: BATCH_SETTLEMENT_ADDRESS,
    };
    let typed = ChannelConfigSol {
        payer: config.payer,
        payerAuthorizer: config.payer_authorizer,
        receiver: config.receiver,
        receiverAuthorizer: config.receiver_authorizer,
        token: config.token,
        withdrawDelay: withdraw_delay,
        salt: config.salt,
    };
    Ok(typed.eip712_signing_hash(&domain))
}

/// Returns an error message when claimed id does not match config, or when
/// `withdrawDelay` overflows `uint40`.
#[must_use]
pub fn channel_id_binding_error(
    config: &ChannelConfig,
    claimed: B256,
    chain_id: u64,
) -> Option<&'static str> {
    match compute_channel_id(config, chain_id) {
        Err(_) => Some("withdrawDelay exceeds uint40"),
        Ok(expected) if expected == claimed => None,
        Ok(_) => Some("channel_id_mismatch"),
    }
}

/// Validates withdraw delay bounds.
#[must_use]
pub fn withdraw_delay_valid(delay: u64) -> bool {
    (super::payload::MIN_WITHDRAW_DELAY..=super::payload::MAX_WITHDRAW_DELAY).contains(&delay)
}

/// Builds a channel config from server extra + client payer fields.
#[must_use]
pub const fn build_channel_config(
    payer: Address,
    payer_authorizer: Address,
    receiver: Address,
    receiver_authorizer: Address,
    token: Address,
    withdraw_delay: u64,
    salt: B256,
) -> ChannelConfig {
    ChannelConfig {
        payer,
        payer_authorizer,
        receiver,
        receiver_authorizer,
        token,
        withdraw_delay,
        salt,
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;

    use super::*;

    #[test]
    fn channel_id_deterministic() {
        let cfg = ChannelConfig {
            payer: Address::repeat_byte(0x11),
            payer_authorizer: Address::repeat_byte(0x11),
            receiver: Address::repeat_byte(0x22),
            receiver_authorizer: Address::repeat_byte(0x33),
            token: Address::repeat_byte(0x44),
            withdraw_delay: 900,
            salt: B256::ZERO,
        };
        let a = compute_channel_id(&cfg, 8453).expect("id");
        let b = compute_channel_id(&cfg, 8453).expect("id");
        assert_eq!(a, b);
        let c = compute_channel_id(&cfg, 1).expect("id");
        assert_ne!(a, c);
    }

    #[test]
    fn channel_id_rejects_uint40_overflow() {
        let cfg = ChannelConfig {
            payer: Address::repeat_byte(0x11),
            payer_authorizer: Address::repeat_byte(0x11),
            receiver: Address::repeat_byte(0x22),
            receiver_authorizer: Address::repeat_byte(0x33),
            token: Address::repeat_byte(0x44),
            withdraw_delay: 1 << 40,
            salt: B256::ZERO,
        };
        assert!(compute_channel_id(&cfg, 8453).is_err());
        assert_eq!(
            channel_id_binding_error(&cfg, B256::ZERO, 8453),
            Some("withdrawDelay exceeds uint40")
        );
    }

    #[test]
    fn binding_detects_mismatch() {
        let cfg = ChannelConfig {
            payer: Address::repeat_byte(0x11),
            payer_authorizer: Address::ZERO,
            receiver: Address::repeat_byte(0x22),
            receiver_authorizer: Address::repeat_byte(0x33),
            token: Address::repeat_byte(0x44),
            withdraw_delay: 900,
            salt: B256::ZERO,
        };
        let id = compute_channel_id(&cfg, 8453).expect("id");
        assert!(channel_id_binding_error(&cfg, id, 8453).is_none());
        assert!(channel_id_binding_error(&cfg, B256::ZERO, 8453).is_some());
    }
}
