//! Configuration for the Casper exact scheme facilitator client.
//!
//! Casper settlement is performed by a facilitator that holds a funded
//! Casper account and a node connection. r402 talks to such a facilitator
//! over the standard x402 `/verify`, `/settle`, and `/supported` endpoints,
//! so this crate ships a configuration type rather than a signing stack —
//! keeping the dependency footprint in line with the other chain crates.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use url::Url;

/// Hosted Casper x402 facilitator operated by the Casper Association.
///
/// Endpoint documentation: <https://docs.cspr.cloud>
pub const DEFAULT_FACILITATOR_URL: &str = "https://x402-facilitator.cspr.cloud";

/// Environment variable consulted by [`CasperFacilitatorConfig::from_env`]
/// to override [`DEFAULT_FACILITATOR_URL`].
pub const FACILITATOR_URL_ENV: &str = "R402_CASPER_FACILITATOR_URL";

/// Errors produced while building a facilitator configuration.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CasperFacilitatorConfigError {
    /// The configured base URL could not be parsed.
    #[error("invalid facilitator url {url:?}: {source}")]
    InvalidUrl {
        /// The offending value.
        url: String,
        /// The underlying parse error.
        #[source]
        source: url::ParseError,
    },
}

/// Connection settings for a remote Casper x402 facilitator.
///
/// # Examples
///
/// ```
/// use r402_casper::exact::facilitator::CasperFacilitatorConfig;
///
/// let config = CasperFacilitatorConfig::default();
/// assert_eq!(
///     config.verify_url().as_str(),
///     "https://x402-facilitator.cspr.cloud/verify"
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CasperFacilitatorConfig {
    /// Base URL of the facilitator.
    #[serde(default = "default_base_url")]
    pub base_url: Url,
    /// Per-request timeout.
    #[serde(default = "default_timeout", with = "duration_secs")]
    pub timeout: Duration,
    /// Whether to run local payload validation before dispatching a request.
    ///
    /// Enabled by default: rejecting a malformed payment locally saves a
    /// network round trip and produces a precise, typed error instead of an
    /// opaque remote failure.
    #[serde(default = "default_true")]
    pub validate_locally: bool,
}

fn default_base_url() -> Url {
    #[allow(
        clippy::expect_used,
        reason = "the compiled-in default URL is validated by a unit test"
    )]
    Url::parse(DEFAULT_FACILITATOR_URL).expect("default facilitator URL is valid")
}

const fn default_timeout() -> Duration {
    Duration::from_secs(30)
}

const fn default_true() -> bool {
    true
}

impl Default for CasperFacilitatorConfig {
    fn default() -> Self {
        Self {
            base_url: default_base_url(),
            timeout: default_timeout(),
            validate_locally: true,
        }
    }
}

impl CasperFacilitatorConfig {
    /// Builds a configuration pointing at an explicit base URL.
    ///
    /// # Errors
    ///
    /// Returns [`CasperFacilitatorConfigError::InvalidUrl`] when `base_url`
    /// is not a valid absolute URL.
    pub fn new(base_url: &str) -> Result<Self, CasperFacilitatorConfigError> {
        let base_url =
            Url::parse(base_url).map_err(|source| CasperFacilitatorConfigError::InvalidUrl {
                url: base_url.to_owned(),
                source,
            })?;
        Ok(Self {
            base_url,
            ..Self::default()
        })
    }

    /// Builds a configuration from the environment, falling back to the
    /// hosted facilitator when [`FACILITATOR_URL_ENV`] is unset or empty.
    ///
    /// # Errors
    ///
    /// Returns [`CasperFacilitatorConfigError::InvalidUrl`] when the
    /// environment variable is set to something that is not a valid URL —
    /// a silent fallback would send payments somewhere the operator did not
    /// intend.
    pub fn from_env() -> Result<Self, CasperFacilitatorConfigError> {
        match std::env::var(FACILITATOR_URL_ENV) {
            Ok(value) if !value.trim().is_empty() => Self::new(value.trim()),
            _ => Ok(Self::default()),
        }
    }

