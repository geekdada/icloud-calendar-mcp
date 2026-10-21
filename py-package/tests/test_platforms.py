import unittest

from icloud_calendar_mcp.platforms import parse_checksums, resolve_asset, supported_platforms


class ResolveAssetTest(unittest.TestCase):
    def test_maps_every_supported_platform(self):
        self.assertEqual(
            resolve_asset("Linux", "x86_64"),
            ("icloud-calendar-mcp-x86_64-unknown-linux-musl", "icloud-calendar-mcp"),
        )
        self.assertEqual(resolve_asset("Linux", "aarch64")[0], "icloud-calendar-mcp-aarch64-unknown-linux-musl")
        self.assertEqual(resolve_asset("Darwin", "x86_64")[0], "icloud-calendar-mcp-x86_64-apple-darwin")
        self.assertEqual(resolve_asset("Darwin", "arm64")[0], "icloud-calendar-mcp-aarch64-apple-darwin")
        self.assertEqual(
            resolve_asset("Windows", "AMD64"),
            ("icloud-calendar-mcp-x86_64-pc-windows-msvc.exe", "icloud-calendar-mcp.exe"),
        )
        self.assertEqual(len(supported_platforms()), 5)

    def test_returns_none_without_a_release_binary(self):
        self.assertIsNone(resolve_asset("Windows", "ARM64"))
        self.assertIsNone(resolve_asset("FreeBSD", "amd64"))
        self.assertIsNone(resolve_asset("Linux", "i686"))


class ParseChecksumsTest(unittest.TestCase):
    def test_parses_text_and_binary_mode_lines(self):
        text = f"{'A' * 64}  icloud-calendar-mcp-x86_64-apple-darwin\r\n{'b' * 64} *icloud-calendar-mcp.exe\n"
        self.assertEqual(
            parse_checksums(text),
            {"icloud-calendar-mcp-x86_64-apple-darwin": "a" * 64, "icloud-calendar-mcp.exe": "b" * 64},
        )

    def test_skips_malformed_lines(self):
        self.assertEqual(parse_checksums(f"not-a-hash  file\n{'c' * 63}  short\n\n"), {})


if __name__ == "__main__":
    unittest.main()
