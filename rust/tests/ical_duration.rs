//! Port of icaldav-core `DurationUtilsTest` (and the duration cases of
//! `ICalAlarmTest`). `isDurationString` and `parseOrDefault` have no Rust
//! counterpart (callers use `parse_duration(..).is_some()` / `unwrap_or`).

use chrono::TimeDelta;
use icloud_calendar_mcp::ical::duration::{format_duration, parse_duration};

fn minutes(s: &str) -> i64 {
    parse_duration(s).unwrap_or_else(|| panic!("{s} should parse")).num_minutes()
}

#[test]
fn standard_formats() {
    assert_eq!(minutes("PT15M"), 15);
    assert_eq!(minutes("-PT15M"), -15);
    assert_eq!(minutes("PT1H30M"), 90);
    assert_eq!(minutes("-PT1H30M"), -90);
    assert_eq!(parse_duration("P1D").unwrap().num_days(), 1);
    assert_eq!(parse_duration("P1W").unwrap().num_days(), 7);
    let complex = parse_duration("P1DT2H30M").unwrap();
    assert_eq!(complex.num_days(), 1);
    assert_eq!(complex.num_hours(), 26);
    assert_eq!(complex.num_minutes() % 60, 30);
    let negative = parse_duration("-P1DT2H30M").unwrap();
    assert!(negative < TimeDelta::zero());
    assert_eq!(negative.num_days(), -1);
    assert_eq!(parse_duration("PT30S").unwrap().num_seconds(), 30);
    let full = parse_duration("P2DT3H45M30S").unwrap();
    assert_eq!(full, TimeDelta::days(2) + TimeDelta::hours(3) + TimeDelta::minutes(45) + TimeDelta::seconds(30));
}

#[test]
fn edge_cases() {
    assert_eq!(parse_duration(""), None);
    assert_eq!(parse_duration("   "), None);
    assert_eq!(minutes("  PT15M"), 15);
    assert_eq!(minutes("PT15M  "), 15);
    assert_eq!(minutes("pt15m"), 15);
    assert_eq!(minutes("Pt15M"), 15);
    assert_eq!(minutes("+PT15M"), 15);
    assert_eq!(parse_duration("PT0S"), Some(TimeDelta::zero()));
    assert_eq!(parse_duration("P365D").unwrap().num_days(), 365);
    assert_eq!(parse_duration("P4W").unwrap().num_days(), 28);
    // A bare "P" is lenient-parsed to zero, as the Kotlin reader allowed.
    assert_eq!(parse_duration("P"), Some(TimeDelta::zero()));
}

#[test]
fn invalid_input() {
    for bad in ["invalid", "T15M", "xyz123", "2024-01-01", "15", "15M"] {
        assert_eq!(parse_duration(bad), None, "{bad}");
    }
}

#[test]
fn formatting() {
    assert_eq!(format_duration(TimeDelta::minutes(15)), "PT15M");
    assert_eq!(format_duration(TimeDelta::minutes(-15)), "-PT15M");
    assert_eq!(format_duration(TimeDelta::minutes(90)), "PT1H30M");
    assert_eq!(format_duration(TimeDelta::days(1)), "P1D");
    assert_eq!(format_duration(TimeDelta::days(1) + TimeDelta::minutes(150)), "P1DT2H30M");
    assert_eq!(format_duration(TimeDelta::zero()), "PT0S");
    assert_eq!(format_duration(TimeDelta::seconds(30)), "PT30S");
    assert_eq!(
        format_duration(TimeDelta::days(2) + TimeDelta::hours(3) + TimeDelta::minutes(45) + TimeDelta::seconds(30)),
        "P2DT3H45M30S"
    );
    assert!(format_duration(TimeDelta::days(-1) - TimeDelta::hours(2)).starts_with("-P"));
}

#[test]
fn round_trips() {
    for original in ["PT15M", "-PT15M", "PT1H30M", "-PT1H30M", "P1D", "P1DT2H30M", "PT30S", "P2DT3H45M30S"] {
        let parsed = parse_duration(original).unwrap();
        assert_eq!(parse_duration(&format_duration(parsed)), Some(parsed), "{original}");
    }
    let original = TimeDelta::hours(2) + TimeDelta::minutes(45);
    assert_eq!(parse_duration(&format_duration(original)), Some(original));
}

#[test]
fn real_world_alarm_triggers() {
    for (input, expected) in [
        ("-PT15M", -15),
        ("-PT30M", -30),
        ("-PT1H", -60),
        ("-PT2H", -120),
        ("-P1D", -1440),
        ("-P1DT12H", -2160),
        ("-P1W", -10080),
        ("PT0M", 0),
        ("PT15M", 15),
    ] {
        assert_eq!(minutes(input), expected, "{input}");
    }
}

#[test]
fn concurrent_parse_and_format() {
    let handles: Vec<_> = (0..8)
        .map(|_| {
            std::thread::spawn(|| {
                for _ in 0..200 {
                    for input in ["PT15M", "-PT30M", "P1D", "PT1H30M", "-P1DT2H"] {
                        let parsed = parse_duration(input).unwrap();
                        assert!(format_duration(parsed).contains('P'));
                    }
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
}
