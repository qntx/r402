//! Settlement-mode contract.

#[cfg(test)]
mod tests {
    use r402_contract::contract_json;
    use serde_json::{Value, json};

    fn require<'a>(value: &'a Value, keys: &[&str]) -> &'a Value {
        let mut current = value;
        for key in keys {
            current = current
                .get(*key)
                .expect("missing key in settlement_modes.json");
        }
        current
    }

    fn assert_mode(
        value: &Value,
        name: &str,
        order: &Value,
        payment_response: bool,
        upto_actual: &str,
        handler_4xx: &str,
    ) {
        assert_eq!(
            require(value, &[name])
                .as_object()
                .expect("mode object")
                .len(),
            4,
            "{name} key count"
        );
        assert_eq!(require(value, &[name, "order"]), order, "{name}.order");
        assert_eq!(
            require(value, &[name, "payment_response"]),
            payment_response,
            "{name}.payment_response"
        );
        assert_eq!(
            require(value, &[name, "upto_actual"]),
            upto_actual,
            "{name}.upto_actual"
        );
        assert_eq!(
            require(value, &[name, "handler_4xx"]),
            handler_4xx,
            "{name}.handler_4xx"
        );
    }

    #[test]
    fn settlement_modes_match_contract() {
        let value =
            contract_json("settlement_modes.json").expect("settlement_modes.json must load");
        assert_eq!(
            value
                .as_object()
                .expect("settlement_modes.json object")
                .len(),
            3,
            "settlement_modes.json top-level key count"
        );

        assert_mode(
            &value,
            "sequential",
            &json!(["verify", "handler", "settle"]),
            true,
            "apply",
            "cancel_settle",
        );
        assert_mode(
            &value,
            "concurrent",
            &json!(["verify", "settle_parallel_handler", "join_settle"]),
            true,
            "reject_402_settlement_aborted",
            "detach_settle",
        );
        assert_mode(
            &value,
            "background",
            &json!(["verify", "spawn_settle", "handler"]),
            false,
            "strip_do_not_fail",
            "detach_settle",
        );
    }
}
