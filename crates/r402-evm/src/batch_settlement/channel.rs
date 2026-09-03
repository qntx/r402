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

/// Computes the chain-bound channel id from config + chain id.
///
/// Matches TS `computeChannelId` / Go `ComputeChannelId`.
#[must_use]
pub fn compute_channel_id(config: &ChannelConfig, chain_id: u64) -> B256 {
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
        withdrawDelay: alloy_primitives::Uint::from(config.withdraw_delay),
        salt: config.salt,
    };
    typed.eip712_signing_hash(&domain)
}

/// Returns an error message when claimed id does not match config.
#[must_use]
pub fn channel_id_binding_error(
    config: &ChannelConfig,
    claimed: B256,
    chain_id: u64,
) -> Option<&'static str> {
    let expected = compute_channel_id(config, chain_id);
    if expected == claimed {
        None
    } else {
        Some("channel_id_mismatch")
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
        let a = compute_channel_id(&cfg, 8453);
        let b = compute_channel_id(&cfg, 8453);
        assert_eq!(a, b);
        let c = compute_channel_id(&cfg, 1);
        assert_ne!(a, c);
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
        let id = compute_channel_id(&cfg, 8453);
        assert!(channel_id_binding_error(&cfg, id, 8453).is_none());
        assert!(channel_id_binding_error(&cfg, B256::ZERO, 8453).is_some());
    }
}
