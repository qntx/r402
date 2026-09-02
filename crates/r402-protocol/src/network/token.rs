//! Token amount paired with deployment metadata.

/// Amount in the token's smallest unit plus the deployment it refers to.
#[derive(Debug, Clone)]
pub struct DeployedTokenAmount<TAmount, TToken> {
    /// Amount in the token's smallest unit (wei, lamports, …).
    pub amount: TAmount,
    /// Token deployment (chain, address, decimals).
    pub token: TToken,
}
