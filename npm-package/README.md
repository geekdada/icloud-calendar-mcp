# iCloud Calendar MCP Server

[![npm](https://img.shields.io/npm/v/@royli/icloud-calendar-mcp.svg)](https://www.npmjs.com/package/@royli/icloud-calendar-mcp)
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://github.com/geekdada/icloud-calendar-mcp/blob/main/LICENSE)
[![Tests](https://img.shields.io/badge/Tests-1387%20passing-brightgreen.svg)](https://github.com/geekdada/icloud-calendar-mcp)
[![Security](https://img.shields.io/badge/Security-OWASP%20MCP%20Top%2010-blue.svg)](https://github.com/geekdada/icloud-calendar-mcp/blob/main/SECURITY.md)

An MCP (Model Context Protocol) server for iCloud Calendar access via CalDAV.

## Installation

```bash
npx @royli/icloud-calendar-mcp
```

Or install globally:

```bash
npm install -g @royli/icloud-calendar-mcp
icloud-calendar-mcp
```

## Requirements

- Node.js 18+, on Linux (x64, arm64), macOS (Intel, Apple silicon) or Windows (x64). The package downloads the native server binary for your platform from the GitHub release and verifies its SHA-256 checksum; no Java is needed.
- iCloud account with [app-specific password](https://support.apple.com/en-us/HT204397)

## Setup

### 1. Get an App-Specific Password

1. Go to [appleid.apple.com](https://appleid.apple.com)
2. Sign in and go to **Security** > **App-Specific Passwords**
3. Generate a new password for "iCloud Calendar MCP"

### 2. Set Environment Variables

```bash
export ICLOUD_USERNAME="your-apple-id@icloud.com"
export ICLOUD_PASSWORD="your-app-specific-password"
```

### 3. Configure Claude Desktop

Add to `~/Library/Application Support/Claude/claude_desktop_config.json`:

```json
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
```

## Available Tools

| Tool | Description |
|------|-------------|
| `list_calendars` | List all available calendars |
| `get_events` | Get events from a calendar |
| `create_event` | Create a new calendar event |
| `update_event` | Update an existing event |
| `delete_event` | Delete an event |

## Security

This server is designed with security as a primary concern:

- **App-Specific Passwords** - Never use your main Apple ID password
- **Environment Variables Only** - Credentials loaded from environment only
- **Input Validation** - All inputs validated with SSRF protection
- **Rate Limiting** - 60 reads/min, 20 writes/min
- **Error Sanitization** - No credentials leaked in error messages
- **OWASP MCP Top 10** - Full compliance with 282 security tests
- **ReDoS Protection** - All regex patterns tested against DoS
- **Unicode Security** - Protection against encoding attacks

## Test Coverage

- **843 total tests**
- **282 security tests** covering OWASP MCP Top 10
- ReDoS, Unicode, SSRF, injection protection

## Links

- [GitHub Repository](https://github.com/geekdada/icloud-calendar-mcp)
- [Security Policy](https://github.com/geekdada/icloud-calendar-mcp/blob/main/SECURITY.md)
- [MCP Documentation](https://modelcontextprotocol.io)

## License

Apache-2.0
