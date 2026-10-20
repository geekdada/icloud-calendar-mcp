//! Port of the Kotlin `InputValidatorTest` and `InputValidatorAlarmTest` suites,
//! plus the validator cases from `ReDoSSecurityTest` and `UnicodeSecurityTest`.

use icloud_calendar_mcp::scope::EventScope;
use icloud_calendar_mcp::validation::*;

fn err(result: ValidationResult) -> String {
    result.expect_err("expected an invalid result")
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| v.to_string()).collect()
}

// ── dates ───────────────────────────────────────────────────────────────────

#[test]
fn date_accepts_valid_and_leap_day() {
    assert!(validate_date(Some("2025-01-15"), "date").is_ok());
    assert!(validate_date(Some("2024-02-29"), "date").is_ok());
}

#[test]
fn date_requires_a_value() {
    assert_eq!(err(validate_date(None, "date")), "date is required");
    assert!(validate_date(Some(""), "date").is_err());
    assert_eq!(
        err(validate_date(None, "start_date")),
        "start_date is required"
    );
}

#[test]
fn date_rejects_wrong_formats() {
    for date in [
        "01-15-2025",
        "15/01/2025",
        "2025/01/15",
        "2025-1-15",
        "2025-01-5",
        "25-01-15",
        "2025-01-15T10:00",
    ] {
        let message = err(validate_date(Some(date), "start_date"));
        assert!(message.contains("YYYY-MM-DD"), "{date}: {message}");
    }
}

#[test]
fn date_rejects_impossible_dates() {
    assert_eq!(
        err(validate_date(Some("2025-02-30"), "date")),
        "date is not a valid date"
    );
    assert!(validate_date(Some("2025-02-29"), "date").is_err());
}

#[test]
fn date_rejects_non_ascii_digits() {
    assert!(
        validate_date(Some("٢٠٢٥-٠١-١٥"), "date")
            .unwrap_err()
            .contains("YYYY-MM-DD")
    );
}

// ── datetimes ───────────────────────────────────────────────────────────────

#[test]
fn datetime_accepts_naive_utc_and_offset_forms() {
    for value in [
        "2025-01-15T09:30:00",
        "2025-01-15T09:30",
        "2025-01-15T09:30:00Z",
        "2025-01-15T09:30Z",
        "2026-08-06T18:30:00+09:00",
        "2026-08-06T04:30:00-05:00",
        "2026-08-06T18:30+09:00",
    ] {
        assert!(
            validate_date_time(Some(value), "start_time").is_ok(),
            "{value}"
        );
    }
}

#[test]
fn datetime_requires_a_value() {
    assert_eq!(
        err(validate_date_time(None, "datetime")),
        "datetime is required"
    );
}

#[test]
fn datetime_rejects_wrong_formats() {
    for value in [
        "2025-01-15",
        "2025-01-15 09:30:00",
        "2025-01-15T9:30:00",
        "09:30:00",
        "2025-01-15T25:00:00",
        "2025-01-15T09:60:00",
        "2025-01-15T09:30:60",
        "2026-13-40T99:99",
        "2026-08-06T09:30:00+9",
        "2026-08-06T09:30:00+09:0",
        "2026-08-06T09:30:00+0900",
        "2026-08-06T09:30:00ZZ",
        "2026-08-06T09:30:00Z+09:00",
        "2026-08-06T09:30:00Zjunk",
        "2026-08-06T09:30:00+19:00",
    ] {
        assert!(
            validate_date_time(Some(value), "start_time").is_err(),
            "should reject {value}"
        );
    }
}

#[test]
fn datetime_error_lists_accepted_forms() {
    let message = err(validate_date_time(Some("nonsense"), "start_time"));
    assert!(message.contains('Z'));
    assert!(message.contains("+HH:MM") || message.contains("offset"));
    assert_eq!(
        err(validate_date_time(Some("2025-01-15T25:00:00"), "x")),
        "x is not a valid datetime"
    );
}

