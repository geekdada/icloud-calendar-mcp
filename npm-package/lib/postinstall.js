#!/usr/bin/env node

/**
 * Postinstall script - downloads the native server binary from GitHub Releases.
 */

const { ensureBinary, RELEASE_URL } = require('./install');

ensureBinary()
  .then(() => {
    console.error('iCloud Calendar MCP server installed. Usage: npx @royli/icloud-calendar-mcp --help');
  })
  .catch((err) => {
    console.error(`Error installing the iCloud Calendar MCP server: ${err.message}`);
    console.error(`Release assets: ${RELEASE_URL}`);
    process.exit(1);
  });
