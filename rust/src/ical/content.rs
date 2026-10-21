//! RFC 5545 §3.1 content lines: unfolding, lexing into properties and
//! components, TEXT escaping, and folding on output.

use std::sync::LazyLock;

use regex::Regex;

use super::ICalError;

/// A property parameter. The name is upper-cased; the value has its
/// surrounding quotes removed when it is a single quoted string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub value: String,
}

/// One unfolded content line, `NAME;PARAM=VALUE:value`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentLine {
    /// Upper-cased property name.
    pub name: String,
    pub params: Vec<Param>,
    /// The value exactly as written (still TEXT-escaped).
    pub value: String,
    /// The whole unfolded line as written, re-emitted verbatim for properties
    /// this crate does not model.
    pub raw: String,
}

impl ContentLine {
    /// The first parameter called `name` (case-insensitive).
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|p| p.name.eq_ignore_ascii_case(name)).map(|p| p.value.as_str())
    }

    /// Lex one unfolded line. `None` when it has no name or no `:` separator.
    pub fn parse(line: &str) -> Option<ContentLine> {
        let name_end = line.find([';', ':'])?;
        let name = line[..name_end].trim();
        if name.is_empty() {
            return None;
        }
        let mut params = Vec::new();
        let mut rest = &line[name_end..];
        while let Some(after_semicolon) = rest.strip_prefix(';') {
            let (param, remaining) = lex_param(after_semicolon)?;
            if let Some(param) = param {
                params.push(param);
            }
            rest = remaining;
        }
        let value = rest.strip_prefix(':')?;
        Some(ContentLine { name: name.to_ascii_uppercase(), params, value: value.to_string(), raw: line.to_string() })
    }
}

/// Lex `NAME=value` up to the next unquoted `;` or `:`; returns the param and
/// the remaining text starting at that delimiter.
fn lex_param(input: &str) -> Option<(Option<Param>, &str)> {
    let mut in_quotes = false;
    let mut end = None;
    for (i, c) in input.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ';' | ':' if !in_quotes => {
                end = Some(i);
                break;
            }
            _ => {}
        }
    }
    let end = end?;
    let text = &input[..end];
    let param = text
        .split_once('=')
        .map(|(name, value)| Param { name: name.trim().to_ascii_uppercase(), value: unquote(value) });
    Some((param, &input[end..]))
}

fn unquote(value: &str) -> String {
    let inner = value.strip_prefix('"').and_then(|v| v.strip_suffix('"'));
    match inner {
        Some(inner) if !inner.contains('"') => inner.to_string(),
        _ => value.to_string(),
    }
}

/// A component (`BEGIN:X` .. `END:X`) with its properties and subcomponents.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Component {
    /// Upper-cased component name.
    pub name: String,
    pub properties: Vec<ContentLine>,
    pub children: Vec<Component>,
}

impl Component {
    /// The first property called `name` (upper-case).
    pub fn property(&self, name: &str) -> Option<&ContentLine> {
        self.properties.iter().find(|p| p.name == name)
    }

    /// Every property called `name` (upper-case), in order.
    pub fn properties_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a ContentLine> + 'a {
        self.properties.iter().filter(move |p| p.name == name)
    }

    /// Direct subcomponents called `name` (upper-case), in order.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Component> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
}