// ── calendar ids ────────────────────────────────────────────────────────────

#[test]
fn calendar_id_accepts_valid_ids() {
    for id in [
        "calendar-1",
        "cal_123",
        "https://caldav.icloud.com/123456/calendars/home/",
        "user@example.com",
        "CAL-001-ABC",
    ] {
        assert!(
            validate_calendar_id(Some(id), "calendar_id").is_ok(),
            "{id}"
        );
    }
}

#[test]
fn calendar_id_requires_a_bounded_value() {
    assert_eq!(
        err(validate_calendar_id(None, "calendar_id")),
        "calendar_id is required"
    );
    assert_eq!(
        err(validate_calendar_id(Some("  "), "event_id")),
        "event_id is required"
    );
    let message = err(validate_calendar_id(Some(&"a".repeat(501)), "calendar_id"));
    assert!(message.contains("too long"));
}

#[test]
fn calendar_id_rejects_injection() {
    for id in [
        "cal; DROP TABLE calendars;",
        "cal<script>alert(1)</script>",
        "cal\nX-Header: injection",
        "cal\r\nSet-Cookie: bad",
        "cal$(whoami)",
        "cal`id`",
        "cal|cat /etc/passwd",
        "../../../etc/passwd",
        "cal%00null",
    ] {
        let message = err(validate_calendar_id(Some(id), "calendar_id"));
        assert!(message.contains("invalid characters"), "{id:?}: {message}");
    }
}

#[test]
fn calendar_id_rejects_ssrf_and_dangerous_schemes() {
    for id in [
        "file:///etc/passwd",
        "javascript:alert(1)",
        "data:text/html",
        "http://localhost/cal",
        "http://127.0.0.1/cal",
        "http://169.254.169.254/latest",
        "http://10.0.0.1/cal",
        "http://192.168.1.1/cal",
        "http://172.16.0.1/cal",
        "http://172.31.255.1/cal",
        "ftp://example.com/cal",
    ] {
        assert!(
            validate_calendar_id(Some(id), "calendar_id").is_err(),
            "{id}"
        );
    }
}

#[test]
fn calendar_id_rejects_encoded_and_control_characters() {
    for id in [
        "p%61ssword=secret123",
        "p%2561ssword=secret123",
        "&#112;assword=secret123",
        "..%C0%AF..%C0%AFetc%C0%AFpasswd",
        "test%C0%80null",
        "calendar\u{0}null",
        "calendar\u{1B}escape",
        "calendar\u{7F}delete",
        "calendar-日本語",
    ] {
        assert!(
            validate_calendar_id(Some(id), "calendar_id").is_err(),
            "{id:?}"
        );
    }
}

#[test]
fn calendar_id_handles_pathological_input_quickly() {
    assert!(validate_calendar_id(Some(&format!("{}!", "a".repeat(30))), "calendar_id").is_err());
    let long_url = format!("http://{}.{}.com", "a".repeat(100), "b".repeat(100));
    let _ = validate_calendar_id(Some(&long_url), "calendar_id");
    assert!(validate_calendar_id(Some(&"x".repeat(1_000_000)), "calendar_id").is_err());
    assert!(validate_calendar_id(Some(&"<>\"';&|`$(){}[]".repeat(10_000)), "calendar_id").is_err());
}

// ── titles and free text ────────────────────────────────────────────────────

#[test]
fn title_rules() {
    assert!(validate_title(Some("Team Meeting"), "title").is_ok());
    assert!(validate_title(Some("Meeting: Q1 Review & Planning!"), "title").is_ok());
    assert!(validate_title(Some(&"a".repeat(25)), "title").is_ok());
    assert_eq!(err(validate_title(None, "title")), "title is required");
    assert!(validate_title(Some("   "), "title").is_err());
    let message = err(validate_title(Some(&"a".repeat(501)), "title"));
    assert_eq!(message, "title is too long (max 500 characters)");
}

