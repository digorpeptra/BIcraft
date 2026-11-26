# Web and OData connectors (as implemented in `crates/webdata`)

Status: implemented 2026-11-19 (M11 lot 11). `Web.Contents(url, options)` and
`OData.Feed(serviceUri, headers, options)` in BIcraft's Power Query evaluator, over HTTP or HTTPS,
anonymously or with a credential set per URL (2026-11-20, [sharepoint.md](sharepoint.md) *Web
credentials*). Opt-in like the other connectors ([data-sources.md](data-sources.md), *Connectors*):
`Web.Contents` runs only for `allow_connectors: ["web"]`, `OData.Feed` only for `["odata"]`.

**Sources.** Public documentation only: the Power Query M reference on Microsoft Learn
(`Web.Contents` and its options, `OData.Feed` and its options), the Power Query connector
articles *Web* and *OData Feed* on Microsoft Learn (the From Web basic dialog, the
`[Implementation="2.0"]` option and the `Source{[Name=…,Signature="table"]}[Data]` navigation
step they show), and the OASIS OData v4.01 standards: *Part 1: Protocol* (service document,
server-driven paging, `OData-MaxVersion`), *Part 2: URL Conventions* (system query options,
literals, `null` in comparisons, `$orderby` null order), *JSON Format* (`@odata.context`,
`@odata.nextLink`, `IEEE754Compatible`, primitive value representations; v3 JSON light names
`odata.metadata`, `odata.nextLink`) and *CSDL XML Representation* (`EntityContainer`,
`EntitySet`, `EntityType`, `BaseType`, `Property`, `EnumType`, `TypeDefinition`, schema
`Alias`). The tests run against an in-process HTTP server of our own
(`crates/webdata/src/fake.rs`); no Desktop capture and no public service were used, and the M
shapes were not compared with a file Desktop saved.

## HTTP

| Point | What BIcraft does |
|---|---|
| Client | `ureq` 3 (blocking), rustls with the Mozilla roots (webpki-roots), no proxy, `User-Agent: bicraft/<version>`; plain `http://` is allowed, as in Desktop |
| Deadlines | connect 30 s; each request 100 s (Power Query's documented default), or the query's `Timeout` <!-- perf-guard: ok (BIcraft's own time-outs, not a measurement) --> |
| Caps | a response body of at most 256 MiB (and the evaluation's `max_bytes`), headers 64 KiB; more is `Error::LimitExceeded` |
| Redirects | up to 10, followed by the client |
| Cache | each connector keeps its successful responses by request (URL, headers, body), up to 64 MiB: a query that reads one URL twice sends one request; `IsRetry = true` skips it |
| URLs | absolute `http`/`https` only, a host and no user name, no space or control character, at most 64 KiB |

## Web.Contents

`Web.Contents(url, [options])` returns the response body as a binary value (the evaluator's
`Json.Document`, `Csv.Document`, `Excel.Workbook`, `Text.FromBinary`… read it). Options:

| Option | Effect |
|---|---|
| `RelativePath` | appended to the URL's path, one `/` between them, before its query |
| `Query` | a record of texts (or lists of texts, one pair per item) added to the query string, percent-encoded (UTF-8, space as `%20`) |
| `Headers` | a record of texts sent as headers; `Host`, `Content-Length`, `Transfer-Encoding`, `Connection`, `Upgrade`, `TE`, `Trailer` are refused |
| `Timeout` | a duration: the request's deadline |
| `Content` | a binary: the request is a `POST` with that body |
| `ManualStatusHandling` | a list of status codes whose body is returned instead of an error |
| `IsRetry` | `true`: the cache is not read |
| `ExcludedFromCacheKey` | accepted, without effect (every request is its own cache key) |
| `ApiKeyName` | refused: it needs a stored Web API key |

Any other option is refused (`Error::UnsupportedSource`). A status outside 200–299 (and outside
`ManualStatusHandling`) is a catchable `DataSource.Error` with the message
`Web.Contents failed to get contents from '<url>' (<status>): <reason>`. A failed connection is
`Error::Connector` (fatal). The binary's metadata (`Value.Metadata`: `Content.Type`,
`Response.Status`, …) is not given.

**Get data › Web** (`source.navigate {kind: web, url}`) reads the URL once, then lists it as a
JSON file when its first non-blank byte is `{` or `[` (`Json.Document(Web.Contents(url))`), as a
delimited text file otherwise (`Csv.Document(Web.Contents(url),[Delimiter=…, Encoding=…,
QuoteStyle=QuoteStyle.Csv])`, the header and type steps of a local file), and refuses an HTML or
XML page (`<`): `Web.Page` and `Web.BrowserContents` are not implemented. The item is named
after the last segment of the URL's path without its extension (else the host).

## OData.Feed

`OData.Feed(serviceUri, headers, options)`: `headers` is a record of texts (as `Headers`);
options:

| Option | Effect |
|---|---|
| `Implementation`, `ODataVersion`, `EnableBatch`, `Concurrent`, `FunctionOverloads` | accepted, without effect |
| `Headers` | more headers (as `Web.Contents`) |
| `Query` | custom query options (texts), sent with every request; a name starting with `$` is refused |
| `Timeout` | each request's deadline |
| `MoreColumns` | `false` accepted; `true` refused |

Any other option (`IncludeAnnotations`, `OmitValues`, `ApiKeyName`, …) is refused. Requests send
`Accept: application/json;odata.metadata=minimal;IEEE754Compatible=true` and
`OData-MaxVersion: 4.0`; the `$metadata` request sends `Accept: application/xml`.

### Navigation

Opening a feed reads the document at its URL once:

- `@odata.context` (v3: `odata.metadata`) without a `#` fragment, or no context: a **service
  document**. Its `value` entries of kind `EntitySet` (no `kind`: an entity set, as in v3) become
  the navigation table `Name`, `Data`, `Signature` (`"table"`), `Kind` (`"Table"`). Singletons
  and function imports are not listed. An entry whose `url` is on another origin is left out.
- A fragment (`$metadata#Products`, `#Products(ID,Name)`): the feed is that **entity set**, a
  table whose rows are that document and its next pages; nothing folds into it.

The `$metadata` URL is the context's part before `#` (relative to the request's folder when
relative), else `<service>/$metadata`; it must be on the feed's origin. It is read once per feed.

**Get data › OData feed** (`source.navigate {kind: odata, url}`) lists the entity sets, each with
the query

```m
let
    Source = OData.Feed("https://host/service/", null, [Implementation="2.0"]),
    Products_table = Source{[Name="Products",Signature="table"]}[Data]
in
    Products_table
```

(a feed that is an entity set: one item, `Source` alone). The `Kind` column and the step names
are BIcraft's; whether Desktop's navigation table has a `Kind` column was not checked.

### Columns and values

The columns are the entity type's structural properties, base types first (`BaseType` followed
up to 32 levels, aliases resolved), in `$metadata` order:

| EDM type | M type | JSON value read |
|---|---|---|
| `Edm.String`, `Edm.Guid`, an enumeration | text | a string |
| `Edm.Boolean` | logical | `true` / `false` |
| `Edm.SByte`, `Edm.Byte`, `Edm.Int16`, `Edm.Int32`, `Edm.Int64` | `Int8`, `Int16` (both), `Int16`, `Int32`, `Int64` | a number or a string of digits (`IEEE754Compatible`), range-checked |
| `Edm.Single`, `Edm.Double` | `Single`, `Double` | a number, or `"INF"`, `"-INF"`, `"NaN"`, a numeric string |
| `Edm.Decimal` | decimal | a number (its JSON text) or a numeric string, exact |
| `Edm.Date` | date | `YYYY-MM-DD` |
| `Edm.DateTimeOffset` | datetimezone | `YYYY-MM-DDThh:mm[:ss[.f…]]` and `Z` or `±hh:mm` (milliseconds kept) |
| `Edm.DateTime` (v2/v3) | datetime | ISO text, or `/Date(ms[±offset])/` (ms since 1970 UTC) |
| `Edm.TimeOfDay` | time | `hh:mm[:ss[.f…]]` |
| `Edm.Binary` | binary | base64 or base64url, padding optional |
| a `TypeDefinition` | its underlying type | |

Left out (no column): complex types, collections, `Edm.Stream`, `Edm.Duration`, geography and
geometry types, navigation properties. A missing property is `null`; annotations
(`@odata.etag`, `Prop@odata.type`) are ignored. A value that does not read as its type is
`Error::Connector`.

### Folding

The remote table folds as the other connectors do ([data-sources.md](data-sources.md), *Query
folding*), into the system query options:

| Plan | Option |
|---|---|
| columns | `$select=A,B` |
| filter | `$filter=…`: `Col eq 5`, `ne`, `lt`, `le`, `gt`, `ge`; `Col eq null` / `Col ne null`; `(a) and (b)`; literals `true`/`false`, whole numbers, `YYYY-MM-DD` dates |
| sort | `$orderby=A desc,B` (`folds_sort`) |
| limit | `$top=n`; a limit of 0 sends no request (the columns come from `$metadata`) |

OData compares with `null` as M does (`eq` and the order comparisons never match a null value,
`ne` does), and orders nulls first ascending, last descending, as M. Text columns are not
compared by the service (its collation is unknown: `[Name] = "x"` runs locally after the fetch),
nor `Edm.Single` and binary columns.

Server-driven paging: `@odata.nextLink` (v3 `odata.nextLink`; absolute, or relative to the page
read) is followed until the limit, the evaluation's row cap or 10 000 pages; a link to another
origin is refused.

### Errors

| Case | Error |
|---|---|
| 401, 403, 407 | `Error::Connector` (credentials): set the feed's credentials in Data source settings (`source.credentials`) |
| another status outside 200–299 | catchable `DataSource.Error` `OData.Feed: <url> answered <status> <reason>: <message>` (the OData error body's `error.message`, v3 `odata.error.message.value`) |
| not JSON, no `value` list, a value of the wrong type, `$metadata` not CSDL | `Error::Connector` |
| an entity set the service document does not name | catchable `DataSource.Error` |

## Not implemented

Windows authentication and OAuth sign-in (Basic, Web API key and a supplied bearer token are
read since 2026-11-20, [sharepoint.md](sharepoint.md)), the privacy firewall,
`Web.Page`, `Web.BrowserContents`, `Web.Headers`, the binary's metadata, OData Atom/XML payloads
(v2/v3 services that do not answer JSON), singletons, function imports, `$expand` and navigation
columns, complex and collection columns, `$count`, delta links, OData batch requests (SharePoint:
[sharepoint.md](sharepoint.md)). See docs/gaps.md, G69.