/// Parse iCalendar data into its VCALENDAR component.
///
/// Fails when the data does not start with `BEGIN:VCALENDAR`, or when a
/// component is left open or closed out of order. Lines that are not valid
/// content lines are skipped, the way a relaxed parser tolerates server noise.
pub fn parse_calendar(data: &str) -> Result<Component, ICalError> {
    // A UTF-8 byte-order mark would otherwise glue itself onto `BEGIN`.
    let unfolded = unfold(&preprocess(data.trim_start_matches('\u{feff}')));
    let mut stack: Vec<Component> = Vec::new();
    let mut root: Option<Component> = None;

    for line in unfolded.split(['\r', '\n']).filter(|l| !l.trim().is_empty()) {
        if root.is_some() {
            break;
        }
        let Some(content) = ContentLine::parse(line) else {
            if stack.is_empty() {
                return Err(ICalError::new("Expected BEGIN:VCALENDAR"));
            }
            continue;
        };
        match content.name.as_str() {
            "BEGIN" => {
                let name = content.value.trim().to_ascii_uppercase();
                if stack.is_empty() && name != "VCALENDAR" {
                    return Err(ICalError::new("Expected BEGIN:VCALENDAR"));
                }
                stack.push(Component { name, ..Component::default() });
            }
            "END" => {
                let name = content.value.trim().to_ascii_uppercase();
                let open = stack.pop().ok_or_else(|| ICalError::new("Unexpected END"))?;
                if open.name != name {
                    return Err(ICalError::new(format!("Expected END:{}, found END:{name}", open.name)));
                }
                match stack.last_mut() {
                    Some(parent) => parent.children.push(open),
                    None => root = Some(open),
                }
            }
            _ => match stack.last_mut() {
                Some(current) => current.properties.push(content),
                None => return Err(ICalError::new("Expected BEGIN:VCALENDAR")),
            },
        }
    }

    match (root, stack.last()) {
        (Some(root), _) => Ok(root),
        (None, Some(open)) => Err(ICalError::new(format!("Unexpected end of data inside {}", open.name))),
        (None, None) => Err(ICalError::new("No VCALENDAR found")),
    }
}

static SHORT_OFFSET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)(TZOFFSETFROM|TZOFFSETTO):([+-])(\d{3})[ \t]*$").expect("valid regex"));
static DAY_DURATION_WITH_T: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^((?:TRIGGER|DURATION)[^:\r\n]*:)(-?)PT(\d+)D[ \t]*$").expect("valid regex"));
static DATE_TYPED_TIMESTAMP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^(DTSTAMP|LAST-MODIFIED|CREATED);VALUE=DATE:(\d{8})[ \t]*$").expect("valid regex")
});

/// Repair known server bugs before lexing (each fix is scoped to a single
/// property so free text cannot be touched):
/// - 3-digit UTC offsets (`+530`) padded to `+0530`;
/// - day durations written with a `T` (`-PT2D`) corrected to `-P2D`;
/// - DATE-typed DTSTAMP/LAST-MODIFIED/CREATED upgraded to midnight UTC.
pub fn preprocess(data: &str) -> String {
    let fixed = SHORT_OFFSET.replace_all(data, |c: &regex::Captures| format!("{}:{}0{}", &c[1], &c[2], &c[3]));
    let fixed = DAY_DURATION_WITH_T.replace_all(&fixed, "${1}${2}P${3}D");
    DATE_TYPED_TIMESTAMP.replace_all(&fixed, "${1}:${2}T000000Z").into_owned()
}

/// Undo RFC 5545 line folding (CRLF or bare LF followed by a space or tab).
pub fn unfold(data: &str) -> String {
    data.replace("\r\n ", "").replace("\r\n\t", "").replace("\n ", "").replace("\n\t", "")
}

