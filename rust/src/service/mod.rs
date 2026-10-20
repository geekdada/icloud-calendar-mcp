//! Calendar orchestration over a [`CalDavClient`]: listing, range queries with
//! recurrence expansion, and writes, including the occurrence-scoped edits of
//! recurring series.

pub mod handle;

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chrono::{DateTime, NaiveDate, NaiveTime, TimeDelta, Utc};

pub use handle::EventHandle;

use crate::caldav::{CalDavClient, CalDavError, CalDavEvent, extract_calendar_id_from_href, normalize_icloud_url};
use crate::ical::expander::ExpansionLimit;
use crate::ics::patcher::{self, EventPatch, PatchError};
use crate::ics::{NewEvent, ParsedAlarm, ParsedEvent, build_event, parse_events, parse_occurrences};
use crate::scope::EventScope;

/// Most events one `get_events` response may carry. A larger response can be
/// truncated into invalid JSON by a downstream consumer, so the service fails
/// with a 413 instead; there is no pagination, the caller narrows the range.
pub const MAX_RETURNED_EVENTS: usize = 1000;

const TOO_MANY_EVENTS_MESSAGE: &str =
    "Too many events in range (exceeds 1000). Narrow the date range (try a week or a month at a time).";

const EXPANSION_TOO_LARGE_MESSAGE: &str = "A recurring event in this range expands to too many occurrences. Narrow the date range (try a week or a month at a time).";

/// A failed service operation with an HTTP-like status code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceError {
    pub code: u16,
    pub message: String,
}

impl ServiceError {
    pub fn new(code: u16, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

impl std::error::Error for ServiceError {}

impl From<CalDavError> for ServiceError {
    fn from(error: CalDavError) -> Self {
        ServiceError::new(error.code, error.message)
    }
}

impl From<ExpansionLimit> for ServiceError {
    fn from(_: ExpansionLimit) -> Self {
        ServiceError::new(413, EXPANSION_TOO_LARGE_MESSAGE)
    }
}

pub type ServiceResult<T> = Result<T, ServiceError>;

/// A calendar as reported to MCP clients.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarInfo {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
    pub read_only: bool,
    pub supported_components: Vec<String>,
}

/// An event (or one occurrence) as reported to MCP clients, with the CalDAV
/// coordinates needed to write it back.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EventInfo {
    pub uid: String,
    pub href: String,
    pub etag: Option<String>,
    /// Opaque, self-contained reference for update/delete (see [`EventHandle`]).
    pub handle: Option<String>,
    pub summary: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub is_all_day: bool,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub rrule: Option<String>,
    /// RECURRENCE-ID wire form when this is one occurrence of a series.
    pub recurrence_id: Option<String>,
    pub status: Option<String>,
    pub url: Option<String>,
    pub categories: Vec<String>,
    pub priority: Option<i32>,
    pub organizer: Option<String>,
    pub attendee_count: usize,
    pub alarms: Vec<ParsedAlarm>,
}

struct CachedEvent {
    event: CalDavEvent,
    cached_at: Instant,
}

/// The UTC window a `get_events` query covers. `end_date` is a day the caller
/// wants included, so the window runs to the start of the following day.
struct DayRangeUtc {
    start_day: NaiveDate,
    end_day: NaiveDate,
}

impl DayRangeUtc {
    fn parse(start_date: &str, end_date: &str) -> Option<Self> {
        let parse = |d: &str| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok();
        Some(Self { start_day: parse(start_date)?, end_day: parse(end_date)? })
    }

    fn end_day_exclusive(&self) -> NaiveDate {
        self.end_day + TimeDelta::days(1)
    }

    fn start_instant(&self) -> DateTime<Utc> {
        self.start_day.and_time(NaiveTime::MIN).and_utc()
    }

    fn end_instant_exclusive(&self) -> DateTime<Utc> {
        self.end_day_exclusive().and_time(NaiveTime::MIN).and_utc()
    }

    /// Last second inside the window; expansion takes an inclusive bound.
    fn last_instant(&self) -> DateTime<Utc> {
        self.end_day.and_hms_opt(23, 59, 59).unwrap_or_default().and_utc()
    }
}

