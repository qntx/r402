#!/usr/bin/env bash
# Fail if any foo.rs sits beside foo/.
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

exit "$fail"
