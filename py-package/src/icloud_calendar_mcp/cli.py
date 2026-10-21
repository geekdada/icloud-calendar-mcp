#!/usr/bin/env python3
"""
iCloud Calendar MCP Server - Python wrapper

Launches the native server binary for this platform, downloading and
verifying it on first run.
"""

import os
import subprocess
import sys

from icloud_calendar_mcp.downloader import RELEASE_URL, InstallError, ensure_binary


def check_credentials() -> tuple[bool, str]:
    """Check if iCloud credentials are set."""
    username = os.environ.get("ICLOUD_USERNAME")
    password = os.environ.get("ICLOUD_PASSWORD")

    if not username or not password:
        return False, """Missing credentials. Set environment variables:
  export ICLOUD_USERNAME="your-apple-id@icloud.com"
  export ICLOUD_PASSWORD="your-app-specific-password"

Get an app-specific password at: https://appleid.apple.com"""
    return True, ""


def show_help():
    """Display help information."""
    print("""
\033[32miCloud Calendar MCP Server\033[0m

A security-first MCP server for iCloud Calendar access.

\033[33mUsage:\033[0m
  uvx icloud-calendar-mcp
  python -m icloud_calendar_mcp

\033[33mEnvironment Variables (required):\033[0m
  ICLOUD_USERNAME    Your Apple ID email
  ICLOUD_PASSWORD    App-specific password (NOT your Apple ID password)

\033[33mClaude Desktop Configuration:\033[0m
  Add to ~/Library/Application Support/Claude/claude_desktop_config.json:

  {
    "mcpServers": {
      "icloud-calendar": {
        "command": "uvx",
        "args": ["icloud-calendar-mcp"],
        "env": {
          "ICLOUD_USERNAME": "your-apple-id@icloud.com",
          "ICLOUD_PASSWORD": "your-app-specific-password"
        }
      }
    }
  }

\033[33mGet App-Specific Password:\033[0m
  https://appleid.apple.com -> Security -> App-Specific Passwords

\033[33mMore Info:\033[0m
  https://github.com/icloud-calendar-mcp/icloud-calendar-mcp
""")


def main():
    """Main entry point."""
    args = sys.argv[1:]

    # Handle --help and --version
    if "--help" in args or "-h" in args:
        show_help()
        sys.exit(0)

    if "--version" in args or "-v" in args:
        from icloud_calendar_mcp import __version__
        print(f"icloud-calendar-mcp v{__version__}")
        sys.exit(0)

    cred_ok, cred_msg = check_credentials()
    if not cred_ok:
        print(f"\033[31mError:\033[0m {cred_msg}", file=sys.stderr)
        sys.exit(1)

    try:
        binary = ensure_binary()
    except (InstallError, OSError) as error:
        print(f"\033[31mError:\033[0m {error}", file=sys.stderr)
        print(f"Release assets: {RELEASE_URL}", file=sys.stderr)
        sys.exit(1)

    try:
        process = subprocess.run([str(binary)], env=os.environ)
        sys.exit(process.returncode)
    except KeyboardInterrupt:
        sys.exit(0)
    except Exception as e:
        print(f"\033[31mFailed to start the server:\033[0m {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
