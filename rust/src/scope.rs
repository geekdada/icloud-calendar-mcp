//! The scope an `update_event`/`delete_event` acts on when the target is (or
//! could be) one occurrence of a recurring series.

/// Which occurrences of a recurring series a write affects.
///
/// The [`token`](EventScope::token) is the exact wire value accepted on the MCP
/// `scope` argument; this enum is the single source of truth for the allowed
/// values, reused by the validator, the service and the tool schemas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventScope {
    /// Only the referenced occurrence (a RECURRENCE-ID exception, or an EXDATE on delete).
    ThisOccurrence,
    /// The referenced occurrence and every later one.
    ThisAndFuture,
    /// The whole series.
    AllEvents,
}

impl EventScope {
    /// Every scope, in declaration order.
    pub const ALL: [EventScope; 3] = [
        EventScope::ThisOccurrence,
        EventScope::ThisAndFuture,
        EventScope::AllEvents,
    ];

    /// The wire tokens in declaration order, for schemas and error messages.
    pub const TOKENS: [&'static str; 3] = ["this_occurrence", "this_and_future", "all_events"];

    /// The wire token for this scope.
    pub fn token(self) -> &'static str {
        match self {
            EventScope::ThisOccurrence => "this_occurrence",
            EventScope::ThisAndFuture => "this_and_future",
            EventScope::AllEvents => "all_events",
        }
    }

    /// The scope for `token`, or `None` when it is absent or unrecognized.
    pub fn from_token(token: Option<&str>) -> Option<EventScope> {
        let token = token?;
        Self::ALL.into_iter().find(|scope| scope.token() == token)
    }

    /// True for the scopes that act on occurrences rather than the whole series.
    pub fn is_occurrence_level(self) -> bool {
        matches!(self, EventScope::ThisOccurrence | EventScope::ThisAndFuture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_round_trip() {
        for scope in EventScope::ALL {
            assert_eq!(EventScope::from_token(Some(scope.token())), Some(scope));
        }
        assert_eq!(EventScope::TOKENS.map(|t| EventScope::from_token(Some(t)).unwrap()), EventScope::ALL);
    }

    #[test]
    fn unknown_or_missing_token_is_none() {
        assert_eq!(EventScope::from_token(None), None);
        assert_eq!(EventScope::from_token(Some("ALL_EVENTS")), None);
        assert_eq!(EventScope::from_token(Some("")), None);
    }
}
