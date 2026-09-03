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
        write_builder_code(obj, &merged, advertised_a(payment_required));
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

fn advertised_info(payment_required: &PaymentRequired) -> Option<&Value> {
    let entry = payment_required.extensions.get(BUILDER_CODE)?;
    match entry {
        ExtensionEntry::Structured { info, .. } => Some(info),
        ExtensionEntry::Raw(value) => value.get("info"),
    }
}

fn advertised_s(payment_required: &PaymentRequired) -> Vec<CompactString> {
    advertised_info(payment_required).map_or_else(Vec::new, s_codes)
}

fn advertised_a(payment_required: &PaymentRequired) -> Option<&str> {
    advertised_info(payment_required)?
        .get("a")
        .and_then(Value::as_str)
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

fn write_builder_code(
    payload: &mut Map<String, Value>,
    merged: &[CompactString],
    advertised_a: Option<&str>,
) {
    let mut extensions = match payload.remove("extensions") {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    let mut entry = match extensions.remove(BUILDER_CODE) {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    if let Some(Value::Object(info)) = entry.get_mut("info") {
        insert_s_and_echo_a(info, merged, advertised_a);
    } else {
        let mut info = Map::new();
        insert_s_and_echo_a(&mut info, merged, advertised_a);
        let _ = entry.insert("info".to_owned(), Value::Object(info));
    }
    let _ = extensions.insert(BUILDER_CODE.to_owned(), Value::Object(entry));
    let _ = payload.insert("extensions".to_owned(), Value::Object(extensions));
}

fn insert_s_and_echo_a(
    info: &mut Map<String, Value>,
    merged: &[CompactString],
    advertised_a: Option<&str>,
) {
    let _ = info.insert("s".to_owned(), json!(merged));
    if !info.contains_key("a")
        && let Some(a) = advertised_a
    {
        let _ = info.insert("a".to_owned(), Value::String(a.to_owned()));
    }
}
