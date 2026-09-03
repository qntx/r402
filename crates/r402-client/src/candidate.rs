//! Scheme client, default-asset lookup, and signed payment candidates.

use std::fmt::{self, Debug, Formatter};
use std::future::Future;
use std::pin::Pin;

use compact_str::CompactString;
use r402_protocol::{ChainId, ClientError, PaymentRequired, PaymentRequirements, SchemeId};

/// USD-pegged default asset used for money strings and client spend caps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultAssetInfo {
    /// Asset id as advertised in payment requirements.
    pub asset: CompactString,
    /// Token decimal places.
    pub decimals: u32,
    /// Ticker (`"USDC"`, `"USDT0"`, `"RLUSD"`, …).
    pub symbol: CompactString,
    /// Transfer-method override when the default is not the scheme ATM default.
    pub asset_transfer_method: Option<CompactString>,
}

impl DefaultAssetInfo {
    /// Constructs a default-asset row without a transfer-method override.
    #[must_use]
    pub fn new(
        asset: impl Into<CompactString>,
        decimals: u32,
        symbol: impl Into<CompactString>,
    ) -> Self {
        Self {
            asset: asset.into(),
            decimals,
            symbol: symbol.into(),
            asset_transfer_method: None,
        }
    }

    /// Sets `assetTransferMethod`.
    #[must_use]
    pub fn with_asset_transfer_method(mut self, method: impl Into<CompactString>) -> Self {
        self.asset_transfer_method = Some(method.into());
        self
    }
}

/// Buyer-side scheme: emit candidates from a 402 challenge.
pub trait SchemeClient: SchemeId + Send + Sync {
    /// Payment options this client can fulfil for `payment_required`.
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate>;

    /// Reverse lookup of a USD-pegged default asset for spend-cap conversion.
    fn find_default_asset(&self, asset: &str, network: &ChainId) -> Option<DefaultAssetInfo>;
}

/// One payment option produced by [`SchemeClient::accept`].
pub struct PaymentCandidate {
    /// CAIP-2 chain id of the target chain.
    pub chain_id: ChainId,
    /// Token asset address / mint.
    pub asset: CompactString,
    /// Amount in the token's smallest unit, stringified.
    pub amount: CompactString,
    /// Scheme identifier (`"exact"`, `"upto"`, ...).
    pub scheme: CompactString,
    /// Recipient address.
    pub pay_to: CompactString,
    /// Wire requirements this candidate was built from.
    pub requirements: PaymentRequirements,
    /// Signer that can produce the authorization.
    pub signer: Box<dyn PaymentCandidateSigner>,
}

impl Debug for PaymentCandidate {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("PaymentCandidate")
            .field("chain_id", &self.chain_id)
            .field("asset", &self.asset)
            .field("amount", &self.amount)
            .field("scheme", &self.scheme)
            .field("pay_to", &self.pay_to)
            .finish_non_exhaustive()
    }
}

impl PaymentCandidate {
    /// Signs the candidate, returning the base64-encoded payload.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] when signing fails.
    pub async fn sign(&self) -> Result<String, ClientError> {
        self.signer.sign_payment().await
    }
}

/// Object-safe signer carried by each [`PaymentCandidate`].
pub trait PaymentCandidateSigner: Send + Sync {
    /// Produces the signed payment payload (base64-encoded).
    fn sign_payment<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<String, ClientError>> + Send + 'a>>;
}
