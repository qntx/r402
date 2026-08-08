# r402-http

HTTP transport for the [x402 payment protocol][x402], part of the
[`r402`](../r402) workspace.

[x402]: https://www.x402.org

## Highlights

- **Server `Paygate` middleware** — axum-style layer enforcing payments
  with three settlement modes (sequential, concurrent, background) and
  pluggable hooks for KYT / API keys / observability.
- **Client `X402Middleware`** — `reqwest_middleware` adapter that handles
  402 challenges, signs payments, and retries the protected request.
- **Remote `FacilitatorClient`** — talks to a remote facilitator's
  `/verify`, `/settle`, and `/supported` endpoints with a 30-second
  default timeout, exponential-backoff retry on `429`/5xx, and a
  10-minute cache for `/supported` shared across clones.
- **CORS-aware** — automatically appends `Access-Control-Expose-Headers`
  for the `Payment-Required` and `Payment-Response` headers.
- **Structured 4xx parsing** — facilitator clients deserialise structured
  `VerifyResponse::Invalid` / `SettleResponse::Failure` bodies regardless
  of HTTP status code, preserving spec §9 error reasons end-to-end.

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
