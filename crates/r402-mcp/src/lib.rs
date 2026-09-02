//! MCP transport for the x402 payment protocol (official `rmcp` SDK).

#[cfg(test)]
mod tests {
    #[test]
    fn crate_loads() {
        assert_eq!(
            env!("CARGO_PKG_NAME"),
            "r402-mcp",
            "package name must match the crate directory"
        );
    }
}
