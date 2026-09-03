#!/usr/bin/env bash
# Fail if any foo.rs sits beside foo/, or if obsolete crate names remain.
set -euo pipefail
root="$(git rev-parse --show-toplevel)"
cd "$root"
fail=0

while IFS= read -r f; do
  [[ "$f" == */mod.rs || "$f" == */lib.rs || "$f" == */main.rs ]] && continue
  dir="${f%.rs}"
  if [[ -d "$dir" ]]; then
    echo "dual module root: $f + $dir/"
    fail=1
  fi
done < <(git ls-files '*.rs')

for d in crates/r402-solana crates/r402-algorand; do
  if [[ -e "$d" ]]; then
    echo "obsolete crate path: $d"
    fail=1
  fi
done

# Hyphen = Cargo package; underscore = Rust ident.
# Restrict to product paths so this script cannot self-match
# (`for d in crates/r402-solana …` lives in scripts/).
# Also exclude the script and CHANGELOG if the path list grows.
if git grep -nE 'r402[-_]solana|r402[-_]algorand' -- \
    Cargo.toml Cargo.lock crates .github README.md Justfile CONTRIBUTING.md \
    ':!CHANGELOG.md' ':!scripts/check-layout.sh' >/dev/null; then
  echo "obsolete crate name in product files:"
  git grep -nE 'r402[-_]solana|r402[-_]algorand' -- \
    Cargo.toml Cargo.lock crates .github README.md Justfile CONTRIBUTING.md \
    ':!CHANGELOG.md' ':!scripts/check-layout.sh' || true
  fail=1
fi

exit "$fail"
