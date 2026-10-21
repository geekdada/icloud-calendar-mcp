/**
 * Maps the running platform to its release asset, and parses the SHA256SUMS
 * file published with each release. Pure functions, so they can be unit tested.
 */

// Linux builds are static musl binaries, so they run on glibc and musl distros alike.
const TARGETS = {
  'linux-x64': 'x86_64-unknown-linux-musl',
  'linux-arm64': 'aarch64-unknown-linux-musl',
  'darwin-x64': 'x86_64-apple-darwin',
  'darwin-arm64': 'aarch64-apple-darwin',
  'win32-x64': 'x86_64-pc-windows-msvc',
};

const BINARY_BASENAME = 'icloud-calendar-mcp';

/**
 * Returns `{ asset, binaryName }` for a Node `process.platform` / `process.arch`
 * pair, or `null` when no release binary exists for it.
 */
function resolveAsset(platform, arch) {
  const target = TARGETS[`${platform}-${arch}`];
  if (!target) {
    return null;
  }
  const extension = platform === 'win32' ? '.exe' : '';
  return {
    asset: `${BINARY_BASENAME}-${target}${extension}`,
    binaryName: `${BINARY_BASENAME}${extension}`,
  };
}

function supportedPlatforms() {
  return Object.keys(TARGETS);
}

/**
 * Parses `sha256sum` output (`<hex>  <name>` or `<hex> *<name>` per line) into
 * a Map from file name to lowercase hex digest. Malformed lines are skipped.
 */
function parseChecksums(text) {
  const checksums = new Map();
  for (const line of text.split(/\r?\n/)) {
    const match = line.trim().match(/^([0-9a-fA-F]{64}) [ *](.+)$/);
    if (match) {
      checksums.set(match[2], match[1].toLowerCase());
    }
  }
  return checksums;
}

module.exports = { resolveAsset, supportedPlatforms, parseChecksums };
