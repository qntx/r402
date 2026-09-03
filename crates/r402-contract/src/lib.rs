//! JSON contract assertions for the x402 payment protocol.

use std::path::Path;

/// Load workspace `tests/fixtures/contract/<name>` as JSON.
///
/// Resolved from `CARGO_MANIFEST_DIR` so `cargo test -p r402-contract` finds
/// fixtures at the workspace root rather than the crate directory.
///
/// # Errors
///
/// Returns an error if the file cannot be read or parsed as JSON.
pub fn contract_json(name: &str) -> std::io::Result<serde_json::Value> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/contract")
        .join(name);
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))
}

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        assert_eq!(
            env!("CARGO_PKG_NAME"),
            "r402-contract",
            "package name must match the crate directory"
        );
    }
}