    /// Builder: overrides the per-request timeout.
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Builder: enables or disables local pre-flight validation.
    #[must_use]
    pub const fn with_local_validation(mut self, validate_locally: bool) -> Self {
        self.validate_locally = validate_locally;
        self
    }

    /// Returns the `POST /verify` endpoint.
    #[must_use]
    pub fn verify_url(&self) -> Url {
        self.endpoint("verify")
    }

    /// Returns the `POST /settle` endpoint.
    #[must_use]
    pub fn settle_url(&self) -> Url {
        self.endpoint("settle")
    }

    /// Returns the `GET /supported` endpoint.
    #[must_use]
    pub fn supported_url(&self) -> Url {
        self.endpoint("supported")
    }

    /// Joins `path` onto the base URL, preserving any base path prefix.
    fn endpoint(&self, path: &str) -> Url {
        let mut url = self.base_url.clone();
        if let Ok(mut segments) = url.path_segments_mut() {
            // A base URL of `https://host/x402/` must yield
            // `https://host/x402/verify`, not `https://host/verify`.
            segments.pop_if_empty().push(path);
        }
        url
    }
}

/// Serialises [`Duration`] as whole seconds for JSON configuration files.
mod duration_secs {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(
        value: &Duration,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(value.as_secs())
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Duration, D::Error> {
        u64::deserialize(deserializer).map(Duration::from_secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_targets_the_hosted_casper_facilitator() {
        let config = CasperFacilitatorConfig::default();
        assert_eq!(config.base_url.as_str(), "https://x402-facilitator.cspr.cloud/");
        assert_eq!(
            config.verify_url().as_str(),
            "https://x402-facilitator.cspr.cloud/verify"
        );
        assert_eq!(
            config.settle_url().as_str(),
            "https://x402-facilitator.cspr.cloud/settle"
        );
        assert_eq!(
            config.supported_url().as_str(),
            "https://x402-facilitator.cspr.cloud/supported"
        );
    }

    #[test]
    fn base_path_prefix_is_preserved() {
        let config = CasperFacilitatorConfig::new("https://example.test/x402/").unwrap();
        assert_eq!(
            config.verify_url().as_str(),
            "https://example.test/x402/verify"
        );
    }

    #[test]
    fn base_url_without_trailing_slash_still_appends() {
        let config = CasperFacilitatorConfig::new("https://example.test/x402").unwrap();
        assert_eq!(
            config.settle_url().as_str(),
            "https://example.test/x402/settle"
        );
    }

    #[test]
    fn invalid_url_is_rejected() {
        let err = CasperFacilitatorConfig::new("not a url").unwrap_err();
        assert!(matches!(
            err,
            CasperFacilitatorConfigError::InvalidUrl { .. }
        ));
    }

    #[test]
    fn builders_override_defaults() {
        let config = CasperFacilitatorConfig::default()
            .with_timeout(Duration::from_secs(5))
            .with_local_validation(false);
        assert_eq!(config.timeout, Duration::from_secs(5));
        assert!(!config.validate_locally);
    }

    #[test]
    fn config_round_trips_through_json() {
        let json = serde_json::json!({
            "baseUrl": "https://facilitator.example/",
            "timeout": 12,
            "validateLocally": false
        });
        let config: CasperFacilitatorConfig = serde_json::from_value(json).unwrap();
        assert_eq!(config.base_url.as_str(), "https://facilitator.example/");
        assert_eq!(config.timeout, Duration::from_secs(12));
        assert!(!config.validate_locally);

        let encoded = serde_json::to_value(&config).unwrap();
        assert_eq!(encoded["timeout"], 12);
    }

    #[test]
    fn json_config_falls_back_to_defaults() {
        let config: CasperFacilitatorConfig = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(config, CasperFacilitatorConfig::default());
    }
}
