//! Parsing of iCloud CalDAV multistatus (207) XML responses.
//!
//! iCloud mixes several namespace prefixes (`D:`, `C:`, `CS:`, `ICAL:`) and may
//! wrap calendar data in CDATA, so elements are matched by local name in any
//! namespace. DTDs are rejected outright (XXE protection), and any document that
//! fails to parse yields an empty/absent result rather than an error, matching
//! how the callers treat a garbled body.

use std::collections::HashMap;

use roxmltree::{Document, Node, ParsingOptions};

use super::etag::normalize_etag;
use super::url_normalizer::normalize_icloud_url;
use super::{CalDavCalendar, CalDavEvent, extract_calendar_id_from_href, normalize_color};

/// Stateless parser for the CalDAV responses this client consumes.
#[derive(Debug, Clone, Copy, Default)]
pub struct ICloudXmlParser;

impl ICloudXmlParser {
    pub fn new() -> Self {
        Self
    }

    /// Calendar collections from a Depth:1 PROPFIND on a calendar-home-set.
    /// Non-calendar collections and responses without a 200 propstat are skipped.
    pub fn parse_calendars(&self, xml: &str, base_url: &str) -> Vec<CalDavCalendar> {
        let Some(doc) = parse_xml(xml) else {
            return Vec::new();
        };
        descendants_named(doc.root(), "response")
            .filter_map(|response| parse_calendar_response(response, base_url))
            .collect()
    }

    /// The `current-user-principal` href, if present.
    pub fn parse_current_user_principal(&self, xml: &str) -> Option<String> {
        let doc = parse_xml(xml)?;
        let principal = descendants_named(doc.root(), "current-user-principal").next()?;
        element_text(principal, "href")
    }

    /// The first `calendar-home-set` href, if present.
    pub fn parse_calendar_home_set(&self, xml: &str) -> Option<String> {
        let doc = parse_xml(xml)?;
        let home_set = descendants_named(doc.root(), "calendar-home-set").next()?;
        element_text(home_set, "href")
    }

    /// Every non-blank href across all `calendar-home-set` elements
    /// (RFC 4791 §6.2.1 allows more than one).
    pub fn parse_calendar_home_sets(&self, xml: &str) -> Vec<String> {
        let Some(doc) = parse_xml(xml) else {
            return Vec::new();
        };
        descendants_named(doc.root(), "calendar-home-set")
            .flat_map(|home_set| descendants_named(home_set, "href"))
            .map(text_content)
            .filter(|href| !href.is_empty())
            .collect()
    }

    /// Event resources from a calendar-query or calendar-multiget REPORT.
    /// Responses without a 200 status, calendar data or a UID are skipped.
    pub fn parse_events(&self, xml: &str, base_url: &str) -> Vec<CalDavEvent> {
        let Some(doc) = parse_xml(xml) else {
            return Vec::new();
        };
        descendants_named(doc.root(), "response")
            .filter_map(|response| self.parse_event_response(response, base_url))
            .collect()
    }

    fn parse_event_response(&self, response: Node, base_url: &str) -> Option<CalDavEvent> {
        if !has_success_status(response) {
            return None;
        }
        // Regional partitions are collapsed so cache keys and minted handles do
        // not depend on which iCloud partition answered.
        let href = normalize_icloud_url(&element_text(response, "href")?);
        let ical_data = element_text(response, "calendar-data")?;
        if ical_data.is_empty() {
            return None;
        }
        let uid = self.extract_uid(&ical_data)?;
        let etag = normalize_etag(element_text(response, "getetag").as_deref());
        Some(CalDavEvent { uid, url: build_url(base_url, &href), href, etag, ical_data })
    }

    /// The UID of the first `UID:` line, after RFC 5545 line unfolding.
    pub fn extract_uid(&self, ics: &str) -> Option<String> {
        if ics.trim().is_empty() {
            return None;
        }
        let unfolded = ics.replace("\r\n ", "").replace("\r\n\t", "").replace("\n ", "").replace("\n\t", "");
        unfolded
            .split(['\r', '\n'])
            .map(str::trim)
            .find(|line| line.len() >= 4 && line[..4].eq_ignore_ascii_case("UID:"))
            .map(|line| line[4..].trim().to_string())
    }

    /// The `sync-token` of a sync-collection response.
    pub fn parse_sync_token(&self, xml: &str) -> Option<String> {
        let doc = parse_xml(xml)?;
        descendants_named(doc.root(), "sync-token").next().map(text_content)
    }

