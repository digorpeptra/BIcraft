# SQL Server connector (TDS, as implemented in `crates/sqlserver`)

Status: implemented 2026-11-18 (M11 lot 11). `Sql.Database(server, database, [options])` and
`Sql.Databases(server, [options])` in BIcraft's Power Query evaluator, over TDS 7.4 on TCP,
always encrypted, with SQL Server authentication. Opt-in like Databricks
([data-sources.md](data-sources.md), *Connectors*): the connector runs only for
`allow_connectors: ["sqlserver"]`.

**Sources.** Public specifications only: `[MS-TDS]` *Tabular Data Stream Protocol* (Microsoft
Open Specifications: packet header, PRELOGIN, TLS inside PRELOGIN, LOGIN7, SQLBatch with
ALL_HEADERS, the token stream, TYPE_INFO and the data type encodings, collations, PLP, ENVCHANGE
routing), `[MC-SQLR]` *SQL Server Resolution Protocol* (instance port over UDP 1434), the T-SQL
reference on Microsoft Learn (`sys.objects`, `sys.schemas`, `sys.databases`, `HAS_DBACCESS`,
`TOP`, `DATEFROMPARTS`, quoted identifiers) and the Power Query M reference (`Sql.Database`,
`Sql.Databases` and their options). The tests run against a scripted server of our own
(`crates/sqlserver/src/fake.rs`); no real SQL Server was used while writing it.

## Connection

