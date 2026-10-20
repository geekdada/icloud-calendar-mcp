//! Canonical, partition-independent iCloud CalDAV URLs.
//!
//! iCloud serves the same resource from regional partitions such as
//! `p180-caldav.icloud.com:443`. A reference (cache key or opaque event handle)
//! minted while talking to one partition must still match the same event
//! resolved through another, so the `p{N}-` prefix and any explicit port are
//! collapsed to `caldav.icloud.com`. Paths, queries and non-iCloud URLs are left
//! untouched, and the transform is idempotent.

use std::sync::LazyLock;

use regex::Regex;

const CANONICAL_HOST: &str = "caldav.icloud.com";

// Does not match the already-canonical host nor look-alikes on other domains.
static REGIONAL_HOST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)p\d+-caldav\.icloud\.com(:\d+)?").expect("valid regex"));

/// `url` with any regional iCloud host collapsed to the canonical host.
pub fn normalize_icloud_url(url: &str) -> String {
    REGIONAL_HOST.replace_all(url, CANONICAL_HOST).into_owned()
}

#[cfg(test)]
mod tests {
    use super::normalize_icloud_url as normalize;

    #[test]
    fn strips_regional_partition_prefix() {
        assert_eq!(
            normalize("https://p180-caldav.icloud.com/123/calendars/home/e.ics"),
            "https://caldav.icloud.com/123/calendars/home/e.ics"
        );
    }

    #[test]
    fn strips_regional_prefix_and_port() {
        assert_eq!(
            normalize("https://p42-caldav.icloud.com:443/x.ics"),
            "https://caldav.icloud.com/x.ics"
        );
    }

    #[test]
    fn is_case_insensitive() {
        assert_eq!(
            normalize("https://P42-CALDAV.ICLOUD.COM/x.ics"),
            "https://caldav.icloud.com/x.ics"
        );
    }

    #[test]
    fn is_idempotent() {
        let once = normalize("https://p1-caldav.icloud.com/x.ics");
        assert_eq!(normalize(&once), once);
    }

    #[test]
    fn leaves_canonical_and_foreign_urls_alone() {
        let canonical = "https://caldav.icloud.com/123/calendars/home/e.ics";
        assert_eq!(normalize(canonical), canonical);
        let other = "https://p180-caldav.example.com/x.ics";
        assert_eq!(normalize(other), other);
    }

    #[test]
    fn passes_through_empty_and_plain_paths() {
        assert_eq!(normalize(""), "");
        assert_eq!(
            normalize("/123/calendars/home/e.ics"),
            "/123/calendars/home/e.ics"
        );
    }
}