    /// Hrefs whose response carries a direct 404 status (deleted in a sync-collection).
    pub fn parse_deleted_event_hrefs(&self, xml: &str) -> Vec<String> {
        let Some(doc) = parse_xml(xml) else {
            return Vec::new();
        };
        descendants_named(doc.root(), "response")
            .filter(|response| direct_child_text(*response, "status").is_some_and(|s| s.contains("404")))
            .filter_map(|response| element_text(response, "href"))
            .collect()
    }

    /// The first `getetag` in the document, normalized.
    pub fn parse_etag(&self, xml: &str) -> Option<String> {
        let doc = parse_xml(xml)?;
        let etag = descendants_named(doc.root(), "getetag").next()?;
        normalize_etag(Some(&text_content(etag)))
    }

    /// href -> normalized ETag for every response carrying an href.
    pub fn parse_etags(&self, xml: &str) -> HashMap<String, Option<String>> {
        let Some(doc) = parse_xml(xml) else {
            return HashMap::new();
        };
        descendants_named(doc.root(), "response")
            .filter_map(|response| {
                let href = element_text(response, "href")?;
                Some((href, normalize_etag(element_text(response, "getetag").as_deref())))
            })
            .collect()
    }
}

fn parse_calendar_response(response: Node, base_url: &str) -> Option<CalDavCalendar> {
    let href = element_text(response, "href")?;
    // Properties are read from the 200 propstat only, so a 404/403 propstat
    // listing the same property names cannot leak into the result.
    let prop = success_prop(response)?;
    if !is_calendar_collection(prop) {
        return None;
    }
    let id = extract_calendar_id_from_href(&href);
    Some(CalDavCalendar {
        display_name: element_text(prop, "displayname").unwrap_or_else(|| id.clone()),
        color: normalize_color(element_text(prop, "calendar-color").as_deref()),
        ctag: element_text(prop, "getctag"),
        is_read_only: !has_write_privilege(prop),
        supported_components: supported_components(prop),
        url: build_url(base_url, &href),
        id,
        href,
    })
}

/// The `<prop>` of the first propstat whose status is 200; a propstat without
/// a status is treated as 200.
fn success_prop<'a, 'i>(response: Node<'a, 'i>) -> Option<Node<'a, 'i>> {
    descendants_named(response, "propstat")
        .filter(|propstat| element_text(*propstat, "status").is_none_or(|status| status.contains("200")))
        .find_map(|propstat| descendants_named(propstat, "prop").next())
}

fn is_calendar_collection(prop: Node) -> bool {
    descendants_named(prop, "resourcetype")
        .next()
        .is_some_and(|resource_type| resource_type.children().any(|child| is_element_named(child, "calendar")))
}

/// Writable unless a privilege set is present and grants neither `write` nor
/// `write-content`.
fn has_write_privilege(prop: Node) -> bool {
    let Some(privilege_set) = descendants_named(prop, "current-user-privilege-set").next() else {
        return true;
    };
    descendants_named(privilege_set, "privilege").any(|privilege| {
        privilege.children().any(|child| is_element_named(child, "write") || is_element_named(child, "write-content"))
    })
}

/// Component names in document order, deduplicated.
fn supported_components(prop: Node) -> Vec<String> {
    let Some(component_set) = descendants_named(prop, "supported-calendar-component-set").next() else {
        return Vec::new();
    };
    let mut components = Vec::new();
    for name in descendants_named(component_set, "comp").filter_map(|comp| comp.attribute("name")) {
        if !name.trim().is_empty() && !components.iter().any(|seen| seen == name) {
            components.push(name.to_string());
        }
    }
    components
}

fn has_success_status(response: Node) -> bool {
    descendants_named(response, "propstat")
        .any(|propstat| element_text(propstat, "status").is_some_and(|status| status.contains("200")))
        || direct_child_text(response, "status").is_some_and(|status| status.contains("200"))
}

fn parse_xml(xml: &str) -> Option<Document<'_>> {
    let options = ParsingOptions { allow_dtd: false, ..ParsingOptions::default() };
    Document::parse_with_options(xml, options).ok()
}

fn is_element_named(node: Node, local_name: &str) -> bool {
    node.is_element() && node.tag_name().name() == local_name
}

