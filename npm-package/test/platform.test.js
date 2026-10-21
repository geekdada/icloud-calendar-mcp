const assert = require('node:assert/strict');
const { test } = require('node:test');

const { resolveAsset, supportedPlatforms, parseChecksums } = require('../lib/platform');

test('maps every supported platform to a release asset', () => {
  assert.deepEqual(resolveAsset('linux', 'x64'), {
    asset: 'icloud-calendar-mcp-x86_64-unknown-linux-musl',
    binaryName: 'icloud-calendar-mcp',
  });
  assert.equal(resolveAsset('linux', 'arm64').asset, 'icloud-calendar-mcp-aarch64-unknown-linux-musl');
  assert.equal(resolveAsset('darwin', 'x64').asset, 'icloud-calendar-mcp-x86_64-apple-darwin');
  assert.equal(resolveAsset('darwin', 'arm64').asset, 'icloud-calendar-mcp-aarch64-apple-darwin');
  assert.deepEqual(resolveAsset('win32', 'x64'), {
    asset: 'icloud-calendar-mcp-x86_64-pc-windows-msvc.exe',
    binaryName: 'icloud-calendar-mcp.exe',
  });
  assert.equal(supportedPlatforms().length, 5);
});

test('returns null for platforms without a release binary', () => {
  assert.equal(resolveAsset('win32', 'arm64'), null);
  assert.equal(resolveAsset('freebsd', 'x64'), null);
  assert.equal(resolveAsset('linux', 'ia32'), null);
});

test('parses sha256sum output in text and binary mode', () => {
  const upper = 'A'.repeat(64);
  const lower = 'b'.repeat(64);
  const checksums = parseChecksums(`${upper}  icloud-calendar-mcp-x86_64-apple-darwin\r\n${lower} *icloud-calendar-mcp-x86_64-pc-windows-msvc.exe\n`);
  assert.equal(checksums.get('icloud-calendar-mcp-x86_64-apple-darwin'), 'a'.repeat(64));
  assert.equal(checksums.get('icloud-calendar-mcp-x86_64-pc-windows-msvc.exe'), lower);
});

test('skips malformed checksum lines', () => {
  const checksums = parseChecksums(`not-a-hash  file\n${'c'.repeat(63)}  short\n\n`);
  assert.equal(checksums.size, 0);
});
