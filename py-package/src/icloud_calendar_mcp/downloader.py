"""Download the native server binary from GitHub Releases on first run.

The binary is verified against the SHA256SUMS file bundled into the package
at publish time. Everything is logged to stderr: stdout is the MCP JSON-RPC
channel.
"""

import hashlib
import os
import platform
import stat
import sys
from importlib import resources
from pathlib import Path

import httpx

from icloud_calendar_mcp import __version__
from icloud_calendar_mcp.platforms import parse_checksums, resolve_asset, supported_platforms

RELEASE_URL = f"https://github.com/icloud-calendar-mcp/icloud-calendar-mcp/releases/download/v{__version__}"
DOWNLOAD_TIMEOUT_SECONDS = 300.0


class InstallError(Exception):
    """The binary could not be located, downloaded or verified."""


def _log(message: str) -> None:
    print(message, file=sys.stderr)


def platform_asset() -> tuple[str, str]:
    resolved = resolve_asset(platform.system(), platform.machine())
    if resolved is None:
        raise InstallError(
            f"No prebuilt binary for {platform.system()}-{platform.machine()}. "
            f"Supported: {', '.join(supported_platforms())}. "
            "Build from source instead: "
            "https://github.com/icloud-calendar-mcp/icloud-calendar-mcp#option-4-build-from-source"
        )
    return resolved


def binary_path(binary_name: str) -> Path:
    """The cache path includes the version, so an upgrade downloads its own binary."""
    return Path.home() / ".cache" / "icloud-calendar-mcp" / __version__ / binary_name


def expected_checksum(asset: str) -> str:
    checksums_file = resources.files("icloud_calendar_mcp") / "SHA256SUMS"
    if not checksums_file.is_file():
        raise InstallError("SHA256SUMS is missing; this package was not built by the release workflow.")
    checksum = parse_checksums(checksums_file.read_text()).get(asset)
    if checksum is None:
        raise InstallError(f"SHA256SUMS has no entry for {asset}.")
    return checksum


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as file:
        for chunk in iter(lambda: file.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def download(url: str, dest: Path) -> None:
    try:
        with httpx.Client(follow_redirects=True, timeout=DOWNLOAD_TIMEOUT_SECONDS) as client:
            with client.stream("GET", url) as response:
                if response.status_code != 200:
                    raise InstallError(f"Download failed: HTTP {response.status_code} for {url}")
                with open(dest, "wb") as file:
                    for chunk in response.iter_bytes(chunk_size=1 << 16):
                        file.write(chunk)
    except httpx.HTTPError as error:
        raise InstallError(f"Download failed: {error}") from error


def install(asset: str, checksum: str, target: Path) -> None:
    url = f"{RELEASE_URL}/{asset}"
    partial = target.with_name(target.name + ".download")
    target.parent.mkdir(parents=True, exist_ok=True)
    _log(f"Downloading iCloud Calendar MCP server: {url}")
    try:
        download(url, partial)
        actual = sha256_file(partial)
        if actual != checksum:
            raise InstallError(f"Checksum mismatch for {asset}: expected {checksum}, got {actual}")
        partial.chmod(partial.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
        os.replace(partial, target)
    finally:
        partial.unlink(missing_ok=True)
    _log("Download complete, checksum verified.")


def ensure_binary() -> Path:
    """Return the path of a verified binary, downloading it first if needed."""
    asset, binary_name = platform_asset()
    checksum = expected_checksum(asset)
    target = binary_path(binary_name)
    if target.is_file() and sha256_file(target) == checksum:
        return target
    install(asset, checksum, target)
    return target
