//! L3: the Web and OData connectors of bicraft's Power Query evaluator.
//!
//! Registered in [`bicraft_data::SourceOptions::connectors`] only when the caller explicitly
//! allows them (`allow_connectors: ["web"]`, `["odata"]`, `["sharepoint"]`), they run M functions
//! Power BI Desktop writes over HTTP or HTTPS (rustls with the bundled Mozilla roots), with the
//! credential stored for each URL ([`auth`]; anonymous when none is):
//!
//! - `Web.Contents(url, options)` ([`WebConnector`], [`web`]): the bytes of a resource, which the
//!   evaluator's own readers then read (`Json.Document`, `Csv.Document`, `Excel.Workbook`, …);
//! - `OData.Feed(serviceUri, headers, options)` ([`ODataConnector`], [`odata`]): the navigation
//!   table of a service's entity sets, each a remote table whose column selection, row filter,
//!   sort and row limit fold into `$select`, `$filter`, `$orderby` and `$top`;
//! - `SharePoint.Files`, `SharePoint.Contents`, `SharePoint.Tables` ([`SharePointConnector`],
//!   [`sharepoint`]): a site's files, folders and lists over its REST API.
//!
//! Every response is read with a size cap, every request has a deadline, and a connector keeps
//! the responses it already has (a query that reads a URL twice sends one request). Network
//! access goes only to the URLs the query names (and, for OData, the `$metadata` and next pages
//! on the feed's origin; for SharePoint, the site's `_api`). Credentials are the caller's
//! ([`auth::Lookup`]), held in memory; a service that refuses the request is a credentials
//! error. See docs/formats/odata.md, docs/formats/sharepoint.md and docs/formats/data-sources.md,
//! *Web*.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod auth;
pub mod csdl;
pub mod http;
pub mod odata;
pub mod sharepoint;
pub mod web;

#[cfg(feature = "fake-server")]
pub mod fake;

use std::sync::Arc;
use std::time::Duration;

use bicraft_data::connect::{Arg, ConnectError, Connector, Source};

/// How the connectors reach the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// The deadline of one request (Power Query's `Web.Contents` default: 100 s), unless the
    /// query's `Timeout` option says otherwise.
    pub timeout: Duration,
    /// Largest response body read, in bytes.
    pub max_response_bytes: u64,
    /// Most pages of one OData entity set followed.
    pub max_pages: usize,
    /// Most redirects followed.
    pub max_redirects: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(100),
            max_response_bytes: 256 << 20,
            max_pages: 10_000,
            max_redirects: 10,
        }
    }
}

/// `Web.Contents` (`allow_connectors: ["web"]`).
pub struct WebConnector {
    client: http::Client,
}

impl WebConnector {
    pub fn new(settings: Settings) -> Self {
        Self::with_credentials(settings, None)
    }

    /// A connector that sends the credential `credentials` gives each URL.
    pub fn with_credentials(settings: Settings, credentials: Option<auth::Lookup>) -> Self {
        Self {
            client: http::Client::with_credentials(settings, credentials),
        }
    }
}

impl Connector for WebConnector {
    fn name(&self) -> &'static str {
        "web"
    }

    fn handles(&self, function: &str) -> bool {
        function == "Web.Contents"
    }

    fn open(&self, function: &str, _: &[Arg]) -> Result<Box<dyn Source>, ConnectError> {
        Err(http::unsupported(format!("`{function}` returns a binary value")))
    }

    fn returns_binary(&self, function: &str) -> bool {
        function == "Web.Contents"
    }

    fn contents(&self, _: &str, args: &[Arg], max_bytes: usize) -> Result<Vec<u8>, ConnectError> {
        let call = web::call(args)?;
        web::run(&self.client, &call, max_bytes)
    }
}

/// `OData.Feed` (`allow_connectors: ["odata"]`).
pub struct ODataConnector {
    client: Arc<http::Client>,
}

impl ODataConnector {
    pub fn new(settings: Settings) -> Self {
        Self::with_credentials(settings, None)
    }

    pub fn with_credentials(settings: Settings, credentials: Option<auth::Lookup>) -> Self {
        Self {
            client: Arc::new(http::Client::with_credentials(settings, credentials)),
        }
    }
}

impl Connector for ODataConnector {
    fn name(&self) -> &'static str {
        "odata"
    }

    fn handles(&self, function: &str) -> bool {
        function == "OData.Feed"
    }

    fn open(&self, _: &str, args: &[Arg]) -> Result<Box<dyn Source>, ConnectError> {
        let opts = odata::options(args)?;
        Ok(Box::new(odata::Feed::open(Arc::clone(&self.client), opts)?))
    }
}

/// `SharePoint.Files`, `SharePoint.Contents`, `SharePoint.Tables` (`allow_connectors:
/// ["sharepoint"]`).
pub struct SharePointConnector {
    client: Arc<http::Client>,
}

impl SharePointConnector {
    pub fn new(settings: Settings, credentials: Option<auth::Lookup>) -> Self {
        Self {
            client: Arc::new(http::Client::with_credentials(settings, credentials)),
        }
    }
}

impl Connector for SharePointConnector {
    fn name(&self) -> &'static str {
        "sharepoint"
    }

    fn handles(&self, function: &str) -> bool {
        sharepoint::Function::of(function).is_some()
    }

    fn open(&self, function: &str, args: &[Arg]) -> Result<Box<dyn Source>, ConnectError> {
        let opts = sharepoint::options(function, args)?;
        Ok(Box::new(sharepoint::Call::open(Arc::clone(&self.client), opts)))
    }
}
