# Contributing to iCloud Calendar MCP Server

Thank you for your interest in contributing! This document provides guidelines for contributing to the project.

## Code of Conduct

Be respectful and constructive in all interactions. We welcome contributors of all backgrounds and experience levels.

## Getting Started

### Prerequisites

- [Rust](https://rustup.rs) 1.85 or newer (CI uses the latest stable)
- Git
- Node.js 18+ and [uv](https://docs.astral.sh/uv/), only to work on the npm or PyPI wrappers

### Development Setup

1. Fork and clone the repository:
   ```bash
   git clone https://github.com/icloud-calendar-mcp/icloud-calendar-mcp.git
   cd icloud-calendar-mcp
   ```

2. Build the project:
   ```bash
   cd rust
   cargo build
   ```

3. Run tests:
   ```bash
   cargo test
   ```

4. Run the live iCloud tests (optional). They need real credentials, and each test deletes the events it creates:
   ```bash
   ICLOUD_USERNAME=... ICLOUD_PASSWORD=... cargo test --test live_icloud -- --ignored --test-threads=1
   ```

## Making Changes

### Branch Naming

- `feature/description` - New features
- `fix/description` - Bug fixes
- `security/description` - Security improvements
- `docs/description` - Documentation updates

### Code Style

We follow standard Rust conventions:

- Format with `cargo fmt` (settings in `rust/rustfmt.toml`)
- Keep `cargo clippy --all-targets -- -D warnings` clean
- Use meaningful variable and function names
- Document public APIs with `///` doc comments

### Testing Requirements

All contributions must include tests:

- **New features**: Add unit tests and integration tests
- **Bug fixes**: Add a test that reproduces the bug
- **Security fixes**: Add adversarial tests

Run the full test suite before submitting:
```bash
cd rust
cargo test
```

### Security Considerations

This is a security-sensitive project. Please ensure:

1. **No credential exposure** - Never log or expose credentials
2. **Input validation** - Validate all user inputs
3. **Error handling** - Don't expose sensitive data in errors
4. **Injection prevention** - Properly escape all user data
5. **ReDoS prevention** - Test regex patterns for catastrophic backtracking
6. **SSRF prevention** - Validate URLs and block internal networks

See [SECURITY.md](SECURITY.md) for security guidelines.

## Test Categories

The Rust crate has 1,387 tests that run without network access, plus 13 live iCloud tests that are ignored unless run explicitly. Integration and security suites live in `rust/tests/`, one file per suite; unit tests sit next to the code in `rust/src/`.

### Security Tests (158 tests)

| Suite | Tests | Description |
|-------|-------|-------------|
| `security_adversarial` | 41 | Secrets in errors, SQL/XSS/command/prompt payloads as data, path traversal, oversized input |
| `ics_patcher_adversarial` | 28 | CRLF injection, property injection, encoding attacks |
| `security_owasp_mcp` | 25 | OWASP MCP Top 10 attack vectors |
| `ical_recurrence_adversarial` | 24 | Hostile recurrence rules |
| `ical_recurrence_resource_exhaustion` | 16 | Expansion caps and work bounds |
| `ics_redos_unicode` | 13 | ReDoS patterns, Unicode round trips (bidi, zero-width, combining marks) |
| `ical_parser_adversarial` | 11 | Malformed and hostile calendar data |

Unit tests in `logging.rs`, `progress.rs`, `cancellation.rs`, `credentials.rs` and `error.rs` cover log injection, token handling, replay and credential masking.

### Functional Tests

| Category | Tests | Description |
|----------|-------|-------------|
| Unit tests (`rust/src/`) | 291 | XML parsing, ETag normalization, error handling, logging, rate limiting, cancellation, progress |
| iCalendar (`ical_*`) | 223 | Parsing, generation, VTIMEZONE, alarms, durations, RFC samples |
| Recurrence (`ical_recurrence_*`) | 202 | RRULE, RDATE and EXDATE expansion, overrides, leap years |
| ICS Format (`ics_*`) | 163 | Parsing, building, patching, per-occurrence expansion, RFC 5545 audit |
| MCP Server (`mcp_*`, `smoke_server`) | 119 | Tool integration, MCP spec compliance, prompts, structured content, annotations |
| Service Layer (`service_*`) | 113 | Calendar operations, event handles, caching, caps |
| CalDAV Protocol (`caldav_*`) | 77 | HTTP client against a mock server, models |
| Input Validation | 41 | All parameter validation rules (incl. alarms) |
| Live iCloud | 13 | Real iCloud round trips (ignored by default) |

## Pull Request Process

### Before Submitting

1. Run all tests: `cargo test` (in `rust/`)
2. Check formatting and lints: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`
3. Update documentation if needed
4. Add yourself to contributors (optional)

### PR Requirements

- Clear description of changes
- Reference any related issues
- All tests passing
- No decrease in test coverage
- Security review for security-related changes

### Review Process

1. Submit PR against `main` branch
2. Automated tests run via GitHub Actions
3. Maintainer review within 3-5 business days
4. Address feedback and update PR
5. Squash and merge when approved

## Types of Contributions

### Bug Reports

Open an issue with:
- Clear description
- Steps to reproduce
- Expected vs actual behavior
- Environment details (OS, server version, install method)

### Feature Requests

Open an issue with:
- Use case description
- Proposed solution (optional)
- Alternatives considered (optional)

### Security Vulnerabilities

**Do not open public issues for security vulnerabilities.**

See [SECURITY.md](SECURITY.md) for responsible disclosure process.

### Documentation

- Fix typos and improve clarity
- Add examples and use cases
- Translate documentation

## Development Tips

### Running Individual Tests

```bash
cd rust

# One test file
cargo test --test security_adversarial

# Tests with a string in their name
cargo test --test security_adversarial mcp01

# Tests matching a name anywhere in the crate
cargo test caldav

# The suite in a non-UTC zone (catches timezone regressions)
TZ=Asia/Seoul cargo test
```

### Building a Release Binary

```bash
cd rust
cargo build --release
# Output: target/release/icloud-calendar-mcp
```

### Testing with MCP Inspector

```bash
ICLOUD_USERNAME="test@icloud.com" \
ICLOUD_PASSWORD="test-password" \
npx @mcp-use/inspector ./target/release/icloud-calendar-mcp
```

### Adding Security Tests

When adding security tests, follow this pattern:

```rust
#[test]
fn attack_vector_is_mitigated() {
    let malicious_input = "..."; // Attack payload

    let result = within(Duration::from_secs(2), || system_under_test.process(malicious_input));

    // Assert attack was blocked or sanitized
    assert!(!result.contains(sensitive_data));
}
```

Key considerations:
- Bound the running time (see the `within` and `assert_fast` helpers in the suites) so a regression fails instead of hanging
- Test both blocking and sanitization behaviors
- Document the attack vector being tested
- Include edge cases (unicode, encoding, etc.)

## Questions?

- Open a GitHub Discussion for general questions
- Open an Issue for bugs or feature requests
- See [SECURITY.md](SECURITY.md) for security concerns

Thank you for contributing!
