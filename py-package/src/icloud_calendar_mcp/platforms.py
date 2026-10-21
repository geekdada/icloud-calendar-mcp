"""Map the running platform to its release asset and parse SHA256SUMS.

Pure functions, so they can be unit tested.
"""

import re

BINARY_BASENAME = "icloud-calendar-mcp"

# Linux builds are static musl binaries, so they run on glibc and musl distros alike.
TARGETS = {
    ("linux", "x86_64"): "x86_64-unknown-linux-musl",
    ("linux", "aarch64"): "aarch64-unknown-linux-musl",
    ("darwin", "x86_64"): "x86_64-apple-darwin",
    ("darwin", "aarch64"): "aarch64-apple-darwin",
    ("windows", "x86_64"): "x86_64-pc-windows-msvc",
}

# platform.machine() spells the same architecture differently per OS.
MACHINE_ALIASES = {
    "amd64": "x86_64",
    "x64": "x86_64",
    "arm64": "aarch64",
}

CHECKSUM_LINE = re.compile(r"^([0-9a-fA-F]{64}) [ *](.+)$")


def resolve_asset(system: str, machine: str) -> tuple[str, str] | None:
    """Return (asset name, installed binary name) for platform.system() and
    platform.machine() values, or None when no release binary exists."""
    system = system.lower()
    machine = machine.lower()
    target = TARGETS.get((system, MACHINE_ALIASES.get(machine, machine)))
    if target is None:
        return None
    extension = ".exe" if system == "windows" else ""
    return f"{BINARY_BASENAME}-{target}{extension}", f"{BINARY_BASENAME}{extension}"


def supported_platforms() -> list[str]:
    return [f"{system}-{machine}" for system, machine in TARGETS]


def parse_checksums(text: str) -> dict[str, str]:
    """Parse sha256sum output into {file name: lowercase hex digest}.

    Malformed lines are skipped."""
    checksums = {}
    for line in text.splitlines():
        match = CHECKSUM_LINE.match(line.strip())
        if match:
            checksums[match.group(2)] = match.group(1).lower()
    return checksums
