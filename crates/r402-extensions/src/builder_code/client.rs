//! Client extension that attaches `s[]` to a signed payment payload.

use compact_str::CompactString;
use r402_client::ClientExtension;
use r402_protocol::payment::{Base64Bytes, ExtensionEntry, Extensions};
use r402_protocol::{ClientError, PaymentRequired};
use serde_json::{Map, Value, json};

use super::{
    BUILDER_CODE, BuilderCodeError, MAX_CLIENT_SERVICE_CODES, normalize_service_codes,
    validate_codes,
};

/// Client-side builder-code codes (`s`).
///
/// Register with [`r402_client::PaymentClient::with_extension`].
#[derive(Debug, Clone)]
pub struct BuilderCodeClient {
    service_codes: Vec<CompactString>,
}

impl BuilderCodeClient {
    /// One or more client service codes (max [`MAX_CLIENT_SERVICE_CODES`]).
    ///
    /// # Errors
    ///
    /// Invalid code or too many codes.
    pub fn try_new<'a, I>(codes: I) -> Result<Self, BuilderCodeError>
    where
        I: IntoIterator<Item = &'a str>,
    {
        Ok(Self {
            service_codes: validate_codes(codes, MAX_CLIENT_SERVICE_CODES)?,
        })
    }

    /// Payload extension `{ info: { s: [...] } }` (official client shape).
    #[must_use]
    pub fn payload_entry(&self) -> ExtensionEntry {
        ExtensionEntry::info(json!({ "s": self.service_codes }))
    }

    /// Inserts this client's `s[]` under `builder-code`.
    pub fn enrich_extensions(&self, extensions: &mut Extensions) {
        extensions.insert(BUILDER_CODE, self.payload_entry());
    }

    /// Configured service codes.
    #[must_use]
    pub fn service_codes(&self) -> &[CompactString] {
        &self.service_codes
    }

    fn enrich_payload(
        &self,
        payload_b64: &str,
        payment_required: &PaymentRequired,
    ) -> Result<String, ClientError> {
        let raw = Base64Bytes::from(payload_b64.as_bytes())
            .decode()
            .map_err(|err| ClientError::Parse(err.to_string()))?;
        let mut value: Value = serde_json::from_slice(&raw)?;
        let Value::Object(obj) = &mut value else {
            return Err(ClientError::Parse(
                "payment payload is not a JSON object".into(),
            ));
        };
        let merged = merge_service_codes(
            &self.service_codes,
            advertised_s(payment_required),
            payload_s(obj),
        );
        write_builder_code_s(obj, &merged);
        Ok(Base64Bytes::encode(serde_json::to_vec(&value)?).to_string())
    }
}

impl ClientExtension for BuilderCodeClient {
    fn key(&self) -> &'static str {
        BUILDER_CODE
    }

    fn enrich_payment_payload<'a>(
        &'a self,
        payload_b64: &'a str,
        payment_required: &'a PaymentRequired,
    ) -> impl Future<Output = Result<String, ClientError>> + Send + 'a {
        std::future::ready(self.enrich_payload(payload_b64, payment_required))
    }
}

fn advertised_s(payment_required: &PaymentRequired) -> Vec<CompactString> {
    let Some(entry) = payment_required.extensions.get(BUILDER_CODE) else {
        return Vec::new();
    };
    let info = match entry {
        ExtensionEntry::Structured { info, .. } => info,
        ExtensionEntry::Raw(value) => match value.get("info") {
            Some(info) => info,
            None => return Vec::new(),
        },
    };
    s_codes(info)
}

fn payload_s(payload: &Map<String, Value>) -> Vec<CompactString> {
    payload
        .get("extensions")
        .and_then(Value::as_object)
        .and_then(|extensions| extensions.get(BUILDER_CODE))
        .and_then(Value::as_object)
        .and_then(|entry| entry.get("info"))
        .map_or_else(Vec::new, s_codes)
}

fn s_codes(info: &Value) -> Vec<CompactString> {
    if !info.is_object() {
        return Vec::new();
    }
    info.get("s")
        .map(normalize_service_codes)
        .unwrap_or_default()
}

fn merge_service_codes(
    client: &[CompactString],
    advertised: Vec<CompactString>,
    payload: Vec<CompactString>,
) -> Vec<CompactString> {
    let mut out = client.to_vec();
    for code in advertised.into_iter().chain(payload) {
        if !out.contains(&code) {
            out.push(code);
        }
    }
    out
}

fn write_builder_code_s(payload: &mut Map<String, Value>, merged: &[CompactString]) {
    let mut extensions = match payload.remove("extensions") {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    let mut entry = match extensions.remove(BUILDER_CODE) {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    match entry.get_mut("info") {
        Some(Value::Object(info)) => {
            let _ = info.insert("s".to_owned(), json!(merged));
        }
        _ => {
            let _ = entry.insert("info".to_owned(), json!({ "s": merged }));
        }
    }
    let _ = extensions.insert(BUILDER_CODE.to_owned(), Value::Object(entry));
    let _ = payload.insert("extensions".to_owned(), Value::Object(extensions));
}
