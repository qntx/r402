# Contributing to r402

This document covers what you need before opening a pull request.

## Code of Conduct

Be kind, technical, and concise. No harassment, no surprises, no
backchannel exploits. Report vulnerabilities through GitHub security
advisories, not public issues. Features and bugs go through GitHub
issues and pull requests.

## Development Setup

```bash
git clone https://github.com/qntx/r402
cd r402

# Build the entire workspace with all features.
cargo build --workspace --all-features

# Run unit + integration + doc tests across the workspace.
cargo test --workspace --all-features
```

The MSRV is **Rust 1.95** (`workspace.package.rust-version` in `Cargo.toml`).
CI builds on stable; match that locally with `rustup`.

`--all-features` (and `r402-hedera` `client`/`facilitator`/`full`) needs
`protoc` on `PATH` and OpenSSL headers (`brew install protobuf openssl` /
`apt-get install protobuf-compiler libssl-dev pkg-config`). The Hiero SDK
compiles `hedera-proto` at build time; there is no vendored `protoc`.

CI clippy is **stable** `-D warnings` via `qntx/workflows`
`ci-rust.yml@v2` (GitHub currently rustc 1.98). Do not add
`-A unknown-lints` to CI. Justfile `fmt` / `clippy` / `doc` stay
`cargo +nightly` and are **local-only**. Install nightly once
(`rustup toolchain install nightly`) if you run those recipes.

### Quality Gates (run before pushing)

```bash
just all
```

CI-equivalent cargo invocations:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Local Justfile equivalents (`+nightly`, not CI):

```bash
cargo +nightly fmt --all --check
cargo +nightly clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo +nightly doc --workspace --all-features --no-deps
```

For dependency hygiene:

```bash
cargo install cargo-audit cargo-deny
cargo audit
cargo deny check all
```

## TOML style

Match kobe’s grouped `=` alignment. Inside one table, a contiguous run of
`key = value` lines (no blank line) is a group; pad keys so every `=` in
that group shares a column. One space on each side of `=`. Keep the
file’s existing trailing-comma style. Do not reorder keys to make
padding easier. There is no taplo/dprint gate; rustfmt remains the only
format check.

## Versioning and publish

Crate version is `[workspace.package].version`. The git tag is `v` plus
that string.

Path workspace deps use the **minor** (`version = "0.19"`, i.e. `^0.19`)
so `cargo publish` can rewrite them to crates.io. **Patch** bumps
(`0.19.1` → `0.19.2`) leave path `version = "0.19"`. **Minor / major**
(`0.19` → `0.20` or `1.0`) update that field in the **same PR** as
`workspace.package.version`.

`publish.yml` Test is `cargo test --workspace` with **default features**,
not `--all-features`. Before tagging, also run that command (CI `just test`
uses `--all-features`). Path deps without a version still fail
`cargo package`; after SIWX is crates.io `0.5`, package every crate in
`publish.yml`.

## Pull Request Workflow

1. **Open an issue** if your change is non-trivial. Architecture-level
   changes benefit from a design discussion before coding.
2. **Branch from `main`** — keep PRs focused on one logical concern.
3. **Add tests** alongside any behaviour change. Wire types and schemes
   should round-trip the fixtures in
   `tests/fixtures/spec_v2/`.
4. **Update the changelog** (`CHANGELOG.md`, Keep-a-Changelog format) if
   you make a user-visible or breaking API change.
5. **Run the quality gates** above. CI will reject `clippy` or `fmt`
   violations.
6. **Open the PR** with a clear title and description; reference any
   related issues.

## Commit Message Convention

Follow [Conventional Commits][cc]. Subject lines stay under 50
characters and use the imperative mood:

```text
feat(http): add /supported retry on 429
fix(solana): require extra.feePayer to match facilitator signers
docs(security): publish private disclosure procedure
```

[cc]: https://www.conventionalcommits.org/en/v1.0.0/

Allowed types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`,
`test`, `chore`, `ci`, `build`, `revert`. Multi-paragraph bodies are
welcome for non-trivial changes; explain *why*, not just *what*.

## Reviewing Changes

If you are reviewing a PR, check for:

- Cross-SDK wire compatibility (new fields are reflected in the
  `spec_v2` fixtures, or the notes call out the divergence).
- Security implications (signature handling, fee-payer checks, replay
  protections).
- Breaking changes appearing in `CHANGELOG.md`.
- New `unsafe`, `unwrap`, or `expect` usages — they need a comment with
  the safety / panic argument.

## Reporting Security Issues

Do **not** open a public issue. Use GitHub security advisories for
private disclosure.

## Licence

By contributing you agree your work is dual-licensed under the MIT and
Apache-2.0 licences distributed with this repository.
