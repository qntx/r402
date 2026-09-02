//! x402 Payment Protocol SDK for Rust — umbrella crate.

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        assert_eq!(
            env!("CARGO_PKG_NAME"),
            "r402",
            "package name must match the crate directory"
        );
    }
}