/// Calendar operations over CalDAV, with a short-lived UID cache so a bare UID
/// from a recent `get_events` can still be written to. Handles never need it.
pub struct CalendarService {
    client: Arc<dyn CalDavClient>,
    cache_ttl: Duration,
    max_cache_size: usize,
    cache: Mutex<HashMap<String, CachedEvent>>,
    connection_validated: AtomicBool,
}

impl CalendarService {
    /// A service with the default cache (5 minutes, 1000 entries).
    pub fn new(client: Arc<dyn CalDavClient>) -> Self {
        Self::with_cache(client, Duration::from_secs(5 * 60), 1000)
    }

    pub fn with_cache(client: Arc<dyn CalDavClient>, cache_ttl: Duration, max_cache_size: usize) -> Self {
        Self {
            client,
            cache_ttl,
            max_cache_size,
            cache: Mutex::new(HashMap::new()),
            connection_validated: AtomicBool::new(false),
        }
    }

    // ───────────────────────────── cache ─────────────────────────────

    fn cache(&self) -> std::sync::MutexGuard<'_, HashMap<String, CachedEvent>> {
        self.cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn cached(&self, uid: &str) -> Option<CalDavEvent> {
        let mut cache = self.cache();
        let expired = cache.get(uid).is_some_and(|c| c.cached_at.elapsed() > self.cache_ttl);
        if expired {
            cache.remove(uid);
            return None;
        }
        cache.get(uid).map(|c| c.event.clone())
    }

    fn remember(&self, event: &CalDavEvent) {
        let mut cache = self.cache();
        if cache.len() >= self.max_cache_size {
            let ttl = self.cache_ttl;
            cache.retain(|_, c| c.cached_at.elapsed() <= ttl);
        }
        cache.insert(event.uid.clone(), CachedEvent { event: event.clone(), cached_at: Instant::now() });
    }

    /// Drop the entry for `href` whatever UID it is keyed under: a handle
    /// carries only the href, and a stale entry would let a later UID-based
    /// update PUT to a deleted href, which CalDAV treats as a re-create.
    fn forget_href(&self, href: &str) {
        let target = normalize_icloud_url(href);
        self.cache().retain(|_, c| normalize_icloud_url(&c.event.href) != target);
    }

    pub fn clear_cache(&self) {
        self.cache().clear();
    }

    pub fn cache_size(&self) -> usize {
        self.cache().len()
    }

    // ───────────────────────────── reads ─────────────────────────────

    async fn ensure_connected(&self) -> ServiceResult<()> {
        if self.connection_validated.load(Ordering::Acquire) {
            return Ok(());
        }
        self.client.check_connection().await?;
        self.connection_validated.store(true, Ordering::Release);
        Ok(())
    }

    /// Every calendar the account can see. Validates the connection on first use.
    pub async fn list_calendars(&self) -> ServiceResult<Vec<CalendarInfo>> {
        self.ensure_connected().await?;
        let calendars = self.client.list_calendars().await?;
        Ok(calendars
            .into_iter()
            .map(|c| CalendarInfo {
                id: c.id,
                name: c.display_name,
                color: c.color,
                read_only: c.is_read_only,
                supported_components: c.supported_components,
            })
            .collect())
    }

    /// Events (one entry per occurrence of a series) overlapping the UTC days
    /// `start_date..=end_date`.
    ///
    /// Fails with 413 rather than truncating when a single series expands past
    /// the expander's bound or the result exceeds [`MAX_RETURNED_EVENTS`].
    pub async fn get_events(&self, calendar_id: &str, start_date: &str, end_date: &str) -> ServiceResult<Vec<EventInfo>> {
        let resources = self.client.get_events(calendar_id, start_date, end_date).await?;
        let range = DayRangeUtc::parse(start_date, end_date)
            .ok_or_else(|| ServiceError::new(400, "Invalid date format: expected YYYY-MM-DD"))?;
        let mut events = Vec::new();
        for resource in resources {
            self.remember(&resource);
            let occurrences = parse_occurrences(&resource.ical_data, range.start_instant(), range.last_instant())?;
            for occurrence in occurrences {
                let info = to_event_info(&occurrence, &resource);
                if !overlaps(&info, &range) {
                    continue;
                }
                events.push(info);
                if events.len() > MAX_RETURNED_EVENTS {
                    return Err(ServiceError::new(413, TOO_MANY_EVENTS_MESSAGE));
                }
            }
        }
        Ok(events)
    }