/// Descendant elements (excluding `parent` itself) with the given local name in
/// any namespace, in document order.
fn descendants_named<'a, 'i>(parent: Node<'a, 'i>, local_name: &'static str) -> impl Iterator<Item = Node<'a, 'i>> {
    parent.descendants().filter(move |node| *node != parent && is_element_named(*node, local_name))
}

/// Trimmed text content of the first descendant with the given local name.
fn element_text(parent: Node, local_name: &'static str) -> Option<String> {
    descendants_named(parent, local_name).next().map(text_content)
}

/// Trimmed text content of the first direct child with the given local name.
fn direct_child_text(parent: Node, local_name: &str) -> Option<String> {
    parent.children().find(|child| is_element_named(*child, local_name)).map(text_content)
}

/// All text (including CDATA) beneath `node`, trimmed.
fn text_content(node: Node) -> String {
    let text: String = node.descendants().filter(Node::is_text).filter_map(|n| n.text()).collect();
    text.trim().to_string()
}

fn build_url(base_url: &str, href: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if href.starts_with('/') { format!("{base}{href}") } else { format!("{base}/{href}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://caldav.icloud.com";

    fn parser() -> ICloudXmlParser {
        ICloudXmlParser::new()
    }

    fn calendar_response(href: &str, prop: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:CS="http://calendarserver.org/ns/" xmlns:C="urn:ietf:params:xml:ns:caldav">
    <D:response>
        <D:href>{href}</D:href>
        <D:propstat>
            <D:prop>{prop}</D:prop>
            <D:status>HTTP/1.1 200 OK</D:status>
        </D:propstat>
    </D:response>
</D:multistatus>"#
        )
    }

    const CALENDAR_TYPE: &str = "<D:resourcetype><D:collection/><C:calendar/></D:resourcetype>";

    fn event_response(href: &str, etag: &str, data: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
<D:response>
<D:href>{href}</D:href>
<D:propstat>
<D:prop>
<D:getetag>{etag}</D:getetag>
<C:calendar-data>{data}</C:calendar-data>
</D:prop>
<D:status>HTTP/1.1 200 OK</D:status>
</D:propstat>
</D:response>
</D:multistatus>"#
        )
    }

    // ── calendar discovery ──

    #[test]
    fn parse_calendars_extracts_calendar_from_standard_response() {
        let xml = calendar_response(
            "/1234567/calendars/home/",
            &format!(
                r##"<D:displayname>Personal</D:displayname>{CALENDAR_TYPE}<CS:getctag>abc123</CS:getctag>
                <ICAL:calendar-color xmlns:ICAL="http://apple.com/ns/ical/">#FF5733FF</ICAL:calendar-color>"##
            ),
        );
        let calendars = parser().parse_calendars(&xml, BASE);
        assert_eq!(calendars.len(), 1);
        let cal = &calendars[0];
        assert_eq!(cal.id, "home");
        assert_eq!(cal.href, "/1234567/calendars/home/");
        assert_eq!(cal.url, "https://caldav.icloud.com/1234567/calendars/home/");
        assert_eq!(cal.display_name, "Personal");
        assert_eq!(cal.color.as_deref(), Some("#FF5733"));
        assert_eq!(cal.ctag.as_deref(), Some("abc123"));
        assert!(!cal.is_read_only);
    }

    #[test]
    fn parse_calendars_handles_multiple_calendars() {
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
<D:response><D:href>/123/calendars/home/</D:href>
<D:propstat><D:prop><D:displayname>Home</D:displayname>{CALENDAR_TYPE}</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
<D:response><D:href>/123/calendars/work/</D:href>
<D:propstat><D:prop><D:displayname>Work</D:displayname>{CALENDAR_TYPE}</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
</D:multistatus>"#
        );
        let calendars = parser().parse_calendars(&xml, BASE);
        assert_eq!(calendars.len(), 2);
        assert_eq!(calendars[0].display_name, "Home");
        assert_eq!(calendars[1].display_name, "Work");
    }

    #[test]
    fn parse_calendars_skips_non_calendar_collections() {
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
<D:response><D:href>/123/calendars/</D:href>
<D:propstat><D:prop><D:displayname>Calendars</D:displayname><D:resourcetype><D:collection/></D:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
<D:response><D:href>/123/calendars/home/</D:href>
<D:propstat><D:prop><D:displayname>Home</D:displayname>{CALENDAR_TYPE}</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
</D:multistatus>"#
        );
        let calendars = parser().parse_calendars(&xml, BASE);
        assert_eq!(calendars.len(), 1);
        assert_eq!(calendars[0].display_name, "Home");
    }

    #[test]
    fn parse_calendars_handles_missing_color_and_ctag() {
        let xml =
            calendar_response("/123/calendars/home/", &format!("<D:displayname>Home</D:displayname>{CALENDAR_TYPE}"));
        let calendars = parser().parse_calendars(&xml, BASE);
        assert_eq!(calendars.len(), 1);
        assert_eq!(calendars[0].color, None);
        assert_eq!(calendars[0].ctag, None);
    }

    #[test]
    fn missing_display_name_falls_back_to_id() {
        let xml = calendar_response("/123/calendars/work/", CALENDAR_TYPE);
        assert_eq!(parser().parse_calendars(&xml, BASE)[0].display_name, "work");
    }

    #[test]
    fn parse_calendars_handles_read_only_calendars() {
        let xml = calendar_response(
            "/shared/calendars/team/",
            &format!(
                "<D:displayname>Team Shared</D:displayname>{CALENDAR_TYPE}
                <D:current-user-privilege-set><D:privilege><D:read/></D:privilege></D:current-user-privilege-set>"
            ),
        );
        let calendars = parser().parse_calendars(&xml, BASE);
        assert_eq!(calendars.len(), 1);
        assert!(calendars[0].is_read_only);
    }

    #[test]
    fn parse_calendars_handles_writable_calendars() {
        let xml = calendar_response(
            "/123/calendars/home/",
            &format!(
                "<D:displayname>Home</D:displayname>{CALENDAR_TYPE}
                <D:current-user-privilege-set>
                    <D:privilege><D:read/></D:privilege>
                    <D:privilege><D:write/></D:privilege>
                </D:current-user-privilege-set>"
            ),
        );
        assert!(!parser().parse_calendars(&xml, BASE)[0].is_read_only);
    }

    #[test]
    fn parse_calendars_recognizes_write_content_privilege() {
        let xml = calendar_response(
            "/123/calendars/home/",
            &format!(
                "{CALENDAR_TYPE}<D:current-user-privilege-set>
                    <D:privilege><D:read/></D:privilege>
                    <D:privilege><D:write-content/></D:privilege>
                </D:current-user-privilege-set>"
            ),
        );
        assert!(!parser().parse_calendars(&xml, BASE)[0].is_read_only);
    }

    #[test]
    fn parse_calendars_returns_empty_for_empty_or_invalid_response() {
        let empty = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<D:multistatus xmlns:D=\"DAV:\">\n</D:multistatus>";
        assert!(parser().parse_calendars(empty, BASE).is_empty());
        assert!(parser().parse_calendars("not xml at all", BASE).is_empty());
    }

    #[test]
    fn parse_calendars_scopes_properties_to_200_propstat() {
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav" xmlns:CS="http://calendarserver.org/ns/">
<D:response><D:href>/123/calendars/home/</D:href>
<D:propstat><D:prop><D:displayname>Home</D:displayname>{CALENDAR_TYPE}<CS:getctag>ctag-200</CS:getctag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat>
<D:propstat><D:prop><CS:getctag>ctag-404-should-be-ignored</CS:getctag></D:prop><D:status>HTTP/1.1 404 Not Found</D:status></D:propstat>
</D:response></D:multistatus>"#
        );
        let calendars = parser().parse_calendars(&xml, BASE);
        assert_eq!(calendars.len(), 1);
        assert_eq!(calendars[0].ctag.as_deref(), Some("ctag-200"));
    }

    #[test]
    fn non_200_propstat_listed_first_is_skipped() {
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav" xmlns:CS="http://calendarserver.org/ns/">
<D:response><D:href>/123/calendars/home/</D:href>
<D:propstat><D:prop><CS:getctag>wrong</CS:getctag></D:prop><D:status>HTTP/1.1 404 Not Found</D:status></D:propstat>
<D:propstat><D:prop>{CALENDAR_TYPE}<CS:getctag>right</CS:getctag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat>
</D:response></D:multistatus>"#
        );
        assert_eq!(parser().parse_calendars(&xml, BASE)[0].ctag.as_deref(), Some("right"));
    }

    #[test]
    fn parse_calendars_treats_propstat_without_status_as_200() {
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
<D:response><D:href>/123/calendars/home/</D:href>
<D:propstat><D:prop><D:displayname>No Status</D:displayname>{CALENDAR_TYPE}</D:prop></D:propstat>
</D:response></D:multistatus>"#
        );
        let calendars = parser().parse_calendars(&xml, BASE);
        assert_eq!(calendars.len(), 1);
        assert_eq!(calendars[0].display_name, "No Status");
    }

    #[test]
    fn parse_calendars_ignores_403_propstat_properties() {
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
<D:response><D:href>/123/calendars/home/</D:href>
<D:propstat><D:prop><D:displayname>Visible</D:displayname>{CALENDAR_TYPE}</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat>
<D:propstat><D:prop><D:displayname>Forbidden Name</D:displayname></D:prop><D:status>HTTP/1.1 403 Forbidden</D:status></D:propstat>
</D:response></D:multistatus>"#
        );
        assert_eq!(parser().parse_calendars(&xml, BASE)[0].display_name, "Visible");
    }

    #[test]
    fn parse_calendars_extracts_supported_components_in_order_deduplicated() {
        let xml = calendar_response(
            "/123/calendars/home/",
            &format!(
                r#"{CALENDAR_TYPE}<C:supported-calendar-component-set>
                    <C:comp name="VEVENT"/><C:comp name="VTODO"/><C:comp name="VEVENT"/><C:comp name=" "/>
                </C:supported-calendar-component-set>"#
            ),
        );
        assert_eq!(parser().parse_calendars(&xml, BASE)[0].supported_components, vec!["VEVENT", "VTODO"]);
    }

    #[test]
    fn parse_calendars_returns_empty_components_when_absent() {
        let xml = calendar_response("/123/calendars/home/", CALENDAR_TYPE);
        assert!(parser().parse_calendars(&xml, BASE)[0].supported_components.is_empty());
    }

    #[test]
    fn dtd_is_rejected() {
        let xml = r#"<?xml version="1.0"?>
<!DOCTYPE foo [<!ENTITY xxe SYSTEM "file:///etc/passwd">]>
<D:multistatus xmlns:D="DAV:"><D:response><D:href>&xxe;</D:href></D:response></D:multistatus>"#;
        assert!(parser().parse_calendars(xml, BASE).is_empty());
        assert!(parser().parse_etags(xml).is_empty());
        assert_eq!(parser().parse_current_user_principal(xml), None);
    }

    #[test]
    fn unbound_namespace_prefix_is_a_parse_failure() {
        let xml = "<D:multistatus><D:response><D:href>/x/</D:href></D:response></D:multistatus>";
        assert!(parser().parse_etags(xml).is_empty());
    }

    // ── principal / home set ──

    #[test]
    fn parse_current_user_principal_extracts_href() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:"><D:response><D:href>/</D:href>
