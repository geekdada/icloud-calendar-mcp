//! iCloud credential loading from the environment.
//!
//! Credentials only ever come from environment variables, and no formatting of
//! them (Debug, Display, error messages) reveals the password or the full
//! username.

use std::fmt;

const ENV_ICLOUD_USERNAME: &str = "ICLOUD_USERNAME";
const ENV_ICLOUD_PASSWORD: &str = "ICLOUD_PASSWORD";

/// App-specific passwords are 19 characters (`xxxx-xxxx-xxxx-xxxx`); formats
/// vary, so only a minimum length is enforced.
const MIN_PASSWORD_LENGTH: usize = 16;

/// Apple ID and app-specific password.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl Credentials {
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self { username: username.into(), password: password.into() }
    }
}

/// Shows at most the first three characters of the username, never the password.
impl fmt::Display for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let username = if self.username.chars().count() > 3 {
            format!("{}***", self.username.chars().take(3).collect::<String>())
        } else {
            "***".to_string()
        };
        write!(f, "Credentials(username={username}, password=****)")
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// A configuration problem with the credentials. The message is safe to show:
/// it names the variable or rule, never the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialError(pub String);

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CredentialError {}

fn non_blank(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

/// Load credentials from the process environment.
///
/// `Ok(None)` when neither variable is set; an error when only one is set or
/// the values are malformed.
pub fn load_from_environment() -> Result<Option<Credentials>, CredentialError> {
    load_with(|name| std::env::var(name).ok())
}

/// [`load_from_environment`] over an arbitrary variable lookup, so tests need
/// not touch the process environment.
pub fn load_with(lookup: impl Fn(&str) -> Option<String>) -> Result<Option<Credentials>, CredentialError> {
    let username = non_blank(lookup(ENV_ICLOUD_USERNAME));
    let password = non_blank(lookup(ENV_ICLOUD_PASSWORD));
    let (username, password) = match (username, password) {
        (None, None) => return Ok(None),
        (None, Some(_)) => {
            return Err(CredentialError(format!("{ENV_ICLOUD_USERNAME} is not set")));
        }
        (Some(_), None) => {
            return Err(CredentialError(format!("{ENV_ICLOUD_PASSWORD} is not set")));
        }
        (Some(username), Some(password)) => (username, password),
    };
    if !username.contains('@') {
        return Err(CredentialError("Invalid username format: expected email address".into()));
    }
    // UTF-16 length, as the Kotlin build measured it.
    if password.encode_utf16().count() < MIN_PASSWORD_LENGTH {
        return Err(CredentialError("Password too short: expected app-specific password".into()));
    }
    Ok(Some(Credentials { username, password }))
}

/// True when both variables are set, without validating them.
pub fn is_configured() -> bool {
    is_configured_with(|name| std::env::var(name).ok())
}

/// [`is_configured`] over an arbitrary variable lookup.
pub fn is_configured_with(lookup: impl Fn(&str) -> Option<String>) -> bool {
    non_blank(lookup(ENV_ICLOUD_USERNAME)).is_some() && non_blank(lookup(ENV_ICLOUD_PASSWORD)).is_some()
}

/// Mask a credential for logging: the first `show_chars` characters then
/// `***`, or just `***` when the value is blank or not longer than `show_chars`.
pub fn mask(value: Option<&str>, show_chars: usize) -> String {
    match value {
        Some(v) if !v.trim().is_empty() && v.chars().count() > show_chars => {
            format!("{}***", v.chars().take(show_chars).collect::<String>())
        }
        _ => "***".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn mask_shows_first_three_chars_by_default() {
        assert_eq!(mask(Some("username@example.com"), 3), "use***");
        assert_eq!(mask(Some("username@example.com"), 5), "usern***");
    }

    #[test]
    fn mask_hides_missing_blank_and_short_values() {
        assert_eq!(mask(None, 3), "***");
        assert_eq!(mask(Some(""), 3), "***");
        assert_eq!(mask(Some("   "), 3), "***");
        assert_eq!(mask(Some("ab"), 3), "***");
        assert_eq!(mask(Some("abc"), 3), "***");
        assert_eq!(mask(Some("abcd"), 3), "abc***");
    }

    #[test]
    fn mask_handles_special_and_unicode_characters() {
        for value in ["abc\ndef", "abc\tdef", "abc\"def", "abc'def", "abc\\def"] {
            assert_eq!(mask(Some(value), 3), "abc***");
        }
        assert_eq!(mask(Some("user@例え.com"), 3), "use***");
    }

    #[test]
    fn formatting_masks_username_and_hides_password() {
        let creds = Credentials::new("user@icloud.com", "xxxx-xxxx-xxxx-xxxx");
        for shown in [creds.to_string(), format!("{creds:?}"), format!("{:?}", creds.clone())] {
            assert!(shown.contains("use***"));
            assert!(!shown.contains("user@icloud.com"));
            assert!(!shown.contains("xxxx-xxxx-xxxx-xxxx"));
            assert!(shown.contains("password=****"));
        }
        assert!(Credentials::new("ab", "xxxx").to_string().contains("username=***"));
    }

    #[test]
    fn equality_compares_all_fields() {
        let a = Credentials::new("user@icloud.com", "pass1");
        assert_ne!(a, Credentials::new("user@icloud.com", "pass2"));
        assert_eq!(a, Credentials::new("user@icloud.com", "pass1"));
    }

    #[test]
    fn load_returns_none_when_unset() {
        assert_eq!(load_with(env(&[])), Ok(None));
        assert_eq!(load_with(env(&[("ICLOUD_USERNAME", " "), ("ICLOUD_PASSWORD", "")])), Ok(None));
    }

    #[test]
    fn load_rejects_partial_configuration() {
        let err = load_with(env(&[("ICLOUD_PASSWORD", "abcd-efgh-ijkl-mnop")])).unwrap_err();
        assert_eq!(err.to_string(), "ICLOUD_USERNAME is not set");
        let err = load_with(env(&[("ICLOUD_USERNAME", "user@icloud.com")])).unwrap_err();
        assert_eq!(err.to_string(), "ICLOUD_PASSWORD is not set");
    }

    #[test]
    fn load_validates_format_without_echoing_values() {
        let err =
            load_with(env(&[("ICLOUD_USERNAME", "user"), ("ICLOUD_PASSWORD", "abcd-efgh-ijkl-mnop")])).unwrap_err();
        assert!(err.to_string().contains("expected email address"));
        let err =
            load_with(env(&[("ICLOUD_USERNAME", "user@icloud.com"), ("ICLOUD_PASSWORD", "short-secret")])).unwrap_err();
        assert!(err.to_string().contains("Password too short"));
        assert!(!err.to_string().contains("short-secret"));
    }

    #[test]
    fn load_accepts_valid_credentials() {
        let creds =
            load_with(env(&[("ICLOUD_USERNAME", "user@icloud.com"), ("ICLOUD_PASSWORD", "abcd-efgh-ijkl-mnop")]))
                .unwrap()
                .unwrap();
        assert_eq!(creds.username, "user@icloud.com");
        assert_eq!(creds.password, "abcd-efgh-ijkl-mnop");
    }

    #[test]
    fn is_configured_requires_both() {
        assert!(!is_configured_with(env(&[])));
        assert!(!is_configured_with(env(&[("ICLOUD_USERNAME", "u@x.com")])));
        assert!(is_configured_with(env(&[("ICLOUD_USERNAME", "u@x.com"), ("ICLOUD_PASSWORD", "p")])));
        // The process-env variant just has to answer.
        let _ = is_configured();
    }
}
