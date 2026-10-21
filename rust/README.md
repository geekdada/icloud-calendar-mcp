# iCloud Calendar MCP server (Rust)

This directory holds a Rust port of the Kotlin server in `src/main/kotlin`. It exposes the same tools, resource and prompts over STDIO, with the same argument names, response shapes and error JSON. It builds to a single native binary, so no JVM is needed.

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

| Module | Kotlin counterpart |
|--------|--------------------|
| `server/` | `Main.kt` (tool, resource and prompt registration) |
| `service/` | `service/CalendarService.kt`, `service/EventHandle.kt` |
| `caldav/` | `caldav/*` (reqwest instead of OkHttp, roxmltree instead of DOM) |
| `ics/` | `ics/IcsParser.kt`, `IcsBuilder.kt`, `IcsPatcher.kt` |
| `ical/` | the parts of `icaldav-core` and ical4j the server uses |
| `validation.rs`, `error.rs`, `ratelimit.rs`, `credentials.rs`, `logging.rs`, `progress.rs`, `cancellation.rs` | same-named Kotlin packages |

`ical/` replaces ical4j. It contains a content-line parser, a VEVENT/VALARM model, a generator, an RFC 5545 recurrence iterator with the same work bounds as the Kotlin expander (10,000 instances per series, plus a cap on periods examined), and VTIMEZONE generation from the IANA database via chrono-tz.

## Differences from the Kotlin build

The ported test suites pin the rest of the behavior to the Kotlin build. These differences are intended:

- Properties the model does not interpret (ORGANIZER, ATTENDEE, X-*, CLASS, GEO, COLOR, IMAGE, CONFERENCE and others) are written back exactly as read. The Kotlin build re-serialized some of them from a partial model.
- All VEVENT properties are written before the VALARM blocks, as RFC 5545 requires.
- RRULE `UNTIL` in UTC is compared as an instant. ical4j compared it against the series' local wall clock, which dropped the last occurrence for zones east of UTC.
- DTSTART is always the first occurrence of a series and counts toward COUNT, as RFC 5545 §3.3.10 requires. ical4j skipped it when the rule did not generate it (for example `FREQ=WEEKLY;BYDAY=MO` starting on a Tuesday).
- An EXDATE removes the occurrence at its exact time. It removes a whole day only for series with at most one occurrence per day, which is how the Kotlin build matched every EXDATE.
- VTIMEZONE blocks use the local wall-clock time of each transition and the real zone abbreviation (`JST`, not `TOKY`).
- A property whose value cannot be read is treated as absent instead of dropping the whole event.
- A JSON `null` argument counts as absent. An object or array where a string belongs returns `VALIDATION_ERROR` instead of `INTERNAL_ERROR`.
- `update_event` validates `start_date` and `end_date`.
- An update that still hits a stale ETag after its one retry returns `CONFLICT`, as the main README describes. The Kotlin build returned `INTERNAL_ERROR`.
- Datetimes without seconds (`2026-01-15T09:00`) work with a `timezone`, where the Kotlin build failed on them.
- The rate limiter belongs to each server instance rather than a process-wide global.

## Tests

```bash
cargo test                      # hermetic: mocks and wiremock, no network
TZ=Asia/Seoul cargo test        # same suite in a non-UTC zone
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

The live suite talks to real iCloud and is ignored by default:

```bash
ICLOUD_USERNAME=... ICLOUD_APP_PASSWORD=... \
  cargo test --test live_icloud -- --ignored --test-threads=1
```

Each live test creates its own events with an `MCP-LIVE:` prefix and deletes them afterwards, even when the test fails.
