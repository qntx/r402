# Contributing to r402

Thank you for considering a contribution. This document codifies the engineering
standards that every patch is expected to meet. A PR that does not meet these
standards will be sent back for rework regardless of the correctness of the fix
itself — review bandwidth is finite and "it works" is the floor, not the bar.

The standards are not arbitrary. They exist because `r402` is a payment-protocol
SDK where a subtle bug can silently burn users' money on-chain, and because the
codebase has to stay legible to newcomers years from now. When in doubt, optimise
for the reader who will be debugging a live settlement failure at 3 AM.

---

## 1. Tooling

The local loop is the same as CI. Any of these failing is a blocker.

```bash
cargo fmt --all --check                                      # zero diff
cargo clippy --workspace --all-targets --all-features -- -D warnings  # zero warning
cargo test --workspace --all-features                        # zero failure
cargo doc --workspace --all-features --no-deps               # zero warning
```

Run them **before** opening the PR. CI is a safety net, not a first-pass linter.

---

## 2. Module and file organisation

- **One file, one concept.** Files above ~300 lines are reviewed for whether they
  are actually carrying two jobs. Tests do not count toward the limit.
- **Module depth ≤ 3.** `crate::domain::entity` is fine, `crate::a::b::c::d` is a
  smell. Flatten or regroup.
- **Visibility is pessimistic.** Default to `pub(crate)`. Only promote to `pub`
  when the item genuinely crosses a crate boundary. `pub(super)` and
  `pub(in path)` are preferred over blanket `pub`.
- **Imports are grouped.** `std` → third-party crates → `crate::` with a blank
  line between groups. Enforced manually because `group_imports` is still
  nightly.
- **`mod` declarations come first in a file**, before `use` statements.

---

## 3. Type system discipline

The compiler is our first line of defence. Every type erased at the boundary is
a runtime bug waiting to happen.

- **No naked `String` for domain data.** `ChainId`, `ChecksummedAddress`,
  `TokenAmount`, `Memo`, etc. A free `String` parameter is a code smell unless
  it is genuinely free-form text (log messages, user-visible strings).
- **No naked `u64`/`U256` for amounts.** Use `TokenAmount` or a newtype that
  carries decimals/semantics. The existing `MoneyAmount` is the reference pattern.
- **No two-or-more `bool` parameters.** Promote to an `enum` or a builder struct.
  One `bool` is tolerable if the site reads unambiguously (`is_production: bool`).
- **Library errors are typed.** `anyhow::Result` is banned from library public
  APIs. Each module defines its own `Error` enum with `thiserror::Error`.
  Binaries may use `anyhow` in `main`.
- **`unwrap`/`expect` is rare.** In library code, either handle the error or
  propagate with `?`. An `.expect("reason")` is acceptable only when the
  invariant is mathematically or syntactically guaranteed, and the reason must
  be in the message. `clippy::unwrap_used` is `warn` workspace-wide.

---

## 4. API ergonomics

- **Builder pattern for more than three construction parameters.**
- **Accept `impl Into<…>` or `&str`** where the caller should not be forced to
  allocate a `String`.
- **Return `impl Iterator` before `Vec`** unless the caller almost always
  collects; let the caller decide.
- **`Default` for every configuration struct** with sensible defaults.
- **`From`/`TryFrom` over `from_foo` methods.** Natural conversions belong to
  the standard traits.

---

## 5. Tests

This is where contributions most often regress. Read this section carefully.

### 5.1 Location

Every file's tests live in a **single** `#[cfg(test)] mod tests { … }` at the
**bottom** of the file. No exceptions, no custom names, no second module, no
production code below the test module. `clippy::tests_outside_test_module` and
`items-after-test-module` enforce this; do not work around the lints.

When a test requires a feature flag, stack the attributes:

```rust
#[cfg(test)]
#[cfg(any(feature = "facilitator", feature = "client"))]
mod tests { … }
```

Do **not** write `#[cfg(all(test, feature = "x"))]` on the module — clippy
cannot see through it and will (correctly) complain that the tests are outside
a `#[cfg(test)]` module.

### 5.2 Naming

Test function names follow `<subject>_<scenario>_<expectation>`. A few examples
from the existing codebase you can copy from:

```rust
fn witness_typehash_equals_spec_byte_string()
fn parse_rejects_precision_higher_than_decimals()
fn hooks_run_in_registration_order()
```

Avoid: `accepts_*`, `rejects_*`, `it_works`, `test1`, or anything that would
read the same if the assertion inside were negated.

### 5.3 Quality

A test earns its place by being able to fail **independently** of the code
under test. If the assertion can only fail when the production constant it
reproduces is intentionally changed in the same commit, it is self-referential
and has no regression value — delete it.

Red flags that cause a PR to be sent back:

- **Self-reference:** `assert_eq!(CONSTANT.to_string(), "0x…")` where the string
  is literally the constant itself. Either cross-check against an independent
  source (spec byte string, on-chain RPC, reference SDK output) or remove the
  test.
- **Tautology:** `assert_eq!(1 + 1, 2)`, `assert!(Some(()).is_some())`. If the
  test body re-implements the function under test, it tests nothing.
- **Ceremony around a single operator:** a four-test module guarding
  `if a != b { err }` is noise. Inline the logic with a comment explaining why
  it matters. Helpers exist to hide complexity, not to manufacture test counts.
- **Business-impossible scenarios:** a test for `amount == 0` when the business
  layer guarantees `amount > 0` is a placeholder, not a test.

Prefer:

- **External authority vectors:** byte strings from the spec, signatures from
  a reference SDK's fixtures, responses from a deterministic test node.
- **Behaviour over shape:** assert that a state transition happened, not that
  a field exists.
- **`assert_eq!` over `assert!`** for comparisons — failure messages are
  strictly better. Use `matches!` for enum shape checks.

### 5.4 Mocks

Mocks are normal Rust types implementing the relevant trait — see
`@r402/src/hooks.rs` for the canonical `MockFacilitator` pattern. Do not reach
for `mockall` unless a handwritten mock becomes unmaintainable.

---

## 6. Documentation

- **Every public item carries a `///` doc comment.** No exceptions for
  "obvious" types — what is obvious to the author is rarely obvious to the
  reader three years later.
- **`# Errors` and `# Panics` sections** on fallible / panicking functions.
  `clippy::missing_errors_doc` and `missing_panics_doc` are `warn`.
- **Why, not what.** `// increment counter` above `counter += 1` is noise;
  delete it. `// wrap around intentionally — discovery is best-effort` earns
  its keep.
- **Doctests for public APIs.** A function whose doc does not show how to call
  it is a function the user will call wrong.

---

## 7. Commit and PR hygiene

- **Conventional Commits, imperative mood, English.** `fix(evm): …`,
  `feat(svm)!: …`, `refactor(http): …`. The `!` marker and a
  `BREAKING CHANGE:` trailer are mandatory for any public-API change.
- **One logical change per commit.** If a PR description needs the word "also",
  split it.
- **Tests and docs land in the same commit** as the code they cover — never as
  a trailing follow-up.
- **The PR description links to the spec section, issue, or audit finding** the
  change addresses. `r402` is a protocol SDK; changes without a written rationale
  are presumed wrong.

---

## 8. When in doubt

Read a recent merged commit that touched the same area. The project's best
existing patterns are the specification; this document only writes down what
the code already does well.
