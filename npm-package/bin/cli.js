#!/usr/bin/env node

/**
 * iCloud Calendar MCP Server - npm wrapper
 *
 * Launches the native server binary that postinstall downloaded for this
 * platform, downloading it first if postinstall was skipped.
 */

const { spawn } = require('child_process');

const pkg = require('../package.json');
const { ensureBinary, RELEASE_URL } = require('../lib/install');

// ANSI colors
const RED = '\x1b[31m';
const GREEN = '\x1b[32m';
const YELLOW = '\x1b[33m';
const RESET = '\x1b[0m';

function log(msg) {
  console.error(msg);
}

function checkCredentials() {
  const username = process.env.ICLOUD_USERNAME;
  const password = process.env.ICLOUD_PASSWORD;

  if (!username || !password) {
    return {
      ok: false,
      error: `Missing credentials. Set environment variables:
  export ICLOUD_USERNAME="your-apple-id@icloud.com"
  export ICLOUD_PASSWORD="your-app-specific-password"

Get an app-specific password at: https://appleid.apple.com`
    };
  }
  return { ok: true };
}

function showHelp() {
  console.log(`
${GREEN}iCloud Calendar MCP Server${RESET}

A security-first MCP server for iCloud Calendar access.

${YELLOW}Usage:${RESET}
  npx @royli/icloud-calendar-mcp

${YELLOW}Environment Variables (required):${RESET}
  ICLOUD_USERNAME    Your Apple ID email
  ICLOUD_PASSWORD    App-specific password (NOT your Apple ID password)

${YELLOW}Claude Desktop Configuration:${RESET}
  Add to ~/Library/Application Support/Claude/claude_desktop_config.json:

  {
    "mcpServers": {
      "icloud-calendar": {
        "command": "npx",
        "args": ["@royli/icloud-calendar-mcp"],
        "env": {
          "ICLOUD_USERNAME": "your-apple-id@icloud.com",
          "ICLOUD_PASSWORD": "your-app-specific-password"
        }
      }
    }
  }

${YELLOW}Get App-Specific Password:${RESET}
  https://appleid.apple.com → Security → App-Specific Passwords

${YELLOW}More Info:${RESET}
  https://github.com/geekdada/icloud-calendar-mcp
`);
}

function run(binary) {
  const server = spawn(binary, [], {
    stdio: 'inherit',
    env: process.env
  });

  server.on('error', (err) => {
    log(`${RED}Failed to start the server:${RESET} ${err.message}`);
    process.exit(1);
  });

  server.on('exit', (code) => {
    process.exit(code || 0);
  });

  // Forward signals
  process.on('SIGINT', () => server.kill('SIGINT'));
  process.on('SIGTERM', () => server.kill('SIGTERM'));
}

async function main() {
  const args = process.argv.slice(2);

  // Handle --help and --version
  if (args.includes('--help') || args.includes('-h')) {
    showHelp();
    process.exit(0);
  }

  if (args.includes('--version') || args.includes('-v')) {
    console.log(`icloud-calendar-mcp v${pkg.version}`);
    process.exit(0);
  }

  const credCheck = checkCredentials();
  if (!credCheck.ok) {
    log(`${RED}Error:${RESET} ${credCheck.error}`);
    process.exit(1);
  }

  let binary;
  try {
    binary = await ensureBinary();
  } catch (err) {
    log(`${RED}Error:${RESET} ${err.message}`);
    log(`Release assets: ${RELEASE_URL}`);
    process.exit(1);
  }

  run(binary);
}

main();