#[test]
fn title_length_counts_utf16_units_like_kotlin() {
    // 250 emoji are 500 UTF-16 units: at the cap. One more is over it.
    assert!(validate_title(Some(&"📅".repeat(250)), "title").is_ok());
    assert!(validate_title(Some(&"📅".repeat(251)), "title").is_err());
}

#[test]
fn optional_text_rules() {
    assert!(validate_optional_text(None, "description", MAX_DESCRIPTION_LENGTH).is_ok());
    assert!(validate_optional_text(Some(""), "description", MAX_DESCRIPTION_LENGTH).is_ok());
    assert!(
        validate_optional_text(
            Some("Meeting notes here"),
            "description",
            MAX_DESCRIPTION_LENGTH
        )
        .is_ok()
    );
    assert!(
        validate_optional_text(
            Some(&"a".repeat(5001)),
            "description",
            MAX_DESCRIPTION_LENGTH
        )
        .is_err()
    );
    let message = err(validate_optional_text(Some("abcdef"), "location", 5));
    assert_eq!(message, "location is too long (max 5 characters)");
    assert!(
        validate_optional_text(
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaab"),
            "test",
            MAX_DESCRIPTION_LENGTH
        )
        .is_ok()
    );
}

// ── time ranges ─────────────────────────────────────────────────────────────

#[test]
fn time_range_basic_ordering() {
    assert!(validate_time_range("2025-01-15T09:00:00", "2025-01-15T10:00:00", None, None).is_ok());
    assert!(validate_time_range("2025-01-15T09:00:00", "2025-01-17T10:00:00", None, None).is_ok());
    let message = err(validate_time_range(
        "2025-01-15T10:00:00",
        "2025-01-15T09:00:00",
        None,
        None,
    ));
    assert_eq!(message, "End time must be after start time");
    let message = err(validate_time_range(
        "2025-01-15T09:00:00",
        "2025-01-15T09:00:00",
        None,
        None,
    ));
    assert_eq!(message, "End time must be different from start time");
    assert!(
        validate_time_range("2025-01-15T09:00:00Z", "2025-01-15T10:00:00Z", None, None).is_ok()
    );
}

#[test]
fn time_range_compares_offsets_as_instants() {
    assert!(
        validate_time_range(
            "2026-08-06T18:30:00+09:00",
            "2026-08-06T10:00:00Z",
            None,
            None
        )
        .is_ok()
    );
    assert!(
        validate_time_range(
            "2026-08-06T18:30:00+09:00",
            "2026-08-06T09:00:00Z",
            None,
            None
        )
        .is_err()
    );
    assert!(
        validate_time_range(
            "2026-08-06T09:00:00",
            "2026-08-06T18:30:00+09:00",
            None,
            None
        )
        .is_ok()
    );
}

#[test]
fn time_range_anchors_naive_values_to_the_timezone() {
    let la = Some("America/Los_Angeles");
    assert!(validate_time_range("2026-08-06T09:00:00", "2026-08-06T15:30:00Z", la, None).is_err());
    assert!(validate_time_range("2026-08-06T09:00:00", "2026-08-06T17:00:00Z", la, None).is_ok());
    // The end falls back to `timezone` unless `end_timezone` is given.
    assert!(
        validate_time_range(
            "2026-08-06T09:00:00",
            "2026-08-06T10:00:00",
            Some("America/New_York"),
            Some("America/Los_Angeles")
        )
        .is_ok()
    );
    assert!(
        validate_time_range(
            "2026-08-06T09:00:00",
            "2026-08-06T05:00:00",
            Some("America/New_York"),
            Some("America/Los_Angeles")
        )
        .is_err()
    );
}

#[test]
fn time_range_rejects_unparseable_input() {
    assert_eq!(
        err(validate_time_range(
            "garbage",
            "2026-08-06T07:00:00",
            None,
            None
        )),
        "Invalid time format"
    );
    assert_eq!(
        err(validate_time_range(
            "2026-08-06T07:00:00",
            "2026-08-06T08:00:00",
            Some("Not/AZone"),
            None
        )),
        "Invalid time format"
    );
}

