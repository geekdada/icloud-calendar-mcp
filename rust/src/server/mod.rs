//! The MCP server: tools, the `calendar://calendars` resource, prompts, and
//! the logging bridge, wired into rmcp's [`ServerHandler`].

// LoggingLevel and the logging notification are deprecated by SEP-2577 but are
// still the wire types for `logging/setLevel` and `notifications/message`.
#![allow(deprecated)]

pub mod args;
pub mod prompts;
pub mod tools;

use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};

use futures::FutureExt;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ErrorCode, ErrorData, GetPromptRequestParams,
    GetPromptResponse, Implementation, ListPromptsResult, ListResourcesResult, ListToolsResult, LoggingLevel,
    LoggingMessageNotificationParam, PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse,
    ReadResourceResult, Resource, ResourceContents, ServerCapabilities, ServerConfig, SetLevelRequestParams,
};
use rmcp::service::{NotificationContext, Peer, RequestContext, RoleServer};
use rmcp::{ServerHandler, model::InitializeRequestParams, model::InitializeResult};
use serde_json::{Map, Value, json};

use crate::error;
use crate::logging::{LogSink, McpLogger};
use crate::ratelimit::RateLimiter;
use crate::service::CalendarService;

/// URI of the calendar list resource.
pub const CALENDARS_URI: &str = "calendar://calendars";

/// Sends log notifications to whichever client peer the server last heard from.
#[derive(Default)]
pub struct PeerLogSink {
    peer: Mutex<Option<Peer<RoleServer>>>,
}

impl PeerLogSink {
    fn set_peer(&self, peer: &Peer<RoleServer>) {
        *self.peer.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(peer.clone());
    }
}

impl LogSink for PeerLogSink {
    fn send(&self, level: LoggingLevel, logger: &str, data: Value) {
        let peer = self.peer.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        let (Some(peer), Ok(runtime)) = (peer, tokio::runtime::Handle::try_current()) else {
            return;
        };
        let param = LoggingMessageNotificationParam::new(level, data).with_logger(logger);
        // Fire and forget: a failed log message must never fail the tool call.
        runtime.spawn(async move {
            if let Err(e) = peer.notify_logging_message(param).await {
                eprintln!("Failed to send log message: {e}");
            }
        });
    }
}

/// The iCloud Calendar MCP server.
pub struct CalendarServer {
    /// `None` when credentials are not configured: every tool then reports an
    /// authentication error instead of the server refusing to start.
    pub(crate) service: Option<Arc<CalendarService>>,
    pub(crate) rate_limiter: Arc<RateLimiter>,
    pub(crate) logger: Arc<McpLogger>,
    peer_sink: Option<Arc<PeerLogSink>>,
}

impl CalendarServer {
    /// A server that logs to its connected client, with the default limits
    /// (60 reads and 20 writes per minute).
    pub fn new(service: Option<Arc<CalendarService>>) -> Self {
        let sink = Arc::new(PeerLogSink::default());
        let mut server = Self::with_parts(service, Arc::new(RateLimiter::new(60, 20, 60_000)), Arc::new(McpLogger::new(sink.clone())));
        server.peer_sink = Some(sink);
        server
    }

    /// A server over explicit parts, for tests.
    pub fn with_parts(service: Option<Arc<CalendarService>>, rate_limiter: Arc<RateLimiter>, logger: Arc<McpLogger>) -> Self {
        Self { service, rate_limiter, logger, peer_sink: None }
    }

    pub fn logger(&self) -> &Arc<McpLogger> {
        &self.logger
    }

    fn remember_peer(&self, peer: &Peer<RoleServer>) {
        if let Some(sink) = &self.peer_sink {
            sink.set_peer(peer);
        }
    }

    /// Run the tool `name`. Unknown tools are a protocol error; everything a
    /// tool itself can hit, including a panic, becomes an error result.
    pub async fn call_tool_by_name(&self, name: &str, arguments: Option<Map<String, Value>>) -> Result<CallToolResult, ErrorData> {
        let args = arguments.unwrap_or_default();
        let call = async {
            match name {
                "list_calendars" => Ok(self.list_calendars().await),
                "get_events" => Ok(self.get_events(&args).await),
                "create_event" => Ok(self.create_event(&args).await),
                "update_event" => Ok(self.update_event(&args).await),
                "delete_event" => Ok(self.delete_event(&args).await),
                _ => Err(ErrorData::invalid_params(format!("Tool not found: {name}"), None)),
            }
        };
        match AssertUnwindSafe(call).catch_unwind().await {
            Ok(result) => result,
            Err(_) => {
                eprintln!("Error in {name}: panic");
                Ok(error::internal_error("panic"))
            }
        }
    }

