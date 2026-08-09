# r402-http

HTTP transport for the [x402 payment protocol][x402], part of the
[`r402`](../r402) workspace.

[x402]: https://www.x402.org

## Highlights

- **Server `Paygate` + `X402Middleware`** — Axum layer enforcing payments
  via `ResourceServer` (verify/settle hooks, cancel on handler failure),
  three settlement modes (sequential, concurrent, background), and
  HTTP-only `PaygateHooks` (API keys / KYT).
- **Client `X402Client`** — thin `reqwest_middleware` adapter over
  `r402_core::PaymentClient`: 402 → sign → retry, plus
  `on_payment_response` with one corrective recovery.
- **Remote `FacilitatorClient`** — `/verify`, `/settle`, `/supported`
  with 30s timeout, 429/5xx backoff, 10-minute `/supported` cache.
- **CORS** — exposes `Payment-Required` and `Payment-Response`.

## Cargo Features

| Feature       | Surface                                          |
| ------------- | ------------------------------------------------ |
| `client`      | `reqwest_middleware`-based buyer middleware.     |
| `server`      | axum paygate / facilitator HTTP servers.         |
| `facilitator` | `FacilitatorClient` for talking to a remote     |
|               | facilitator.                                     |
| `telemetry`   | OpenTelemetry tracing instrumentation.           |
| `full`        | All of the above.                                |

## Documentation

- Crate docs: <https://docs.rs/r402-http>
- Project README: [`../README.md`](../README.md).

## License

Dual-licensed under [MIT](../LICENSE-MIT) and
[Apache-2.0](../LICENSE-APACHE).