// ── date spans ──────────────────────────────────────────────────────────────

#[test]
fn date_span_limits() {
    assert!(validate_date_span(Some("2026-01-01"), Some("2027-01-01")).is_ok());
    assert!(validate_date_span(Some("2024-01-01"), Some("2025-01-01")).is_ok());
    assert!(validate_date_span(Some("2026-05-10"), Some("2026-05-10")).is_ok());
    let message = err(validate_date_span(Some("2024-01-01"), Some("2025-01-02")));
    assert!(
        message.contains("366") && message.to_lowercase().contains("narrow"),
        "{message}"
    );
    let message = err(validate_date_span(Some("2026-05-10"), Some("2026-05-09")));
    assert!(
        message.contains("end_date") && message.to_lowercase().contains("precede"),
        "{message}"
    );
}

#[test]
fn date_span_defers_malformed_dates() {
    assert!(validate_date_span(Some("not-a-date"), Some("2026-05-10")).is_ok());
    assert!(validate_date_span(Some("2026-05-10"), Some("2026-13-40")).is_ok());
    assert!(validate_date_span(None, Some("2026-05-10")).is_ok());
    assert!(validate_date_span(Some("2026-05-10"), None).is_ok());
}

// ── ICS sanitization ────────────────────────────────────────────────────────

#[test]
fn sanitize_for_ics_only_collapses_line_breaks() {
    assert_eq!(sanitize_for_ics("path\\to\\file"), "path\\to\\file");
    assert_eq!(sanitize_for_ics("item1;item2"), "item1;item2");
    assert_eq!(sanitize_for_ics("item1,item2"), "item1,item2");
    assert_eq!(sanitize_for_ics("line1\nline2"), "line1 line2");
    assert_eq!(sanitize_for_ics("line1\r\nline2"), "line1 line2");
    assert_eq!(sanitize_for_ics("line1\rline2"), "line1 line2");
}

#[test]
fn sanitize_for_ics_blocks_property_injection() {
    let out = sanitize_for_ics("Meeting\nATTENDEE:mailto:evil@hacker.com");
    assert_eq!(out, "Meeting ATTENDEE:mailto:evil@hacker.com");
    let out =
        sanitize_for_ics("Title;ORGANIZER:mailto:evil@hacker.com\r\nATTENDEE:innocent@user.com");
    assert!(!out.contains('\n') && !out.contains('\r') && out.contains(';'));
    let out = sanitize_for_ics("Meeting: 会議\nLocation: 東京\nNotes: 备注");
    assert!(!out.contains('\n') && out.contains("会議") && out.contains("東京"));
    let pathological = format!("{};{}", "\r\n".repeat(100_000), ",".repeat(100_000));
    assert!(!sanitize_for_ics(&pathological).contains('\r'));
}

// ── collect_errors ──────────────────────────────────────────────────────────

#[test]
fn collect_errors_keeps_order() {
    assert!(collect_errors(&[Ok(()), Ok(())]).is_empty());
    let errors = collect_errors(&[Ok(()), Err("Error 1".into()), Ok(()), Err("Error 2".into())]);
    assert_eq!(errors, vec!["Error 1", "Error 2"]);
    assert_eq!(
        collect_errors(&[Err("E1".into()), Err("E2".into()), Err("E3".into())]).len(),
        3
    );
}

// ── timezones ───────────────────────────────────────────────────────────────

#[test]
fn timezone_rules() {
    assert!(validate_timezone(None, "timezone").is_ok());
    assert!(validate_timezone(Some(""), "timezone").is_ok());
    assert!(validate_timezone(Some("America/New_York"), "timezone").is_ok());
    assert!(validate_timezone(Some("UTC"), "timezone").is_ok());
    assert_eq!(
        err(validate_timezone(Some("Not/AZone"), "end_timezone")),
        "end_timezone is not a recognized IANA timezone"
    );
    assert_eq!(
        err(validate_timezone(Some(&"A".repeat(101)), "timezone")),
        "timezone is too long"
    );
}

