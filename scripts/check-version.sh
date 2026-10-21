#!/usr/bin/env bash
# Verify every committed version string agrees with rust/Cargo.toml (the single
# source of truth). Run in CI on every push and PR, and locally after
# scripts/bump-version.sh. Exits non-zero and lists every mismatch.
set -euo pipefail

cd "$(dirname "$0")/.."

version="$(sed -n 's/^version = "\(.*\)"/\1/p' rust/Cargo.toml | head -n 1)"
if [ -z "$version" ]; then
  echo "error: could not read version from rust/Cargo.toml" >&2
  exit 1
fi
echo "Canonical version (rust/Cargo.toml): $version"

fail=0
report() { echo "  MISMATCH: $1" >&2; fail=1; }

# Each file must carry the exact expected literal.
grep -A1 '^name = "icloud-calendar-mcp"$' rust/Cargo.lock | grep -q "^version = \"$version\"" \
  || report "rust/Cargo.lock version != $version"
grep -q "\"version\": \"$version\"" npm-package/package.json \
  || report "npm-package/package.json version != $version"
grep -q "^version = \"$version\"" py-package/pyproject.toml \
  || report "py-package/pyproject.toml version != $version"
grep -q "^__version__ = \"$version\"" py-package/src/icloud_calendar_mcp/__init__.py \
  || report "py-package __init__.py __version__ != $version"

# server.json carries the version twice (top level + npm package entry).
count="$(grep -c "\"version\": \"$version\"" server.json || true)"
[ "$count" -eq 2 ] || report "server.json has $count of 2 expected \"version\": \"$version\" entries"

if [ "$fail" -ne 0 ]; then
  echo "" >&2
  echo "Version drift detected. Run: scripts/bump-version.sh $version" >&2
  exit 1
fi
echo "All version strings agree with $version."
