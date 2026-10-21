# iCloud Calendar MCP server (Rust)

The server's source. It speaks MCP over STDIO and builds to a single native binary. The npm package in `npm-package/` downloads the release builds of this binary.

## Build and run

```bash
cd rust
cargo build --release
ICLOUD_USERNAME="you@icloud.com" \
ICLOUD_PASSWORD="xxxx-xxxx-xxxx-xxxx" \
./target/release/icloud-calendar-mcp
```

Use an [app-specific password](https://support.apple.com/en-us/HT204397). Without credentials the server still starts, and every tool returns `AUTH_ERROR`.

Claude Desktop config:

```json
{
  "mcpServers": {
    "icloud-calendar": {
      "command": "/path/to/icloud-calendar-mcp",
      "env": {
        "ICLOUD_USERNAME": "your-apple-id@icloud.com",
        "ICLOUD_PASSWORD": "your-app-specific-password"
      }
    }
  }
}
```

Set `CALDAV_LOG_HTTP=true` to log CalDAV requests and responses to stderr. stdout carries JSON-RPC only.

## Layout

| Module | Contents |
|--------|----------|
| `server/` | Tool, resource and prompt registration, argument parsing, rate limiting at the tool boundary |
| `service/` | `CalendarService`, event handles, the short-lived event cache |
| `caldav/` | CalDAV client on reqwest, iCloud XML parsing on roxmltree, ETag and URL normalization |
| `ics/` | ICS parser, builder and patcher |
| `ical/` | iCalendar model, content-line parser, generator, recurrence expansion, VTIMEZONE generation |
| `validation.rs`, `error.rs`, `ratelimit.rs`, `credentials.rs`, `logging.rs`, `progress.rs`, `cancellation.rs` | Input validation, error sanitization and the other security controls |

`ical/` contains a content-line parser, a VEVENT/VALARM model, a generator, an RFC 5545 recurrence iterator with fixed work bounds (10,000 instances per series, plus a cap on periods examined), and VTIMEZONE generation from the IANA database via chrono-tz.

## Tests

```bash
cargo test                      # hermetic: mocks and wiremock, no network
TZ=Asia/Seoul cargo test        # same suite in a non-UTC zone
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

CI builds with the latest stable Rust; the crate's `rust-version` (1.85) is the oldest toolchain it supports.

The live suite talks to real iCloud and is ignored by default:

```bash
ICLOUD_USERNAME=... ICLOUD_PASSWORD=... \
  cargo test --test live_icloud -- --ignored --test-threads=1
```

Each live test creates its own events with an `MCP-LIVE:` prefix and deletes them afterwards, even when the test fails.
