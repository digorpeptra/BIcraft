//! The Web, OData and SharePoint connectors of the session (lot 11): `allow_connectors: ["web"]`
//! registers `Web.Contents`, `["odata"]` registers `OData.Feed`, `["sharepoint"]` registers
//! `SharePoint.Files`, `SharePoint.Contents` and `SharePoint.Tables` (`refresh::connectors`).
//! Each request carries the credential the session holds for its URL (`source.credentials {kind:
//! web}`, anonymous otherwise). All need network access (refused in server mode unless the server
//! allows it). See docs/formats/odata.md, docs/formats/sharepoint.md and
//! docs/formats/data-sources.md, *Web*.

use std::sync::Arc;

use bicraft_webdata::{ODataConnector, Settings, SharePointConnector, WebConnector};

use crate::Session;

pub(crate) fn web(s: &Session) -> Result<Arc<WebConnector>, String> {
    crate::sandbox::require_network(s, "the Web connector")?;
    Ok(Arc::new(WebConnector::with_credentials(
        Settings::default(),
        Some(s.web_credentials.lookup()),
    )))
}

pub(crate) fn odata(s: &Session) -> Result<Arc<ODataConnector>, String> {
    crate::sandbox::require_network(s, "the OData connector")?;
    Ok(Arc::new(ODataConnector::with_credentials(
        Settings::default(),
        Some(s.web_credentials.lookup()),
    )))
}

pub(crate) fn sharepoint(s: &Session) -> Result<Arc<SharePointConnector>, String> {
    crate::sandbox::require_network(s, "the SharePoint connector")?;
    Ok(Arc::new(SharePointConnector::new(
        Settings::default(),
        Some(s.web_credentials.lookup()),
    )))
}