    /// An event from the UID cache.
    pub fn get_event_by_id(&self, event_id: &str) -> ServiceResult<EventInfo> {
        self.cached(event_id)
            .and_then(|cached| parse_events(&cached.ical_data).first().map(|p| to_event_info(p, &cached)))
            .ok_or_else(|| ServiceError::new(404, format!("Event not found: {event_id}")))
    }

    // ───────────────────────────── create ─────────────────────────────

    /// Create `event` in `calendar_id`, stamping CREATED and LAST-MODIFIED.
    pub async fn create_event(&self, calendar_id: &str, mut event: NewEvent) -> ServiceResult<EventInfo> {
        let calendars = self.client.list_calendars().await?;
        let calendar = calendars
            .iter()
            .find(|c| c.id == calendar_id)
            .ok_or_else(|| ServiceError::new(404, format!("Calendar not found: {calendar_id}")))?;
        if calendar.is_read_only {
            return Err(ServiceError::new(403, format!("Calendar is read-only: {}", calendar.display_name)));
        }
        let now = Utc::now();
        event.created_at = Some(now);
        event.last_modified = Some(now);
        let ics = build_event(&event).map_err(|e| ServiceError::new(400, format!("Invalid event data: {e}")))?;
        let created = self.client.create_event(calendar_id, &ics).await?;
        self.remember(&created);
        Ok(match parse_events(&created.ical_data).first() {
            Some(parsed) => to_event_info(parsed, &created),
            None => synthesized(&created, &event.summary, None),
        })
    }

    // ───────────────────────────── update ─────────────────────────────

    /// Update an event; only the fields set in `patch` change.
    ///
    /// `event_id` is a handle (resolved from the server, no cache needed) or a
    /// legacy bare UID (resolved from the cache). An occurrence handle requires
    /// a `scope`; see [`check_write_scope`].
    pub async fn update_event(&self, event_id: &str, patch: EventPatch, scope: Option<EventScope>) -> ServiceResult<EventInfo> {
        let handle = EventHandle::decode(event_id);
        let is_occurrence = handle.as_ref().is_some_and(EventHandle::is_occurrence_ref);
        check_write_scope(scope, is_occurrence, patch.rrule.as_deref(), patch.rdates.as_deref(), patch.exdates.as_deref())?;
        let recurrence_id = handle.and_then(|h| h.recurrence_id).unwrap_or_default();
        match scope {
            Some(EventScope::ThisAndFuture) => self.update_this_and_future(event_id, &recurrence_id, &patch).await,
            Some(EventScope::ThisOccurrence) => self.update_single_occurrence(event_id, &recurrence_id, &patch).await,
            Some(EventScope::AllEvents) | None => self.update_whole(event_id, &patch).await,
        }
    }

    async fn update_whole(&self, event_id: &str, patch: &EventPatch) -> ServiceResult<EventInfo> {
        let base = self.resolve_for_write(event_id).await?;
        // Whole-event writes always produce an EventInfo.
        self.with_stale_etag_retry(base, RetryableWrite::Whole(patch)).await.map(Option::unwrap_or_default)
    }

    /// Resolve `event_id` to the current resource to write against.
    ///
    /// A handle fetches the current body but keeps the handle's ETag for the
    /// first If-Match: that is the version the caller saw, so a concurrent edit
    /// since then trips a 412 (and one refetch-and-retry) instead of being
    /// silently overwritten.
    async fn resolve_for_write(&self, event_id: &str) -> ServiceResult<CalDavEvent> {
        if let Some(handle) = EventHandle::decode(event_id) {
            let mut current = self.client.get_event(&handle.href).await?;
            if handle.etag.is_some() {
                current.etag = handle.etag;
            }
            return Ok(current);
        }
        self.cached(event_id).ok_or_else(|| ServiceError::new(404, format!("Event not found: {event_id}")))
    }