    /// The calendar list resource.
    pub fn resources() -> Vec<Resource> {
        vec![
            Resource::new(CALENDARS_URI, "iCloud Calendars")
                .with_title("iCloud Calendars")
                .with_description("List of all calendars in the connected iCloud account")
                .with_mime_type("application/json"),
        ]
    }

    /// Read `uri`. Failures are JSON-RPC errors: -32000 rate limited, -32001
    /// not configured, -32002 service failure.
    pub async fn read_resource_uri(&self, uri: &str) -> Result<ReadResourceResult, ErrorData> {
        if uri != CALENDARS_URI {
            return Err(ErrorData::resource_not_found(format!("Resource not found: {uri}"), None));
        }
        let rate = self.rate_limiter.acquire_read_with_status();
        if !rate.allowed {
            return Err(ErrorData::new(
                ErrorCode(-32000),
                format!("Rate limit exceeded. Retry after {}ms", rate.retry_after_ms),
                None,
            ));
        }
        let service = self.service.as_ref().ok_or_else(|| ErrorData::new(ErrorCode(-32001), "iCloud credentials not configured", None))?;
        let calendars = service
            .list_calendars()
            .await
            .map_err(|e| ErrorData::new(ErrorCode(-32002), format!("Calendar service error: {}", e.message), None))?;
        let list: Vec<Value> = calendars
            .iter()
            .map(|c| {
                let mut entry = Map::new();
                entry.insert("id".into(), json!(c.id));
                entry.insert("name".into(), json!(c.name));
                entry.insert("uri".into(), json!(format!("{CALENDARS_URI}/{}", c.id)));
                if let Some(color) = &c.color {
                    entry.insert("color".into(), json!(color));
                }
                entry.insert("readOnly".into(), json!(c.read_only));
                Value::Object(entry)
            })
            .collect();
        let text = json!({ "calendars": list }).to_string();
        let contents = ResourceContents::text(text, CALENDARS_URI).with_mime_type("application/json");
        Ok(ReadResourceResult::new(vec![contents]))
    }
}

impl ServerHandler for CalendarServer {
    fn get_info(&self) -> ServerConfig {
        let capabilities = ServerCapabilities::builder()
            .enable_logging()
            .enable_prompts()
            .enable_resources()
            .enable_resources_list_changed()
            .enable_tools()
            .enable_tool_list_changed()
            .build();
        ServerConfig::new(capabilities)
            .with_server_info(Implementation::new("icloud-calendar-mcp", env!("CARGO_PKG_VERSION")))
    }

    async fn initialize(&self, request: InitializeRequestParams, context: RequestContext<RoleServer>) -> Result<InitializeResult, ErrorData> {
        context.peer.set_peer_info(request.clone());
        self.remember_peer(&context.peer);
        self.negotiate_initialize(&request)
    }

    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        self.remember_peer(&context.peer);
    }

    async fn set_level(&self, request: SetLevelRequestParams, context: RequestContext<RoleServer>) -> Result<(), ErrorData> {
        self.remember_peer(&context.peer);
        self.logger.set_level(request.level);
        Ok(())
    }

    async fn list_tools(&self, _request: Option<PaginatedRequestParams>, _context: RequestContext<RoleServer>) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools::definitions()))
    }

    async fn call_tool(&self, request: CallToolRequestParams, context: RequestContext<RoleServer>) -> Result<CallToolResponse, ErrorData> {
        self.remember_peer(&context.peer);
        self.call_tool_by_name(&request.name, request.arguments).await.map(CallToolResponse::from)
    }

    async fn list_resources(&self, _request: Option<PaginatedRequestParams>, _context: RequestContext<RoleServer>) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::with_all_items(Self::resources()))
    }

    async fn read_resource(&self, request: ReadResourceRequestParams, context: RequestContext<RoleServer>) -> Result<ReadResourceResponse, ErrorData> {
        self.remember_peer(&context.peer);
        self.read_resource_uri(&request.uri).await.map(ReadResourceResponse::from)
    }

    async fn list_prompts(&self, _request: Option<PaginatedRequestParams>, _context: RequestContext<RoleServer>) -> Result<ListPromptsResult, ErrorData> {
        Ok(ListPromptsResult::with_all_items(prompts::definitions()))
    }

    async fn get_prompt(&self, request: GetPromptRequestParams, _context: RequestContext<RoleServer>) -> Result<GetPromptResponse, ErrorData> {
        prompts::render(&request.name, request.arguments.as_ref())
            .map(GetPromptResponse::from)
            .ok_or_else(|| ErrorData::invalid_params(format!("Prompt not found: {}", request.name), None))
    }
}
