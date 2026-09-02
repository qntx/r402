//! Resource server, `paymentFlow`, and `SettlementMode` scheduler for the x402 payment protocol.

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        assert_eq!(
            env!("CARGO_PKG_NAME"),
            "r402-server",
            "package name must match the crate directory"
        );
    }
}