// ── recurrence date lists ───────────────────────────────────────────────────

#[test]
fn recurrence_dates_accept_dates_and_utc_or_naive_datetimes() {
    let values = strings(&[
        "2026-08-18",
        "2026-08-18T10:00:00Z",
        "2026-08-18T10:00:00",
        "2026-08-18T10:00",
    ]);
    assert!(validate_recurrence_date_list(Some(&values), "rdates").is_ok());
    assert!(validate_recurrence_date_list(None, "rdates").is_ok());
    assert!(validate_recurrence_date_list(Some(&[]), "rdates").is_ok());
}

#[test]
fn recurrence_dates_reject_offsets_garbage_and_oversize_lists() {
    let message = err(validate_recurrence_date_list(
        Some(&strings(&["2026-08-18T10:00:00+09:00"])),
        "exdates",
    ));
    assert_eq!(
        message,
        "exdates[0] must be a date (YYYY-MM-DD) or a UTC datetime (…Z), not an offset datetime"
    );
    assert!(
        validate_recurrence_date_list(Some(&strings(&["2026-08-18", "tomorrow"])), "rdates")
            .unwrap_err()
            .starts_with("rdates[1]")
    );
    let many = vec!["2026-08-18".to_string(); 367];
    assert_eq!(
        err(validate_recurrence_date_list(Some(&many), "rdates")),
        "rdates has too many entries (max 366)"
    );
}

// ── scope ───────────────────────────────────────────────────────────────────

#[test]
fn scope_accepts_allowed_values_and_omission() {
    for token in EventScope::TOKENS {
        assert!(validate_scope(Some(token), "scope").is_ok());
    }
    assert!(validate_scope(None, "scope").is_ok());
    assert!(validate_scope(Some(""), "scope").is_ok());
    assert!(validate_scope(Some("   "), "scope").is_ok());
}

#[test]
fn scope_rejects_other_values_naming_the_allowed_ones() {
    let message = err(validate_scope(Some("everything"), "scope"));
    assert_eq!(
        message,
        "scope must be one of this_occurrence, this_and_future, all_events"
    );
}

#[test]
fn occurrence_scope_rejects_series_fields() {
    let message = err(validate_occurrence_scope_fields(
        Some("this_occurrence"),
        Some("FREQ=WEEKLY"),
        None,
        None,
        "scope",
    ));
    assert!(message.contains("rrule"), "{message}");
    let dates = strings(&["2026-08-18"]);
    assert!(
        validate_occurrence_scope_fields(
            Some("this_and_future"),
            None,
            Some(&dates),
            None,
            "scope"
        )
        .is_err()
    );
    assert!(
        validate_occurrence_scope_fields(
            Some("this_and_future"),
            None,
            None,
            Some(&dates),
            "scope"
        )
        .is_err()
    );
    let message = err(validate_occurrence_scope_fields(
        Some("this_and_future"),
        Some("FREQ=DAILY"),
        Some(&dates),
        Some(&dates),
        "scope",
    ));
    assert_eq!(
        message,
        "rrule, rdates, exdates cannot be set with scope=this_and_future; series-level fields belong to the whole series (use scope=all_events)"
    );
}

#[test]
fn occurrence_scope_allows_series_fields_elsewhere() {
    let dates = strings(&["2026-08-18"]);
    assert!(
        validate_occurrence_scope_fields(
            Some("all_events"),
            Some("FREQ=WEEKLY"),
            Some(&dates),
            None,
            "scope"
        )
        .is_ok()
    );
    assert!(
        validate_occurrence_scope_fields(Some("this_occurrence"), None, None, None, "scope")
            .is_ok()
    );
    assert!(
        validate_occurrence_scope_fields(None, Some("FREQ=WEEKLY"), None, None, "scope").is_ok()
    );
    // Blank rrule and empty lists are not offenders.
    assert!(
        validate_occurrence_scope_fields(
            Some("this_occurrence"),
            Some(" "),
            Some(&[]),
            Some(&[]),
            "scope"
        )
        .is_ok()
    );
}

