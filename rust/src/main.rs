//! iCloud Calendar MCP server over STDIO.
//!
//! Credentials come from `ICLOUD_USERNAME` / `ICLOUD_PASSWORD` (an app-specific
//! password). Without them the server still starts and every tool reports an
//! authentication error. stdout carries JSON-RPC only; diagnostics go to stderr.

use std::sync::Arc;

use icloud_calendar_mcp::caldav::{CalDavCredentials, HttpCalDavClient};
use icloud_calendar_mcp::credentials;
use icloud_calendar_mcp::server::CalendarServer;
use icloud_calendar_mcp::service::CalendarService;
use rmcp::ServiceExt;

const ICLOUD_CALDAV_URL: &str = "https://caldav.icloud.com";

#[tokio::main]
async fn main() {
    if !credentials::is_configured() {
        eprintln!(
            "Warning: iCloud credentials not configured. Set ICLOUD_USERNAME and ICLOUD_PASSWORD environment variables."
        );
    }
    let server = CalendarServer::new(create_calendar_service());
    let running = match server.serve(rmcp::transport::stdio()).await {
        Ok(running) => running,
        Err(e) => {
            eprintln!("Failed to start MCP server: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = running.waiting().await {
        eprintln!("MCP server stopped with an error: {e}");
        std::process::exit(1);
    }
}

/// The calendar service, or `None` when credentials are absent or invalid.
fn create_calendar_service() -> Option<Arc<CalendarService>> {
    let creds = match credentials::load_from_environment() {
        Ok(Some(creds)) => creds,
        Ok(None) => return None,
        Err(e) => {
            eprintln!("Failed to initialize CalendarService: {}", e.0);
            return None;
        }
    };
    let caldav_credentials = CalDavCredentials::new(creds.username.clone(), creds.password.clone());
    match HttpCalDavClient::new(ICLOUD_CALDAV_URL, caldav_credentials) {
        Ok(client) => Some(Arc::new(CalendarService::new(Arc::new(client)))),
        Err(e) => {
            eprintln!("Failed to initialize CalendarService: {}", e.message);
            None
        }
    }
}
