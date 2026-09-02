//! Aptos chain support for the x402 payment protocol.

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        assert_eq!(
            env!("CARGO_PKG_NAME"),
            "r402-aptos",
            "package name must match the crate directory"
        );
    }
}