    /// Run `write` against `target`; on a 412 refetch the resource once and run
    /// it again on the fresh body, which recomputes the payload so a concurrent
    /// change is not clobbered. At most one retry.
    async fn with_stale_etag_retry(&self, target: CalDavEvent, write: RetryableWrite<'_>) -> ServiceResult<Option<EventInfo>> {
        let href = target.href.clone();
        let first = self.write_once(target, &write).await;
        match first {
            Err(ref e) if e.code == 412 => match self.client.get_event(&href).await {
                Ok(fresh) => self.write_once(fresh, &write).await,
                Err(_) => first,
            },
            other => other,
        }
    }

    /// One attempt of `write` against `target`'s current body and ETag.
    async fn write_once(&self, target: CalDavEvent, write: &RetryableWrite<'_>) -> ServiceResult<Option<EventInfo>> {
        let ics = match write {
            RetryableWrite::Whole(patch) => patcher::patch(Some(&target.ical_data), &target.uid, patch).map_err(|e| match e {
                PatchError::Unparseable(_) => ServiceError::new(
                    422,
                    "Could not patch event: existing ICS is unparseable. This can happen with server-side line-ending quirks; try a full update (sending all fields) instead of a partial one.",
                ),
                other => patch_error(other),
            })?,
            RetryableWrite::Occurrence { recurrence_id, patch } => {
                if !is_live_occurrence(&target.ical_data, recurrence_id)? {
                    return Err(stale_occurrence_error());
                }
                patcher::patch_occurrence(Some(&target.ical_data), recurrence_id, patch).map_err(occurrence_error)?
            }
            RetryableWrite::Exdate { recurrence_id } => {
                if !is_live_occurrence(&target.ical_data, recurrence_id)? {
                    return Err(stale_occurrence_error());
                }
                patcher::exdate_occurrence(Some(&target.ical_data), recurrence_id).map_err(occurrence_error)?
            }
        };
        let updated = self.client.update_event(&target.href, &ics, target.etag.as_deref()).await?;
        self.remember(&updated);
        match write {
            RetryableWrite::Whole(patch) => {
                let parsed =
                    parse_events(&updated.ical_data).into_iter().next().or_else(|| parse_events(&ics).into_iter().next());
                Ok(Some(match parsed {
                    Some(parsed) => to_event_info(&parsed, &updated),
                    None => synthesized(&updated, patch.summary.as_deref().unwrap_or(&updated.uid), None),
                }))
            }
            RetryableWrite::Occurrence { recurrence_id, .. } => occurrence_event_info(&updated, &ics, recurrence_id).map(Some),
            RetryableWrite::Exdate { .. } => Ok(None),
        }
    }

    /// Edit one occurrence through a RECURRENCE-ID exception on the master.
    async fn update_single_occurrence(&self, event_id: &str, recurrence_id: &str, patch: &EventPatch) -> ServiceResult<EventInfo> {
        let base = self.resolve_for_write(event_id).await?;
        let write = RetryableWrite::Occurrence { recurrence_id, patch };
        // Occurrence writes always produce an EventInfo.
        self.with_stale_etag_retry(base, write).await.map(Option::unwrap_or_default)
    }