<D:propstat><D:prop><D:current-user-principal><D:href>/1234567/principal/</D:href></D:current-user-principal></D:prop>
<D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#;
        assert_eq!(parser().parse_current_user_principal(xml).as_deref(), Some("/1234567/principal/"));
    }

    #[test]
    fn parse_current_user_principal_returns_none_when_missing() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:"><D:response><D:href>/</D:href>
<D:propstat><D:prop></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#;
        assert_eq!(parser().parse_current_user_principal(xml), None);
    }

    const TWO_HOME_SETS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:response><D:href>/principal/</D:href>
<D:propstat><D:prop><C:calendar-home-set><D:href>/123/calendars/</D:href><D:href>/123/shared/</D:href></C:calendar-home-set></D:prop>
<D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#;

    #[test]
    fn parse_calendar_home_set_extracts_first_href() {
        assert_eq!(parser().parse_calendar_home_set(TWO_HOME_SETS).as_deref(), Some("/123/calendars/"));
    }

    #[test]
    fn parse_calendar_home_sets_returns_all_hrefs() {
        assert_eq!(parser().parse_calendar_home_sets(TWO_HOME_SETS), vec!["/123/calendars/", "/123/shared/"]);
    }

    #[test]
    fn parse_calendar_home_sets_single_and_empty() {
        let single = TWO_HOME_SETS.replace("<D:href>/123/shared/</D:href>", "");
        assert_eq!(parser().parse_calendar_home_sets(&single), vec!["/123/calendars/"]);
        let none = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:"><D:response><D:href>/</D:href>
<D:propstat><D:prop></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#;
        assert!(parser().parse_calendar_home_sets(none).is_empty());
    }

    // ── events ──

    #[test]
    fn parse_events_extracts_event() {
        let ics = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:event-123@icloud.com\nDTSTART:20250115T090000Z\nDTEND:20250115T100000Z\nSUMMARY:Meeting\nEND:VEVENT\nEND:VCALENDAR";
        let xml = event_response("/123/calendars/home/event-123.ics", "\"etag-abc\"", ics);
        let events = parser().parse_events(&xml, BASE);
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.uid, "event-123@icloud.com");
        assert_eq!(event.href, "/123/calendars/home/event-123.ics");
        assert_eq!(event.url, "https://caldav.icloud.com/123/calendars/home/event-123.ics");
        assert_eq!(event.etag.as_deref(), Some("etag-abc"));
        assert!(event.ical_data.contains("SUMMARY:Meeting"));
    }

    #[test]
    fn parse_events_handles_cdata() {
        let ics = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:cdata-test@icloud.com\nSUMMARY:CDATA Test\nEND:VEVENT\nEND:VCALENDAR";
        let xml = event_response("/123/calendars/home/cdata-test.ics", "\"etag-xyz\"", &format!("<![CDATA[{ics}]]>"));
        let events = parser().parse_events(&xml, BASE);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].uid, "cdata-test@icloud.com");
        assert!(events[0].ical_data.contains("SUMMARY:CDATA Test"));
    }

    #[test]
    fn parse_events_decodes_entities() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:amp@test\nSUMMARY:Tom &amp; Jerry\nEND:VEVENT\nEND:VCALENDAR";
        let xml = event_response("/cal/amp.ics", "&quot;e&quot;", ics);
        let events = parser().parse_events(&xml, BASE);
        assert!(events[0].ical_data.contains("SUMMARY:Tom & Jerry"));
        assert_eq!(events[0].etag.as_deref(), Some("e"));
    }

    #[test]
    fn parse_events_handles_multiple_events() {
        let ics1 = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:uid1@test\nEND:VEVENT\nEND:VCALENDAR";
        let ics2 = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:uid2@test\nEND:VEVENT\nEND:VCALENDAR";
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
<D:response><D:href>/cal/event1.ics</D:href><D:propstat><D:prop><D:getetag>"e1"</D:getetag><C:calendar-data>{ics1}</C:calendar-data></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
<D:response><D:href>/cal/event2.ics</D:href><D:propstat><D:prop><D:getetag>"e2"</D:getetag><C:calendar-data>{ics2}</C:calendar-data></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
</D:multistatus>"#
        );
        let events = parser().parse_events(&xml, BASE);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].uid, "uid1@test");
        assert_eq!(events[1].uid, "uid2@test");
    }

    #[test]
    fn parse_events_normalizes_regional_hrefs() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:r@test\nEND:VEVENT\nEND:VCALENDAR";
        let xml = event_response("https://p180-caldav.icloud.com:443/123/calendars/home/r.ics", "\"e\"", ics);
        assert_eq!(parser().parse_events(&xml, BASE)[0].href, "https://caldav.icloud.com/123/calendars/home/r.ics");
    }

    #[test]
    fn parse_events_skips_responses_without_data_uid_or_success() {
        let no_data = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:"><D:response><D:href>/cal/event1.ics</D:href>
<D:propstat><D:prop><D:getetag>"e1"</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#;
        assert!(parser().parse_events(no_data, BASE).is_empty());

        let not_found = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:"><D:response><D:href>/cal/deleted.ics</D:href>
<D:propstat><D:prop/><D:status>HTTP/1.1 404 Not Found</D:status></D:propstat></D:response></D:multistatus>"#;
        assert!(parser().parse_events(not_found, BASE).is_empty());

        let no_uid = event_response(
            "/cal/x.ics",
            "\"e\"",
            "BEGIN:VCALENDAR\nBEGIN:VEVENT\nSUMMARY:x\nEND:VEVENT\nEND:VCALENDAR",
        );
        assert!(parser().parse_events(&no_uid, BASE).is_empty());

        let blank = event_response("/cal/x.ics", "\"e\"", "   ");
        assert!(parser().parse_events(&blank, BASE).is_empty());

        assert!(parser().parse_events("not xml", BASE).is_empty());
    }

    #[test]
    fn parse_events_accepts_direct_response_status() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:response><D:href>/cal/d.ics</D:href>
