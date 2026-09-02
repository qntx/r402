//! Header-name and status-code contract.

#[cfg(test)]
mod tests {
    use r402_contract::contract_json;
    use serde_json::Value;

    fn require<'a>(value: &'a Value, keys: &[&str]) -> &'a Value {
        let mut current = value;
        for key in keys {
            current = current.get(*key).expect("missing key in headers.json");
        }
        current
    }

    #[test]
    fn headers_match_contract() {
        let value = contract_json("headers.json").expect("headers.json must load");
        assert_eq!(
            value.as_object().expect("headers.json object").len(),
            8,
            "headers.json top-level key count"
        );
        assert_eq!(
            require(&value, &["payment_signature"]),
            "Payment-Signature",
            "payment_signature"
        );
        assert_eq!(
            require(&value, &["payment_required"]),
            "Payment-Required",
            "payment_required"
        );
        assert_eq!(
            require(&value, &["payment_response"]),
            "Payment-Response",
            "payment_response"
        );
        assert_eq!(
            require(&value, &["sign_in_with_x"]),
            "SIGN-IN-WITH-X",
            "sign_in_with_x"
        );
        assert_eq!(
            require(&value, &["extension_responses"]),
            "EXTENSION-RESPONSES",
            "extension_responses"
        );
        assert_eq!(
            require(&value, &["expose_headers"]),
            "Payment-Required, Payment-Response",
            "expose_headers"
        );

        assert_eq!(
            require(&value, &["status"])
                .as_object()
                .expect("status object")
                .len(),
            7,
            "status key count"
        );
        assert_eq!(require(&value, &["status", "unpaid"]), 402, "status.unpaid");
        assert_eq!(
            require(&value, &["status", "malformed_payment"]),
            402,
            "status.malformed_payment"
        );
        assert_eq!(
            require(&value, &["status", "permit2_allowance"]),
            412,
            "status.permit2_allowance"
        );
        assert_eq!(
            require(&value, &["status", "facilitator_transport"]),
            502,
            "status.facilitator_transport"
        );
        assert_eq!(
            require(&value, &["status", "missing_scheme"]),
            500,
            "status.missing_scheme"
        );
        assert_eq!(
            require(&value, &["status", "incompatible_mode"]),
            500,
            "status.incompatible_mode"
        );
        assert_eq!(require(&value, &["status", "ok"]), 200, "status.ok");

        assert_eq!(
            require(&value, &["cache_control"])
                .as_object()
                .expect("cache_control object")
                .len(),
            3,
            "cache_control key count"
        );
        assert_eq!(
            require(&value, &["cache_control", "payment_required"]),
            "no-store",
            "cache_control.payment_required"
        );
        assert_eq!(
            require(&value, &["cache_control", "settle_failure"]),
            "no-store",
            "cache_control.settle_failure"
        );
        assert_eq!(
            require(&value, &["cache_control", "ok_with_receipt"]),
            "private",
            "cache_control.ok_with_receipt"
        );
    }
}
