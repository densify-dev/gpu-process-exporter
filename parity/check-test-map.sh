#!/usr/bin/env bash
# Fails if a Go `func TestXxx` has no Rust `#[test]` named `xxx` (snake_case), unless listed in
# parity/test-map-exceptions.txt with a reason.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

rust_tests=$(grep -rh -A1 -E '^\s*#\[(tokio::)?test\]' crates/gpu-process-exporter/src |
  sed -nE 's/^\s*(async )?fn ([a-z0-9_]+)\(.*/\2/p' | sort -u)
exceptions=$(sed -nE 's/^(Test[A-Za-z0-9]+)\s+#.*/\1/p' parity/test-map-exceptions.txt)

missing=0
for go_test in $(grep -h '^func Test' pkg/*/*_test.go | sed -E 's/^func (Test[A-Za-z0-9]+).*/\1/'); do
  grep -qx "$go_test" <<<"$exceptions" && continue
  rust_test=$(sed -E 's/^Test//; s/([a-z0-9])([A-Z])/\1_\2/g; s/([A-Z])([A-Z][a-z])/\1_\2/g' <<<"$go_test" | tr 'A-Z' 'a-z')
  if ! grep -qx "$rust_test" <<<"$rust_tests"; then
    echo "missing Rust test $rust_test for $go_test"
    missing=1
  fi
done
exit $missing