<C:calendar-data>BEGIN:VCALENDAR
UID:direct@test
END:VCALENDAR</C:calendar-data><D:status>HTTP/1.1 200 OK</D:status></D:response></D:multistatus>"#;
        assert_eq!(parser().parse_events(xml, BASE)[0].uid, "direct@test");
    }

    // ── UID extraction ──

    #[test]
    fn extract_uid_from_standard_ics() {
        let ics = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:unique-id-123@example.com\nSUMMARY:Test\nEND:VEVENT\nEND:VCALENDAR";
        assert_eq!(parser().extract_uid(ics).as_deref(), Some("unique-id-123@example.com"));
    }

    #[test]
    fn extract_uid_handles_folded_lines() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:very-long-unique-identifier-\r\n and-continues-here@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR";
        assert_eq!(
            parser().extract_uid(ics).as_deref(),
            Some("very-long-unique-identifier-and-continues-here@example.com")
        );
    }

    #[test]
    fn extract_uid_is_case_insensitive_and_trims() {
        assert_eq!(parser().extract_uid("BEGIN:VEVENT\nuid:  abc  \nEND:VEVENT").as_deref(), Some("abc"));
    }

    #[test]
    fn extract_uid_returns_none_when_missing_or_empty() {
        let ics = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nSUMMARY:No UID Event\nEND:VEVENT\nEND:VCALENDAR";
        assert_eq!(parser().extract_uid(ics), None);
        assert_eq!(parser().extract_uid(""), None);
        assert_eq!(parser().extract_uid("   "), None);
    }

    // ── sync token / deletions / etags ──

    #[test]
    fn parse_sync_token_extracts_or_returns_none() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:"><D:sync-token>https://caldav.icloud.com/sync/token-abc123</D:sync-token>
