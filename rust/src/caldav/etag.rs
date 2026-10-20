//! ETag normalization per RFC 7232.
//!
//! CalDAV servers emit ETags quoted, weak (`W/"…"`), unquoted, or XML-entity
//! encoded when read out of a multistatus body. Normalizing them to the bare
//! opaque value keeps cache keys and handles stable regardless of the source.

/// Strip an ETag down to its opaque value: decode `&quot;`, drop a `W/` prefix
/// and surrounding quotes. Returns `None` for absent or empty input.
pub fn normalize_etag(etag: Option<&str>) -> Option<String> {
    let decoded = etag?.trim().replace("&quot;", "\"");
    let mut result = decoded.as_str();
    if let Some(rest) = result.strip_prefix("W/") {
        result = rest;
    }
    if result.len() >= 2 && result.starts_with('"') && result.ends_with('"') {
        result = &result[1..result.len() - 1];
    }
    (!result.is_empty()).then(|| result.to_string())
}

#[cfg(test)]
mod tests {
    use super::normalize_etag;

    fn norm(s: &str) -> Option<String> {
        normalize_etag(Some(s))
    }

    #[test]
    fn null_and_blank_input_return_none() {
        assert_eq!(normalize_etag(None), None);
        assert_eq!(norm(""), None);
        assert_eq!(norm("   "), None);
    }

    #[test]
    fn plain_etag_unchanged() {
        assert_eq!(norm("abc123").as_deref(), Some("abc123"));
    }

    #[test]
    fn quoted_etag_strips_quotes() {
        assert_eq!(norm("\"abc123\"").as_deref(), Some("abc123"));
    }

    #[test]
    fn weak_etag_strips_prefix_and_quotes() {
        assert_eq!(norm("W/\"abc123\"").as_deref(), Some("abc123"));
    }

    #[test]
    fn xml_entity_quotes_decoded_and_stripped() {
        assert_eq!(norm("&quot;abc123&quot;").as_deref(), Some("abc123"));
        assert_eq!(norm("W/&quot;abc&quot;").as_deref(), Some("abc"));
    }

    #[test]
    fn whitespace_trimmed() {
        assert_eq!(norm("  \"abc\"  ").as_deref(), Some("abc"));
    }

    #[test]
    fn single_character() {
        assert_eq!(norm("\"a\"").as_deref(), Some("a"));
    }

    #[test]
    fn empty_quotes_return_none() {
        assert_eq!(norm("\"\""), None);
    }

    #[test]
    fn icloud_style_long_etag() {
        assert_eq!(
            norm("\"C=123@U=456-789\"").as_deref(),
            Some("C=123@U=456-789")
        );
    }

    #[test]
    fn weak_etag_without_quotes() {
        assert_eq!(norm("W/abc").as_deref(), Some("abc"));
    }

    #[test]
    fn lone_quote_is_kept() {
        assert_eq!(norm("\"").as_deref(), Some("\""));
    }
}
