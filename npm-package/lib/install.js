/**
 * Downloads the native server binary for this platform from GitHub Releases
 * and verifies it against the SHA256SUMS file bundled into the package at
 * publish time. Used by postinstall, and by the CLI when postinstall was
 * skipped (`--ignore-scripts`, pnpm's default script policy).
 *
 * Everything is logged to stderr: when the CLI calls this, stdout is the
 * MCP JSON-RPC channel.
 */

const crypto = require('crypto');
const fs = require('fs');
const https = require('https');
const path = require('path');

const { resolveAsset, supportedPlatforms, parseChecksums } = require('./platform');

const pkg = require('../package.json');
const RELEASE_URL = `https://github.com/icloud-calendar-mcp/icloud-calendar-mcp/releases/download/v${pkg.version}`;
const CHECKSUMS_PATH = path.join(__dirname, 'SHA256SUMS');
const BIN_DIR = path.join(__dirname, 'bin');
const MAX_REDIRECTS = 5;

function log(message) {
  console.error(message);
}

function platformAsset() {
  const resolved = resolveAsset(process.platform, process.arch);
  if (!resolved) {
    throw new Error(
      `No prebuilt binary for ${process.platform}-${process.arch}. ` +
        `Supported: ${supportedPlatforms().join(', ')}. ` +
        'Build from source instead: https://github.com/icloud-calendar-mcp/icloud-calendar-mcp#option-4-build-from-source',
    );
  }
  return resolved;
}

function binaryPath() {
  return path.join(BIN_DIR, platformAsset().binaryName);
}

function expectedChecksum(asset) {
  if (!fs.existsSync(CHECKSUMS_PATH)) {
    throw new Error(`${CHECKSUMS_PATH} is missing; this package was not built by the release workflow.`);
  }
  const checksum = parseChecksums(fs.readFileSync(CHECKSUMS_PATH, 'utf8')).get(asset);
  if (!checksum) {
    throw new Error(`SHA256SUMS has no entry for ${asset}.`);
  }
  return checksum;
}

function sha256File(file) {
  return crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
}

function download(url, dest, redirectsLeft = MAX_REDIRECTS) {
  return new Promise((resolve, reject) => {
    https
      .get(url, (response) => {
        try {
          handleResponse(response);
        } catch (err) {
          response.resume();
          reject(err);
        }
      })
      .on('error', reject);

    function handleResponse(response) {
      const { statusCode, headers } = response;
      if ([301, 302, 303, 307, 308].includes(statusCode) && headers.location) {
        response.resume();
        if (redirectsLeft === 0) {
          reject(new Error(`Too many redirects downloading ${url}`));
          return;
        }
        const next = new URL(headers.location, url).toString();
        download(next, dest, redirectsLeft - 1).then(resolve, reject);
        return;
      }
      if (statusCode !== 200) {
        response.resume();
        reject(new Error(`Download failed: HTTP ${statusCode} for ${url}`));
        return;
      }
      const file = fs.createWriteStream(dest);
      response.pipe(file);
      file.on('finish', () => file.close(resolve));
      file.on('error', reject);
      response.on('error', reject);
    }
  });
}

/** True when the binary is present and matches its published checksum. */
function isInstalled() {
  const { asset } = platformAsset();
  const target = binaryPath();
  return fs.existsSync(target) && sha256File(target) === expectedChecksum(asset);
}

/** Downloads, verifies and installs the binary. Returns its path. */
async function install() {
  const { asset } = platformAsset();
  const checksum = expectedChecksum(asset);
  const target = binaryPath();
  const partial = `${target}.download`;
  const url = `${RELEASE_URL}/${asset}`;

  fs.mkdirSync(BIN_DIR, { recursive: true });
  log(`Downloading iCloud Calendar MCP server: ${url}`);
  try {
    await download(url, partial);
    const actual = sha256File(partial);
    if (actual !== checksum) {
      throw new Error(`Checksum mismatch for ${asset}: expected ${checksum}, got ${actual}`);
    }
    fs.chmodSync(partial, 0o755);
    fs.renameSync(partial, target);
  } finally {
    fs.rmSync(partial, { force: true });
  }
  log('Download complete, checksum verified.');
  return target;
}

/** Returns the path of a verified binary, downloading it first if needed. */
async function ensureBinary() {
  return isInstalled() ? binaryPath() : install();
}

module.exports = { ensureBinary, binaryPath, RELEASE_URL };
