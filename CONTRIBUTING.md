# Contributing to r402

Thanks for your interest in helping ship a production-grade Rust SDK for
the [x402 payment protocol][x402]. This document covers everything you
need to know before opening a pull request.

[x402]: https://www.x402.org

## Code of Conduct

Be kind, technical, and concise. No harassment, no surprises, no
backchannel exploits. Vulnerabilities go to [`SECURITY.md`](SECURITY.md);
features and bugs go through GitHub issues and pull requests.

## Development Setup

```bash
git clone --recurse-submodules https://github.com/qntx/r402
cd r402

# Build the entire workspace with all features.
cargo build --workspace --all-features

# Run unit + integration + doc tests across the workspace.
cargo test --workspace --all-features
```

The submodules under `3rdparty/` (notably `x402` and `siwx`) provide the
spec, fixtures, and reference SDKs we cross-check against. Re-sync with
`git submodule update --remote --recursive`.

### Toolchain

The MSRV is pinned to **Rust 1.91** in `rust-toolchain.toml`. CI builds
on stable; you should match that locally with `rustup`.

### Quality Gates (run before pushing)

The same checks run in CI; running them locally is the fastest way to
keep iteration cycles short.

```bash
cargo fmt --all --check
cargo clippy --workspace --all-features --tests -- -D warnings
cargo test  --workspace --all-features
cargo doc   --workspace --all-features --no-deps
```

For dependency hygiene:

```bash
cargo install cargo-audit cargo-deny
cargo audit
cargo deny check all
```

## Pull Request Workflow

1. **Open an issue** if your change is non-trivial. Architecture-level
   changes benefit from a quick design discussion before coding.
2. **Branch from `main`** — keep PRs focused on one logical concern.
3. **Add tests** alongside any behaviour change. Where you touch a wire
   type or scheme, prefer round-trip tests against the fixtures in
   `3rdparty/x402/specs/` so cross-SDK compatibility stays honest.
4. **Update the changelog** (`CHANGELOG.md`, Keep-a-Changelog format) and
   `MIGRATION.md` if you make breaking API changes.
5. **Run the quality gates** above. CI will reject `clippy` or `fmt`
   violations.
6. **Open the PR** with a clear title and description; reference any
   related issues.

## Commit Message Convention

Follow [Conventional Commits][cc]. Subject lines stay under 50
characters and use the imperative mood:

```text
feat(http): add /supported retry on 429
fix(svm): require extra.feePayer to match facilitator signers
docs(security): publish private disclosure procedure
```

[cc]: https://www.conventionalcommits.org/en/v1.0.0/

Allowed types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`,
`test`, `chore`, `ci`, `build`, `revert`. Multi-paragraph bodies are
welcome for non-trivial changes; explain *why*, not just *what*.

## Reviewing Changes

If you are reviewing a PR, check for:

- Cross-SDK compatibility (new wire fields are reflected in fixtures or
  the audit notes call out the divergence).
- Security implications (signature handling, fee-payer checks, replay
  protections).
- Breaking changes appearing in `CHANGELOG.md` and `MIGRATION.md`.
- New `unsafe`, `unwrap`, or `expect` usages — they need a comment with
  the safety / panic argument.

## Reporting Security Issues

Do **not** open a public issue. Follow [`SECURITY.md`](SECURITY.md) for
the private disclosure procedure.

## Licence

By contributing you agree your work is dual-licensed under the MIT and
Apache-2.0 licences distributed with this repository.
