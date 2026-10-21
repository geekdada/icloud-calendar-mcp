//! Port of icaldav-core `ICalParserAppleHolidaysFeedTest`: Apple's holiday feed
//! stamps every VEVENT with `DTSTAMP;VALUE=DATE:19760401`.
//!
//! The fixture is read from the vendored Kotlin test resources.

use icloud_calendar_mcp::ical::parser::parse_all_events;

const FEED: &str = include_str!("fixtures/apple_us_holidays.ics");

#[test]
fn every_vevent_in_the_feed_parses() {
    let expected = FEED.matches("BEGIN:VEVENT").count();
    assert!(expected > 0, "fixture has no VEVENTs");
    assert_eq!(parse_all_events(FEED).unwrap().len(), expected);
}

#[test]
fn date_valued_dtstamp_keeps_the_publisher_date() {
    let events = parse_all_events(FEED).unwrap();
    let stamp = events.iter().find_map(|e| e.dtstamp).expect("a parseable DTSTAMP");
    assert_eq!(stamp.timestamp, 197_164_800_000, "1976-04-01T00:00:00Z");
}