| Step | What BIcraft sends / accepts |
|---|---|
| Server text | `host`, `host,port`, `host\instance`, `tcp:` prefix, `(local)`, `.`; an instance without a port asks the SQL Server Browser (`CLNT_UCAST_INST`, UDP 1434, 3 s) |
| PRELOGIN | version, `ENCRYPTION = ENCRYPT_ON`, `INSTOPT`, `THREADID`, `MARS` off; an answer of `ENCRYPT_OFF` or `ENCRYPT_NOT_SUP` is refused: nothing is sent unencrypted |
| TLS | rustls (TLS 1.2 and 1.3), handshake records wrapped in PRELOGIN packets as `[MS-TDS]` 2.2.6.5 says, then raw TLS for the rest of the connection; the server certificate is checked against the Mozilla roots (webpki-roots), the PEM file of `BICRAFT_SQLSERVER_CA_FILE`, and the name typed; `BICRAFT_SQLSERVER_TRUST_SERVER_CERTIFICATE=1` skips the check (Desktop's *Trust server certificate*) |
| LOGIN7 | TDS 7.4, packet size 4096 (the server's `PACKETSIZE` change is followed), the user, the password (nibble-swapped and XOR `0xA5`, as the spec says), the database, application name `BIcraft`, client library `bicraft` |
| Routing | an `ENVCHANGE` type 20 (Azure SQL gateway) is followed once, to the host and port it names |
| Batch | one `SQLBatch` per statement, with the transaction descriptor header; the answer is read token by token, every value bounded (256 MiB per value, 4096 columns) |

Timeouts: connection 15 s (`ConnectionTimeout` option), each statement 10 minutes
(`CommandTimeout` option, 1 s to 24 h).

## Credentials

- SQL Server authentication only. `source.credentials {server, user, password}` stores a login
  for the session, in memory, keyed by the normalized server name (`host[\instance][,port]`, lower
  case); without one, `BICRAFT_SQLSERVER_USER` and `BICRAFT_SQLSERVER_PASSWORD` of the process are
  used. They are read when a connection opens, never written to a file, the recovery journal
  (`Replay::Skip`), an answer, a log or `Debug` output.
- `source.credentials` is refused in server mode (a server session is shared) and to readers.
- `authentication: "windows"` is refused with a message: no pure-Rust integrated authentication
  is used. Azure Active Directory logins are not supported either.
- A refused login (errors 18452, 18456, 18470, 18486 to 18488) is a credentials error, which `try` cannot catch.

## Navigation

`Sql.Database(server, database)` is a navigation table of the database's user tables and views
(`sys.objects` types `U` and `V`, not `is_ms_shipped`), one row each:

| Column | Value |
|---|---|
| `Name` | `schema.item` |
| `Data` | the table (lazy) |
| `Schema`, `Item` | the schema and the object name |
| `Kind` | `Table` or `View` |

so the query `Source{[Schema="dbo",Item="Orders"]}[Data]` finds its table without listing the
database again. `HierarchicalNavigation = true` lists the schemas first. `Sql.Databases(server)`
lists the databases the login may open (`HAS_DBACCESS`), `Kind = "Database"`. With `[Query =
"…"]`, `Sql.Database` is the result of that statement, sent as typed; nothing folds into it.

The Navigator (`source.navigate {kind: "sqlserver", server, database, query?}`) lists the same
items and writes each one's query:

```
let
    Source = Sql.Database("server", "database"),
    dbo_Orders = Source{[Schema="dbo",Item="Orders"]}[Data]
in
    dbo_Orders
```

These shapes are BIcraft's own reading of the M reference; they were not compared with a query
Desktop wrote against a real server.

## Query folding

The steps of [data-sources.md](data-sources.md), *Query folding*, fold into one `SELECT`, plus
`Table.Sort`:

| M | T-SQL |
|---|---|
| `Table.SelectColumns`, `Table.RemoveColumns` | the column list (`[name]`, `]` doubled) |
| `Table.SelectRows` (comparisons below, `and`) | `WHERE`, `<>` as `(c <> v OR c IS NULL)` |
| `Table.Sort` | `ORDER BY`, a later sort's keys first; only before a `Table.FirstN` and only on non-text columns |
| `Table.FirstN(t, n)` | `TOP (n)` |

Constants are written as numbers, `1`/`0` for logicals and `DATEFROMPARTS(y, m, d)` for dates.
Comparisons fold on whole numbers, decimals, `float`, `bit` and `date` columns. Text never
folds (comparison or sort): SQL Server compares text by the column's collation, usually case- and
accent-insensitive and ignoring trailing spaces, while M compares code units exactly. `real`,
`datetimeoffset` and `time` columns do not fold. Everything else runs locally after the fetch,
as for Databricks.

`varchar`, `char` and `text` columns are read through `CAST(… AS nvarchar(n))` so that their
text arrives as UTF-16 whatever the code page; a value of a code page other than 1252 or UTF-8
in a native query's result is an error that asks for the cast.

## Types

| SQL Server | M type |
|---|---|
| `tinyint`, `smallint`, `int`, `bigint` | `Int8.Type` … `Int64.Type` |
| `bit` | `logical` |
| `real` / `float` | `Single.Type` / `Double.Type` |
| `decimal`, `numeric`, `money`, `smallmoney` | `Decimal.Type` (a decimal number in the model) |
| `date` | `date` |
| `datetime`, `smalldatetime`, `datetime2` | `datetime` (to the millisecond) |
| `datetimeoffset` | `datetimezone` |
| `time` | `time` |
| `char`, `varchar`, `nchar`, `nvarchar`, `text`, `ntext`, `xml`, `uniqueidentifier` | `text` (a GUID in lower case) |
| `binary`, `varbinary`, `image`, CLR types | `binary` |
| `sql_variant` | refused: cast it in the query |

## Errors

A server `ERROR` token of class 11 or more is a catchable `DataSource.Error` "Microsoft SQL: message"
(the prefix of Power Query's SQL Server errors; not compared with Desktop). Network, TLS and protocol failures
are connector errors. Readers of the token stream are fuzzed (`tds_tokens`,
[docs/fuzzing.md](../fuzzing.md)).

## Not done

- Windows and Azure AD authentication; `MultiSubnetFailover`, `MaxDegreeOfParallelism` and the
  other options listed in `IGNORED_OPTIONS` are accepted and change nothing.
- `Sql.Databases` is not offered by the Get data gallery (M only).
- Never run against a real SQL Server or Azure SQL: the TLS-in-PRELOGIN handshake, routing and
  the Browser lookup are tested against our scripted server only.
  `BICRAFT_SQLSERVER_TEST_SERVER` (with the user and password variables) runs one opt-in test
  against a real server.
