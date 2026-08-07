# r402-evm

EIP-155 (EVM) chain support for the [x402 payment protocol][x402], part
of the [`r402`](../r402) workspace.

[x402]: https://www.x402.org

## Highlights

- **Exact scheme** — ERC-3009 `transferWithAuthorization` and
  `x402ExactPermit2Proxy` with EIP-1271 / EIP-6492 smart-wallet support.
- **Upto scheme** — `x402UptoPermit2Proxy.settle` with the
  `eip2612GasSponsoring` extension for buyers without a Permit2
  approval.
- **42 supported EVM chains** with native USDC deployments (see
  `r402_evm::networks`).
- **Multicall3 verification** — `eth_call` allowance + balance + nonce
  checks bundled into a single round trip.

## Cargo Features

| Feature        | Surface                                           |
| -------------- | ------------------------------------------------- |
| `client`       | Client-side EIP-712 signing and Permit2 approval. |
| `server`       | Server-side `PriceTag` generation.                |
| `facilitator`  | Facilitator-side verify and settle.               |
| `telemetry`    | OpenTelemetry tracing instrumentation.            |
| `full`         | All of the above.                                 |

## Documentation

- Crate docs: <https://docs.rs/r402-evm>
- Audit reports: [`../audit/`](../audit) (architecture, security, and
  cross-SDK interop).
- Project README: [`../README.md`](../README.md).

## License

Dual-licensed under [MIT](../LICENSE-MIT) and
[Apache-2.0](../LICENSE-APACHE).
