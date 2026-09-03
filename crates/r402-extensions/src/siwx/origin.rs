//! Configured public origin for SIWX challenges.
//!
//! Domain and URI are derived from this value. They are never taken from the
//! HTTP `Host` header.

use compact_str::CompactString;

/// Failed to parse a configured public origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SiwxOriginError {
    /// Origin string was empty.
    #[error("siwx origin is required")]
    Missing,
    /// Origin is not an absolute `http://` or `https://` URL.
    ///
    /// A bare host (the shape of an HTTP `Host` header) is rejected.
    #[error("siwx origin must be an absolute http(s) URL")]
    NotAbsolute,
    /// Authority is empty or contains userinfo.
    #[error("siwx origin authority is invalid")]
    InvalidAuthority,
}

/// Absolute public origin used as the SIWX trust anchor.
///
/// Constructed from server configuration. There is no constructor that
/// accepts an HTTP `Host` header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiwxOrigin {
    origin: CompactString,
    domain: CompactString,
}

impl SiwxOrigin {
    /// Parses an absolute `http://` or `https://` origin.
    ///
    /// Path, query, and fragment are discarded. Userinfo is rejected.
    ///
    /// # Errors
    ///
    /// [`SiwxOriginError`] when the value is missing, not absolute http(s),
    /// or has an empty/userinfo authority.
    pub fn parse(raw: &str) -> Result<Self, SiwxOriginError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(SiwxOriginError::Missing);
        }
        let rest = trimmed
            .strip_prefix("https://")
            .or_else(|| trimmed.strip_prefix("http://"))
            .ok_or(SiwxOriginError::NotAbsolute)?;
        let authority = rest.split_once(['/', '?', '#']).map_or(rest, |(h, _)| h);
        if authority.is_empty() || authority.contains('@') {
            return Err(SiwxOriginError::InvalidAuthority);
        }
        let scheme_len = trimmed.len() - rest.len();
        let origin = trimmed
            .get(..scheme_len + authority.len())
            .map(CompactString::from)
            .ok_or(SiwxOriginError::InvalidAuthority)?;
        Ok(Self {
            origin,
            domain: CompactString::from(authority),
        })
    }

    /// Scheme + host + optional port, with no path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.origin
    }

    /// CAIP-122 `domain` (host and non-default port). Never from `Host`.
    #[must_use]
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// Resource URI: configured origin joined with `path`.
    #[must_use]
    pub fn uri(&self, path: &str) -> CompactString {
        join_origin_path(&self.origin, path)
    }

    /// Paid-address store key: configured origin + request path.
    #[must_use]
    pub fn store_key(&self, path: &str) -> CompactString {
        join_origin_path(&self.origin, path)
    }
}

fn join_origin_path(origin: &str, path: &str) -> CompactString {
    let path = if path.is_empty() { "/" } else { path };
    let mut out = CompactString::from(origin);
    if !path.starts_with('/') {
        out.push('/');
    }
    out.push_str(path);
    out
}