// ── alarms ──────────────────────────────────────────────────────────────────

fn alarm(trigger: &str) -> AlarmInput {
    AlarmInput {
        trigger: Some(trigger.to_string()),
        ..AlarmInput::default()
    }
}

#[test]
fn alarms_accept_mixed_actions_within_bounds() {
    let alarms = vec![
        alarm("-PT15M"),
        AlarmInput {
            action: Some("AUDIO".into()),
            ..alarm("-P1D")
        },
        AlarmInput {
            action: Some("EMAIL".into()),
            summary: Some("Reminder subject".into()),
            description: Some("Body".into()),
            ..alarm("20260115T093000Z")
        },
        AlarmInput {
            repeat_count: Some(3),
            repeat_duration: Some("PT5M".into()),
            ..alarm("-PT5M")
        },
    ];
    assert!(validate_alarm_list(Some(&alarms), "alarms").is_ok());
    assert!(validate_alarm_list(None, "alarms").is_ok());
    assert!(validate_alarm_list(Some(&[]), "alarms").is_ok());
}

#[test]
fn alarms_reject_more_than_eight() {
    let alarms: Vec<_> = (1..=9).map(|i| alarm(&format!("-PT{i}M"))).collect();
    assert!(err(validate_alarm_list(Some(&alarms), "alarms")).contains("max 8"));
}

#[test]
fn alarms_reject_unknown_action() {
    let alarms = vec![AlarmInput {
        action: Some("PROCEDURE".into()),
        ..alarm("-PT15M")
    }];
    assert!(err(validate_alarm_list(Some(&alarms), "alarms")).contains("DISPLAY/AUDIO/EMAIL"));
}

#[test]
fn alarms_reject_malformed_trigger() {
    let message = err(validate_alarm_list(Some(&[alarm("yesterday")]), "alarms"));
    assert!(message.contains("RFC 5545 duration") || message.contains("absolute UTC instant"));
}

#[test]
fn alarms_reject_missing_trigger() {
    let alarms = vec![AlarmInput {
        action: Some("DISPLAY".into()),
        ..AlarmInput::default()
    }];
    assert_eq!(
        err(validate_alarm_list(Some(&alarms), "alarms")),
        "alarms[0].trigger is required"
    );
}

#[test]
fn alarms_reject_bad_text_repeat_and_duration() {
    let long = "x".repeat(501);
    let cases = [
        (
            AlarmInput {
                description: Some(long.clone()),
                ..alarm("-PT1M")
            },
            "alarms[0].description exceeds 500 characters",
        ),
        (
            AlarmInput {
                summary: Some(long),
                ..alarm("-PT1M")
            },
            "alarms[0].summary exceeds 500 characters",
        ),
        (
            AlarmInput {
                repeat_count: Some(-1),
                ..alarm("-PT1M")
            },
            "alarms[0].repeat_count must be >= 0",
        ),
        (
            AlarmInput {
                repeat_duration: Some("5 minutes".into()),
                ..alarm("-PT1M")
            },
            "alarms[0].repeat_duration must be an RFC 5545 duration",
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(err(validate_alarm_list(Some(&[input]), "alarms")), expected);
    }
}

#[test]
fn alarm_trigger_forms() {
    for trigger in [
        "-PT15M",
        "+P1D",
        "PT0S",
        "P1W",
        "-P1DT2H",
        "PT1H30M",
        "20260115T093000Z",
    ] {
        assert!(
            validate_alarm_list(Some(&[alarm(trigger)]), "alarms").is_ok(),
            "{trigger}"
        );
    }
    for trigger in [
        "P1Y",
        "-15M",
        "20260115T093000",
        "2026-01-15T09:30:00Z",
        "P1W2D",
    ] {
        assert!(
            validate_alarm_list(Some(&[alarm(trigger)]), "alarms").is_err(),
            "{trigger}"
        );
    }
}
