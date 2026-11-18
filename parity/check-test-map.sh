#!/usr/bin/env bash
# Fails if a Go test in parity/go-tests.txt has no Rust `#[test]` with its snake_case name (TestFooBar ->
# foo_bar), unless parity/test-map-exceptions.txt lists it with a reason.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

rust_tests=$(grep -rh -A1 -E '^\s*#\[(tokio::)?test\]' crates/gpu-process-exporter/src |
  sed -nE 's/^\s*(async )?fn ([a-z0-9_]+)\(.*/\2/p' | sort -u)
exceptions=$(sed -nE 's/^(Test[A-Za-z0-9]+)\s+#.*/\1/p' parity/test-map-exceptions.txt)

missing=0
while read -r go_test; do
  grep -qx "$go_test" <<<"$exceptions" && continue
  rust_test=$(sed -E 's/^Test//; s/([a-z0-9])([A-Z])/\1_\2/g; s/([A-Z])([A-Z][a-z])/\1_\2/g' <<<"$go_test" |
    tr '[:upper:]' '[:lower:]')
  if ! grep -qx "$rust_test" <<<"$rust_tests"; then
    echo "missing Rust test $rust_test for $go_test"
    missing=1
  fi
done < <(grep -v '^#' parity/go-tests.txt)
exit $missing