<D:response><D:href>/cal/event.ics</D:href><D:propstat><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#;
        assert_eq!(parser().parse_sync_token(xml).as_deref(), Some("https://caldav.icloud.com/sync/token-abc123"));
        let none = r#"<D:multistatus xmlns:D="DAV:"><D:response><D:href>/cal/</D:href></D:response></D:multistatus>"#;
        assert_eq!(parser().parse_sync_token(none), None);
    }

    #[test]
    fn parse_deleted_event_hrefs_extracts_404s() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:">
<D:response><D:href>/cal/event1.ics</D:href><D:status>HTTP/1.1 200 OK</D:status></D:response>
<D:response><D:href>/cal/deleted-event.ics</D:href><D:status>HTTP/1.1 404 Not Found</D:status></D:response>
</D:multistatus>"#;
        assert_eq!(parser().parse_deleted_event_hrefs(xml), vec!["/cal/deleted-event.ics"]);
        let none = xml.replace("404 Not Found", "200 OK");
        assert!(parser().parse_deleted_event_hrefs(&none).is_empty());
    }

    #[test]
    fn parse_etag_extracts_or_returns_none() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:"><D:response><D:href>/cal/new-event.ics</D:href>
<D:propstat><D:prop><D:getetag>"new-etag-123"</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#;
        assert_eq!(parser().parse_etag(xml).as_deref(), Some("new-etag-123"));
        let none =
            r#"<D:multistatus xmlns:D="DAV:"><D:response><D:href>/cal/event.ics</D:href></D:response></D:multistatus>"#;
        assert_eq!(parser().parse_etag(none), None);
    }

    #[test]
    fn parse_etags_maps_href_to_etag() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:">
<D:response><D:href>/cal/event1.ics</D:href><D:propstat><D:prop><D:getetag>"etag-a"</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
<D:response><D:href>/cal/event2.ics</D:href><D:propstat><D:prop><D:getetag>"etag-b"</D:getetag></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
<D:response><D:href>/cal/event3.ics</D:href><D:propstat><D:prop></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
</D:multistatus>"#;
        let etags = parser().parse_etags(xml);
        assert_eq!(etags.len(), 3);
        assert_eq!(etags["/cal/event1.ics"].as_deref(), Some("etag-a"));
        assert_eq!(etags["/cal/event2.ics"].as_deref(), Some("etag-b"));
        assert_eq!(etags["/cal/event3.ics"], None);
    }

    #[test]
    fn build_url_joins_with_single_slash() {
        assert_eq!(build_url("https://h/", "/a"), "https://h/a");
        assert_eq!(build_url("https://h", "a"), "https://h/a");
    }
}
