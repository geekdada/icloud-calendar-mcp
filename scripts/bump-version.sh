#!/usr/bin/env bash
# Set the project version everywhere it is committed, from one argument.
#
# rust/Cargo.toml is the source of truth; the npm wrapper must agree with it,
# because it downloads the binaries from the release named after its own
# version. The docs contain no version. They link the constant
# releases/latest/download asset names.
#
# After running this, commit the result and tag v<version>. CI enforces the rest:
# scripts/check-version.sh fails a PR on any drift, and the release workflow refuses
# to build if the tag does not match rust/Cargo.toml.
#
# Uses perl for in-place edits so it works the same on GNU and BSD (macOS) userlands.
set -euo pipefail

cd "$(dirname "$0")/.."

new="${1:-}"
if ! printf '%s' "$new" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.-]+)?$'; then
  echo "usage: scripts/bump-version.sh <version>   (e.g. 3.3.0)" >&2
  exit 1
fi

edit() {  # edit <file> <perl-expression>
  perl -i -pe "$2" "$1"
  echo "  updated $1"
}

edit rust/Cargo.toml                                  "s/^version = \".*\"/version = \"$new\"/ if \$. <= 5"
perl -0 -i -pe "s/(name = \"icloud-calendar-mcp\"\nversion = \")[^\"]*\"/\${1}$new\"/" rust/Cargo.lock
echo "  updated rust/Cargo.lock"
edit server.json                                      "s/\"version\": \"[^\"]*\"/\"version\": \"$new\"/g"
edit npm-package/package.json                         "s/\"version\": \"[^\"]*\"/\"version\": \"$new\"/"

echo ""
echo "Bumped to $new. Verifying..."
bash scripts/check-version.sh