    /// Edit an occurrence and every later one: create the continuing series
    /// first, then truncate the master.
    ///
    /// Creating first keeps the operation recoverable: if the create fails,
    /// nothing was truncated; if the truncation then fails, the future
    /// occurrences exist twice (a visible duplicate) rather than not at all. On
    /// a 412 the truncation is recomputed on the fresh master; the new series
    /// is not recreated. At the first occurrence this is a whole-series edit.
    async fn update_this_and_future(&self, event_id: &str, recurrence_id: &str, patch: &EventPatch) -> ServiceResult<EventInfo> {
        let base = self.resolve_for_write(event_id).await?;
        if !is_live_occurrence(&base.ical_data, recurrence_id)? {
            return Err(stale_occurrence_error());
        }
        let split = match patcher::split_series(Some(&base.ical_data), recurrence_id, patch) {
            Ok(split) => split,
            Err(PatchError::FirstOccurrence) => {
                let whole = EventPatch { rrule: None, rdates: None, exdates: None, ..patch.clone() };
                return self.update_whole(event_id, &whole).await;
            }
            Err(other) => return Err(occurrence_error(other)),
        };

        let created = self.client.create_event(&calendar_id_for_href(&base.href), &split.new_series).await?;
        self.remember(&created);

        if let Err(e) = self.put_truncated(&base, &split.truncated_master, recurrence_id).await {
            return Err(ServiceError::new(
                e.code,
                format!(
                    "The continuing series for this and later occurrences was created, but truncating the original series failed: {}. Those occurrences now exist in both series (a duplicate). Re-list with get_events and delete the duplicate future occurrences from the original series.",
                    e.message
                ),
            ));
        }
        Ok(match parse_events(&created.ical_data).first() {
            Some(parsed) => to_event_info(parsed, &created),
            None => synthesized(&created, patch.summary.as_deref().unwrap_or(&created.uid), None),
        })
    }