/// Decode RFC 5545 §3.3.11 TEXT escapes: `\\`, `\;`, `\,`, `\n`, `\N`.
pub fn unescape_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other @ ('\\' | ';' | ',')) => out.push(other),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Split a TEXT list on unescaped commas, decoding each element.
pub fn split_text_list(value: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    for c in value.chars() {
        if escaped {
            current.push('\\');
            current.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == ',' {
            items.push(unescape_text(&current));
            current.clear();
        } else {
            current.push(c);
        }
    }
    if escaped {
        current.push('\\');
    }
    items.push(unescape_text(&current));
    items
}

/// Encode RFC 5545 §3.3.11 TEXT escapes.
pub fn escape_text(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\n', "\\n").replace(',', "\\,").replace(';', "\\;")
}

/// Append `line` with CRLF, folded so no physical line exceeds 75 octets.
///
/// Folds on character boundaries, so a multi-byte character is never split.
pub fn push_folded(out: &mut String, line: &str) {
    if line.len() <= 75 {
        out.push_str(line);
        out.push_str("\r\n");
        return;
    }
    let mut budget = 75;
    let mut used = 0;
    for c in line.chars() {
        let width = c.len_utf8();
        if used + width > budget && used > 0 {
            out.push_str("\r\n ");
            budget = 74;
            used = 0;
        }
        out.push(c);
        used += width;
    }
    out.push_str("\r\n");
}

/// Append `line` with CRLF and no folding (for short, fixed-shape lines).
pub fn push_line(out: &mut String, line: &str) {
    out.push_str(line);
    out.push_str("\r\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexes_name_params_and_value() {
        let line = ContentLine::parse("DTSTART;TZID=America/New_York;VALUE=DATE-TIME:20240101T090000").unwrap();
        assert_eq!(line.name, "DTSTART");
        assert_eq!(line.param("tzid"), Some("America/New_York"));
        assert_eq!(line.param("VALUE"), Some("DATE-TIME"));
        assert_eq!(line.value, "20240101T090000");
    }

    #[test]
    fn quoted_params_may_contain_delimiters() {
        let line = ContentLine::parse(
            r#"X-APPLE-STRUCTURED-LOCATION;VALUE=URI;X-TITLE="Apple Park; Cupertino:CA":geo:37.3,-122.0"#,
        )
        .unwrap();
        assert_eq!(line.param("X-TITLE"), Some("Apple Park; Cupertino:CA"));
        assert_eq!(line.value, "geo:37.3,-122.0");
    }

    #[test]
    fn value_may_contain_colons() {
        let line = ContentLine::parse("URL:https://example.com:8443/x").unwrap();
        assert_eq!(line.value, "https://example.com:8443/x");
        assert_eq!(ContentLine::parse("lowercase:value").unwrap().name, "LOWERCASE");
    }

    #[test]
    fn rejects_lines_without_separator() {
        assert!(ContentLine::parse("no separator here").is_none());
        assert!(ContentLine::parse(":value").is_none());
    }

    #[test]
    fn builds_component_tree() {
        let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:a\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let cal = parse_calendar(ics).unwrap();
        assert_eq!(cal.name, "VCALENDAR");
        let event = cal.children_named("VEVENT").next().unwrap();
        assert_eq!(event.property("UID").unwrap().value, "a");
        assert_eq!(event.children_named("VALARM").count(), 1);
    }

    #[test]
    fn accepts_lf_only_and_folded_lines() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:a\nSUMMARY:Long\n  title\nEND:VEVENT\nEND:VCALENDAR\n";
        let cal = parse_calendar(ics).unwrap();
        let event = cal.children_named("VEVENT").next().unwrap();
        assert_eq!(event.property("SUMMARY").unwrap().value, "Long title");
    }

    #[test]
    fn rejects_non_calendar_and_unbalanced_input() {
        assert!(parse_calendar("not valid ical data").is_err());
        assert!(parse_calendar("").is_err());
        assert!(parse_calendar("BEGIN:VEVENT\nEND:VEVENT").is_err());
        assert!(parse_calendar("BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x").is_err());
        assert!(parse_calendar("BEGIN:VCALENDAR\nBEGIN:VEVENT\nEND:VTODO\nEND:VCALENDAR").is_err());
    }

    #[test]
    fn text_escaping_round_trips() {
        let original = "a,b;c\\d\ne";
        assert_eq!(escape_text(original), "a\\,b\\;c\\\\d\\ne");
        assert_eq!(unescape_text(&escape_text(original)), original);
        assert_eq!(unescape_text("line\\Nbreak"), "line\nbreak");
        assert_eq!(unescape_text("C:\\Users"), "C:\\Users");
    }

    #[test]
    fn splits_text_lists_on_unescaped_commas() {
        assert_eq!(split_text_list("a,b\\,c,d"), vec!["a", "b,c", "d"]);
        assert_eq!(split_text_list("solo"), vec!["solo"]);
    }

    #[test]
    fn folds_at_75_octets_on_char_boundaries() {
        let line = format!("SUMMARY:{}", "é".repeat(60));
        let mut out = String::new();
        push_folded(&mut out, &line);
        for physical in out.split("\r\n").filter(|l| !l.is_empty()) {
            assert!(physical.len() <= 75, "{} octets", physical.len());
        }
        assert_eq!(unfold(&out).trim_end(), line);
    }

    #[test]
    fn preprocess_repairs_server_quirks() {
        let fixed = preprocess("TZOFFSETFROM:+530\nTRIGGER:-PT2D\nDTSTAMP;VALUE=DATE:20240101\nDESCRIPTION:-PT2D");
        assert!(fixed.contains("TZOFFSETFROM:+0530"));
        assert!(fixed.contains("TRIGGER:-P2D"));
        assert!(fixed.contains("DTSTAMP:20240101T000000Z"));
        assert!(fixed.contains("DESCRIPTION:-PT2D"));
    }
}
