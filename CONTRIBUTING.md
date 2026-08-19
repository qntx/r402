# Contributing to r402

This document covers what you need before opening a pull request.

[x402]: https://www.x402.org

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

The MSRV is pinned to **Rust 1.91** in `rust-toolchain.toml`. CI builds
on stable; match that locally with `rustup`.

The quality bar in [`Justfile`](Justfile) uses `cargo +nightly` for
`fmt`, `clippy`, and rustdoc. Install nightly once (`rustup toolchain
install nightly`) if you run those recipes.

### Quality Gates (run before pushing)

```bash
just all
just test
```

Equivalent cargo invocations:

```bash
cargo +nightly fmt --all --check
cargo +nightly clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo +nightly doc --workspace --all-features --no-deps
cargo test --workspace --all-features
```

For dependency hygiene:

```bash
cargo install cargo-audit cargo-deny
cargo audit
cargo deny check all
```

## Pull Request Workflow

1. **Open an issue** if your change is non-trivial. Architecture-level
   changes benefit from a design discussion before coding.
2. **Branch from `main`** — keep PRs focused on one logical concern.
3. **Add tests** alongside any behaviour change. Wire types and schemes
   should round-trip the fixtures in
   `crates/r402-core/tests/fixtures/spec_v2/`.
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
fix(svm): require extra.feePayer to match facilitator signers
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
