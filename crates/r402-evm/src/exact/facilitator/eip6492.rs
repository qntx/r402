//! EIP-6492 factory allowlist for exact-scheme verify and settle.

use alloy_primitives::Address;

/// Official exact-scheme deny reason when the 6492 factory is not allowlisted.
pub(crate) const FACTORY_NOT_ALLOWED: &str = "eip6492_factory_not_allowed";

/// Official exact-scheme deny reason when `getCode(asset)` is empty.
pub(crate) const ASSET_NOT_DEPLOYED: &str = "asset_not_deployed_contract";

/// Official exact-scheme deny reason when a success receipt has no matching Transfer.
pub(crate) const TRANSFER_EVENT_MISMATCH: &str = "invalid_exact_evm_transfer_event_mismatch";

/// Whether `factory` is in `allowlist`. Empty allowlist admits none.
#[must_use]
pub(crate) fn factory_allowed(factory: Address, allowlist: &[Address]) -> bool {
    allowlist.contains(&factory)
}

/// Counterfactual 6492 path: no code at payer and factory not allowlisted.
#[must_use]
pub(crate) fn deny_undeployed_factory(
    factory: Address,
    payer_code: &[u8],
    allowlist: &[Address],
) -> bool {
    payer_code.is_empty() && !factory_allowed(factory, allowlist)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_allowlist_admits_none() {
        let factory = Address::repeat_byte(0xF1);
        assert!(!factory_allowed(factory, &[]));
        assert!(deny_undeployed_factory(factory, &[], &[]));
    }

    #[test]
    fn allowlisted_factory_is_admitted() {
        let factory = Address::repeat_byte(0xF1);
        let allowlist = [factory];
        assert!(factory_allowed(factory, &allowlist));
        assert!(!deny_undeployed_factory(factory, &[], &allowlist));
    }

    #[test]
    fn deployed_payer_skips_factory_gate() {
        let factory = Address::repeat_byte(0xF1);
        assert!(!deny_undeployed_factory(factory, &[0xef, 0x01], &[]));
    }

    #[test]
    fn other_factory_is_denied() {
        let factory = Address::repeat_byte(0xF1);
        let allowlist = [Address::repeat_byte(0xF2)];
        assert!(!factory_allowed(factory, &allowlist));
        assert!(deny_undeployed_factory(factory, &[], &allowlist));
    }

    #[test]
    fn official_reason_strings() {
        assert_eq!(FACTORY_NOT_ALLOWED, "eip6492_factory_not_allowed");
        assert_eq!(ASSET_NOT_DEPLOYED, "asset_not_deployed_contract");
        assert_eq!(
            TRANSFER_EVENT_MISMATCH,
            "invalid_exact_evm_transfer_event_mismatch"
        );
    }
}