    /// PUT the truncated master; on a 412 recompute the truncation on the
    /// fresh body and retry once.
    async fn put_truncated(&self, target: &CalDavEvent, truncated: &str, recurrence_id: &str) -> ServiceResult<()> {
        match self.client.update_event(&target.href, truncated, target.etag.as_deref()).await {
            Ok(updated) => {
                self.remember(&updated);
                Ok(())
            }
            Err(e) if e.code == 412 => {
                let fresh = self.client.get_event(&target.href).await.map_err(|_| ServiceError::from(e.clone()))?;
                let recomputed =
                    patcher::truncate_series(Some(&fresh.ical_data), recurrence_id).map_err(|_| ServiceError::from(e.clone()))?;
                let updated = self.client.update_event(&fresh.href, &recomputed, fresh.etag.as_deref()).await?;
                self.remember(&updated);
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    // ───────────────────────────── delete ─────────────────────────────

    /// Delete an event, or with an occurrence scope, cancel one occurrence or
    /// truncate the series. `event_id` resolves as in [`update_event`](Self::update_event).
    pub async fn delete_event(&self, event_id: &str, scope: Option<EventScope>) -> ServiceResult<()> {
        let handle = EventHandle::decode(event_id);
        let is_occurrence = handle.as_ref().is_some_and(EventHandle::is_occurrence_ref);
        check_write_scope(scope, is_occurrence, None, None, None)?;
        let recurrence_id = handle.as_ref().and_then(|h| h.recurrence_id.clone()).unwrap_or_default();
        match scope {
            Some(EventScope::ThisAndFuture) => self.delete_this_and_future(event_id, &recurrence_id).await,
            Some(EventScope::ThisOccurrence) => self.delete_single_occurrence(event_id, &recurrence_id).await,
            Some(EventScope::AllEvents) | None => self.delete_whole(event_id).await,
        }
    }

    /// DELETE the resource; on a 412 refetch the ETag and retry once.
    async fn delete_whole(&self, event_id: &str) -> ServiceResult<()> {
        let (href, etag) = match EventHandle::decode(event_id) {
            Some(handle) => (handle.href, handle.etag),
            None => {
                let cached =
                    self.cached(event_id).ok_or_else(|| ServiceError::new(404, format!("Event not found: {event_id}")))?;
                (cached.href, cached.etag)
            }
        };
        let result = match self.client.delete_event(&href, etag.as_deref()).await {
            Err(e) if e.code == 412 => match self.client.get_event(&href).await {
                Ok(fresh) => self.client.delete_event(&href, fresh.etag.as_deref()).await,
                Err(_) => Err(e),
            },
            other => other,
        };
        result?;
        self.forget_href(&href);
        Ok(())
    }

    /// Cancel one occurrence with an EXDATE on the master (a PUT, not a DELETE).
    async fn delete_single_occurrence(&self, event_id: &str, recurrence_id: &str) -> ServiceResult<()> {
        let base = self.resolve_for_write(event_id).await?;
        self.with_stale_etag_retry(base, RetryableWrite::Exdate { recurrence_id }).await.map(|_| ())
    }

    /// Cancel an occurrence and every later one by truncating the master (a
    /// PUT that creates nothing). At the first occurrence this deletes the
    /// whole resource. Not routed through the generic retry helper, whose
    /// retry would re-drive that delegated DELETE and exceed one retry.
    async fn delete_this_and_future(&self, event_id: &str, recurrence_id: &str) -> ServiceResult<()> {
        let mut target = self.resolve_for_write(event_id).await?;
        let mut allow_retry = true;
        loop {
            if !is_live_occurrence(&target.ical_data, recurrence_id)? {
                return Err(stale_occurrence_error());
            }
            let truncated = match patcher::truncate_series(Some(&target.ical_data), recurrence_id) {
                Ok(ics) => ics,
                Err(PatchError::FirstOccurrence) => return self.delete_whole(event_id).await,
                Err(other) => return Err(occurrence_error(other)),
            };
            match self.client.update_event(&target.href, &truncated, target.etag.as_deref()).await {
                Ok(updated) => {
                    self.remember(&updated);
                    return Ok(());
                }
                Err(e) if e.code == 412 && allow_retry => {
                    allow_retry = false;
                    target = self.client.get_event(&target.href).await.map_err(|_| ServiceError::from(e))?;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
}

/// A write that is recomputed from the current resource body on each attempt.
enum RetryableWrite<'a> {
    /// Patch the whole event (scope all_events, or a standalone event).
    Whole(&'a EventPatch),
    /// Patch one occurrence through a RECURRENCE-ID exception.
    Occurrence { recurrence_id: &'a str, patch: &'a EventPatch },
    /// Cancel one occurrence with an EXDATE.
    Exdate { recurrence_id: &'a str },
}

/// The fail-loud scope matrix, checked before any network call. Rejects:
/// - an occurrence scope carrying rrule/rdates/exdates (those edit the series);
/// - an occurrence scope on a reference that is not an occurrence handle;
/// - no scope on an occurrence handle, so one instance never silently widens
///   into a whole-series change.
fn check_write_scope(
    scope: Option<EventScope>,
    is_occurrence: bool,
    rrule: Option<&str>,
    rdates: Option<&[String]>,
    exdates: Option<&[String]>,
) -> ServiceResult<()> {
    match scope {
        Some(scope) if scope.is_occurrence_level() => {
            if rrule.is_some() || rdates.is_some() || exdates.is_some() {
                return Err(ServiceError::new(
                    400,
                    format!(
                        "Cannot change series-level recurrence fields (rrule, rdates, exdates) with scope {}; that scope acts on occurrences. Use scope all_events to edit the series.",
                        scope.token()
                    ),
                ));
            }
            if !is_occurrence {
                return Err(ServiceError::new(
                    400,
                    format!(
                        "scope {} needs an occurrence reference (one instance of a recurring series, as returned by get_events); this reference is a standalone event or a whole series.",
                        scope.token()
                    ),
                ));
            }
            Ok(())
        }
        None if is_occurrence => Err(ServiceError::new(
            400,
            "This reference is one occurrence of a recurring series. Set scope to this_occurrence, this_and_future, or all_events so the change does not silently affect the whole series.",
        )),
        _ => Ok(()),
    }
}

/// True when `event` overlaps the half-open UTC window (RFC 4791 §9.9:
/// `start < window_end AND end > window_start`).
///
/// iCloud returns all-day events whose exclusive DTEND only touches the query
/// start; those belong to the previous day and are dropped here. Events whose
/// dates cannot be read are kept: dropping one the caller needs is worse.
fn overlaps(event: &EventInfo, range: &DayRangeUtc) -> bool {
    let parse_day = |d: &Option<String>| d.as_deref().and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
    let parse_instant = |t: &Option<String>| {
        t.as_deref().and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|d| d.with_timezone(&Utc))
    };
    if event.is_all_day {
        let Some(start) = parse_day(&event.start_date) else {
            return true;
        };
        // end_date is inclusive; shift it back to a half-open bound.
        let end_exclusive = parse_day(&event.end_date).unwrap_or(start) + TimeDelta::days(1);
        return start < range.end_day_exclusive() && end_exclusive > range.start_day;
    }
    let Some(start) = parse_instant(&event.start_time) else {
        return true;
    };
    let end = parse_instant(&event.end_time).unwrap_or(start);
    // A zero-length event still occupies its start instant.
    let end_exclusive = if end > start { end } else { start + TimeDelta::milliseconds(1) };
    start < range.end_instant_exclusive() && end_exclusive > range.start_instant()
}

/// True when `recurrence_id` names a live occurrence of the series in `ics`.
fn is_live_occurrence(ics: &str, recurrence_id: &str) -> ServiceResult<bool> {
    Ok(find_occurrence(ics, recurrence_id)?.is_some())
}

/// The occurrence whose RECURRENCE-ID equals `recurrence_id`, or `None` when
/// that instant is not live (past COUNT/UNTIL, EXDATE'd, or never generated).
/// The ±1 day window around its day absorbs any zone shift.
fn find_occurrence(ics: &str, recurrence_id: &str) -> ServiceResult<Option<ParsedEvent>> {
    let Some(day) = recurrence_id.get(..8).and_then(|d| NaiveDate::parse_from_str(d, "%Y%m%d").ok()) else {
        return Ok(None);
    };
    let start = (day - TimeDelta::days(1)).and_time(NaiveTime::MIN).and_utc();
    let end = (day + TimeDelta::days(2)).and_time(NaiveTime::MIN).and_utc();
    let occurrences = parse_occurrences(ics, start, end)?;
    Ok(occurrences.into_iter().find(|o| o.recurrence_id.as_deref() == Some(recurrence_id)))
}

/// The edited occurrence as written, preferring the server echo. Never the
/// master: its fields and `evt1_` handle would misreport the edit.
fn occurrence_event_info(updated: &CalDavEvent, sent_ics: &str, recurrence_id: &str) -> ServiceResult<EventInfo> {
    let occurrence = match find_occurrence(&updated.ical_data, recurrence_id)? {
        Some(o) => Some(o),
        None => find_occurrence(sent_ics, recurrence_id)?,
    };
    Ok(match occurrence {
        Some(o) => to_event_info(&o, updated),
        None => synthesized(updated, "", Some(recurrence_id)),
    })
}

/// The calendar owning an event href: the collection segment of its parent path.
fn calendar_id_for_href(href: &str) -> String {
    let parent = href.rfind('/').map_or(href, |i| &href[..i]);
    extract_calendar_id_from_href(parent)
}

fn to_event_info(parsed: &ParsedEvent, resource: &CalDavEvent) -> EventInfo {
    EventInfo {
        uid: parsed.uid.clone(),
        href: resource.href.clone(),
        etag: resource.etag.clone(),
        handle: Some(handle::encode(&resource.href, resource.etag.as_deref(), parsed.recurrence_id.as_deref())),
        summary: parsed.summary.clone(),
        description: parsed.description.clone(),
        location: parsed.location.clone(),
        is_all_day: parsed.is_all_day,
        start_time: parsed.start_time.clone(),
        end_time: parsed.end_time.clone(),
        start_date: parsed.start_date.clone(),
        end_date: parsed.end_date.clone(),
        rrule: parsed.rrule.clone(),
        recurrence_id: parsed.recurrence_id.clone(),
        status: parsed.status.clone(),
        url: parsed.url.clone(),
        categories: parsed.categories.clone(),
        priority: parsed.priority,
        organizer: parsed.organizer.clone(),
        attendee_count: parsed.attendee_count,
        alarms: parsed.alarms.clone(),
    }
}

/// A minimal [`EventInfo`] for a written resource whose body yields no VEVENT
/// ("should not happen", but never return nothing).
fn synthesized(resource: &CalDavEvent, summary: &str, recurrence_id: Option<&str>) -> EventInfo {
    EventInfo {
        uid: resource.uid.clone(),
        href: resource.href.clone(),
        etag: resource.etag.clone(),
        handle: Some(handle::encode(&resource.href, resource.etag.as_deref(), recurrence_id)),
        summary: summary.to_string(),
        recurrence_id: recurrence_id.map(str::to_string),
        ..EventInfo::default()
    }
}

fn stale_occurrence_error() -> ServiceError {
    ServiceError::new(
        409,
        "That occurrence is no longer part of the series (already changed, cancelled, or never existed). Re-list with get_events for a current reference.",
    )
}

fn occurrence_error(error: PatchError) -> ServiceError {
    match error {
        PatchError::NotARecurringSeries => ServiceError::new(
            400,
            "This event is not a recurring series, so a single-occurrence scope does not apply. Use scope all_events (or omit scope) to edit it.",
        ),
        PatchError::Unparseable(_) => {
            ServiceError::new(422, "Could not edit the occurrence: the existing event data could not be parsed.")
        }
        other => patch_error(other),
    }
}

fn patch_error(error: PatchError) -> ServiceError {
    match error {
        PatchError::ExpansionLimit(limit) => limit.into(),
        PatchError::InvalidInput(detail) => ServiceError::new(400, format!("Invalid event data: {detail}")),
        other => ServiceError::new(422, other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_matrix() {
        let rdates = vec!["2025-01-01".to_string()];
        assert!(check_write_scope(None, false, None, None, None).is_ok());
        assert!(check_write_scope(Some(EventScope::AllEvents), true, Some("FREQ=DAILY"), None, None).is_ok());
        assert!(check_write_scope(Some(EventScope::ThisOccurrence), true, None, None, None).is_ok());
        assert_eq!(check_write_scope(None, true, None, None, None).unwrap_err().code, 400);
        assert!(
            check_write_scope(Some(EventScope::ThisOccurrence), false, None, None, None)
                .unwrap_err()
                .message
                .contains("needs an occurrence reference")
        );
        assert!(
            check_write_scope(Some(EventScope::ThisAndFuture), true, None, Some(&rdates), None)
                .unwrap_err()
                .message
                .contains("this_and_future")
        );
    }

    #[test]
    fn calendar_id_from_event_href() {
        assert_eq!(calendar_id_for_href("/123/calendars/work/abc.ics"), "work");
        assert_eq!(calendar_id_for_href("https://caldav.icloud.com/123/calendars/home/x.ics"), "home");
    }

    #[test]
    fn overlap_drops_all_day_event_ending_at_window_start() {
        let range = DayRangeUtc::parse("2025-07-27", "2025-07-27").unwrap();
        let ended = EventInfo {
            is_all_day: true,
            start_date: Some("2025-07-26".into()),
            end_date: Some("2025-07-26".into()),
            ..EventInfo::default()
        };
        assert!(!overlaps(&ended, &range));
        let spanning = EventInfo { end_date: Some("2025-07-27".into()), ..ended.clone() };
        assert!(overlaps(&spanning, &range));
        let unreadable = EventInfo { start_date: Some("garbage".into()), ..ended };
        assert!(overlaps(&unreadable, &range));
    }

    #[test]
    fn overlap_for_timed_and_zero_length_events() {
        let range = DayRangeUtc::parse("2025-01-15", "2025-01-15").unwrap();
        let timed = |s: &str, e: &str| EventInfo {
            start_time: Some(s.into()),
            end_time: Some(e.into()),
            ..EventInfo::default()
        };
        assert!(overlaps(&timed("2025-01-14T23:00:00Z", "2025-01-15T01:00:00Z"), &range));
        assert!(!overlaps(&timed("2025-01-14T23:00:00Z", "2025-01-15T00:00:00Z"), &range));
        assert!(overlaps(&timed("2025-01-15T00:00:00Z", "2025-01-15T00:00:00Z"), &range));
        assert!(!overlaps(&timed("2025-01-16T00:00:00Z", "2025-01-16T01:00:00Z"), &range));
    }
}
