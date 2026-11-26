# SharePoint connectors, web credentials and privacy levels (as implemented)

Status: implemented 2026-11-20 (M11 lot 11). `SharePoint.Files`, `SharePoint.Contents` and
`SharePoint.Tables` in BIcraft's Power Query evaluator over the SharePoint REST API
(`crates/webdata/src/sharepoint.rs`); credentials per URL for `Web.Contents`, `OData.Feed` and
SharePoint (`crates/webdata/src/auth.rs`, `source.credentials {kind: "web"}`); privacy levels per
data source, recorded and shown (`source.privacy`, `source.settings`,
`crates/engine/src/source_settings.rs`). Opt-in like the other connectors: SharePoint runs only
for `allow_connectors: ["sharepoint"]`.

**Sources.** Public documentation only: the Power Query M reference on Microsoft Learn
(`SharePoint.Files`, `SharePoint.Contents`, `SharePoint.Tables`, `Web.Contents` and its
`ApiKeyName` option), the Power Query connector articles *SharePoint folder*, *SharePoint list*
and *SharePoint Online list* (the site URL, the API version and view mode options, the
`Implementation = "2.0"` option), *Authentication in desktop apps* (the authentication kinds, the
level a credential applies to, Data source settings, Edit Permissions, Clear Permissions), the
Power BI article *Power BI Desktop privacy levels* and the Power Query article *Behind the scenes
of the Data Privacy Firewall* (the four levels and what they mean), and the SharePoint REST API
reference on Microsoft Learn (`_api/web/lists`, `RootFolder`, `GetFolderByServerRelativeUrl`,
`GetFileByServerRelativeUrl(…)/$value`, `items`, `DefaultView/ViewFields`, the
`odata=nometadata` JSON format and its next links). The tests run against an in-process HTTP
server of our own (`crates/webdata/src/fake.rs`, `start_sharepoint`); no Desktop capture and no
real SharePoint site were used, and the M shapes were not compared with a file Desktop saved.

## SharePoint functions

| Function | Answer |
|---|---|
| `SharePoint.Files(url, [ApiVersion])` | one row per file of every document library of the site, folders flattened |
| `SharePoint.Contents(url, [ApiVersion])` | the site's document libraries and root folders as a navigation table; a folder's `Content` is its own table, read when used |
| `SharePoint.Tables(url, [ApiVersion, Implementation, ViewMode, DisableAppendNoteColumns])` | the site's lists that are not hidden: `Id`, `Title`, `Items` (the list's items, read when used) |

File rows have Learn's columns: `Content` (binary, downloaded only when read, capped like a
`Web.Contents` body), `Name`, `Extension`, `Date accessed` (null), `Date modified`,
`Date created`, `Attributes` (a record with `Size`, `Directory`, …) and `Folder Path` (the
absolute URL of the folder, ending with `/`).

Options as Learn lists them: `ApiVersion` 14, 15 or `"Auto"` (accepted; BIcraft calls the
version 15 `_api` endpoints in every case), `Implementation` `"2.0"` or null, `ViewMode`
`"All"` or `"Default"` (only with `Implementation = "2.0"`; `"Default"` keeps the columns of the
list's default view through `DefaultView/ViewFields`), `DisableAppendNoteColumns` (accepted, no
effect). API version 14 with implementation 2.0 is refused, as Learn says it is not supported. A
site URL with a query or a fragment is refused (Learn: the root site address, without a page or
query).

### Requests

Every request asks for `Accept: application/json;odata=nometadata`. Collections are read from
`value` (v3 JSON light) or `d.results` (verbose JSON), and paged by `odata.nextLink`,
`@odata.nextLink` or `__next`, followed only on the site's origin, up to the connector's page
limit (10 000) and the evaluation's row cap. Folders are read with
`RootFolder?$expand=Folders,Files` and `GetFolderByServerRelativeUrl('…')?$expand=Folders,Files`;
document libraries are the lists with `BaseTemplate` 101 that are not hidden; list items with
`lists(guid'…')/items?$top=5000`. Nested folders deeper than 64 levels and JSON values nested
deeper than 16 levels stop the read with an error.

### Navigator

Get data › SharePoint folder lists the site (`Combine` and `Load` items, like a local folder,
[data-sources.md](data-sources.md) *Navigator*); its query is
`SharePoint.Files("<site>", [ApiVersion = 15])`. Get data › SharePoint list lists the site's
lists by `Title`, each with
`Source = SharePoint.Tables("<site>", [Implementation = "2.0", ViewMode = "All"]),
#"<id>" = Source{[Id="<id>"]}[Items]`.

### Errors

| Case | Error |
|---|---|
| 401, 403, 407 | `Error::Connector` (credentials): set a bearer token for the site with `source.credentials` |
| another status outside 200–299 | catchable `DataSource.Error` `SharePoint: <url> answered <status> <reason>: <message>` |
| not JSON, not a collection, a next link to another site | `Error::Connector` |
| more items, pages, rows, columns or nesting than the caps | `Error::Connector` (limit) |

## Web credentials

`source.credentials {kind: "web", url, authentication, …}` keeps a credential for an http(s)
URL in the session's memory only: never in a file, the recovery journal, an answer, the UI's
sent history or a log. Learn's kinds:

| `authentication` | Sent |
|---|---|
| `anonymous` | nothing |
| `basic` (`user`, `password`) | `Authorization: Basic …` |
| `key` (`key`, `header?`) | Web API: the key as the query parameter named by `Web.Contents`'s `ApiKeyName`, or in the header `header` |
| `bearer` / `organizational` (`token`) | `Authorization: Bearer <token>` (an access token obtained elsewhere; BIcraft has no sign-in) |

`windows` is refused. As Learn describes the level a credential applies to, the URL given is a
prefix: a request uses the credential of the longest stored prefix that covers its URL (same
scheme, host and port, whole path segments). Redirects are followed by BIcraft itself (at most
the client's limit): each hop looks up its own credential, and `Authorization` and `Cookie`
headers are dropped when a hop changes origin. `Web.Contents` with `ApiKeyName` and no key
credential for its URL is a credentials error before anything is sent. Error texts never contain
the secret (the URL with a key is scrubbed).

## Privacy levels

`source.privacy {kind, source, level}` records one of Learn's four levels for a data source:
`none`, `public`, `organizational`, `private` (`clear: true` forgets it). `source.settings` lists
the model's data sources (the literal URL, server or path arguments of its connector calls),
each with its kind, the functions that use it, the kind of credential stored (never the secret)
and the level, plus the stored credentials and levels of sources the model no longer uses, and
`firewall: false`.

Learn does not say precisely where Desktop stores the levels (it shows them under Data source
settings, with global permissions and permissions for the current file), so BIcraft keeps them in
the session only, like credentials. **The firewall is not enforced**: BIcraft never refuses or
reorders a query because of the levels, which is what Desktop does with *Ignore the Privacy
Levels* selected. See docs/gaps.md, G69.

## Not implemented

OAuth sign-in (an organizational account's token must be supplied), Windows authentication, the
Data Privacy Firewall, levels saved in a file, the version 14 (`_vti_bin/ListData.svc`)
endpoints, list item columns that are records (lookups, people) loaded into a model table (they
can be previewed, selected away or expanded), `Xml.Tables`, `Web.Page`. See docs/gaps.md, G69.
