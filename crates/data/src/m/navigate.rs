//! The Navigator of Get data: what a local source holds (the sheets and tables of a workbook, a
//! text file, the lists of a JSON file, the files of a folder) and the M query that loads each
//! item, in the shapes Power BI Desktop writes for them (docs/formats/data-sources.md,
//! *Navigator*).
//!
//! Each query reads the source with the file functions of the evaluator (so only under
//! [`SourceOptions::file_roots`]), promotes the first row to headers when it looks like one, and
//! ends with a `Table.TransformColumnTypes` step whose types are detected from the first
//! [`super::entered::DETECT_ROWS`] rows, as Power Query's type detection does. An item whose
//! query does not load (a nested JSON value, a cell the evaluator cannot read) keeps its query
//! and says why in [`NavItem::error`].

use std::path::Path;

use super::entered::{self, EnteredType, m_field, m_literal};
use super::eval::Interp;
use super::value::{Flow, Value};
use crate::value::{self as cv, Culture};
use crate::{Error, SourceOptions, TableData};

/// Most items a source lists.
pub const MAX_ITEMS: usize = 1_000;
/// Bytes of a text file read to find its delimiter and encoding.
const SNIFF_BYTES: usize = 64 * 1024;

/// What kind of source a path is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavKind {
    /// An Excel workbook (`.xlsx`, `.xlsm`).
    Excel,
    /// A delimited text file (`.csv`, `.tsv`, `.txt`, `.prn`).
    Csv,
    /// A JSON file.
    Json,
    /// A folder of files.
    Folder,
}

impl NavKind {
    pub const ALL: [NavKind; 4] = [NavKind::Excel, NavKind::Csv, NavKind::Json, NavKind::Folder];

    /// The name in commands (`excel`, `csv`, `json`, `folder`).
    pub fn name(self) -> &'static str {
        match self {
            NavKind::Excel => "excel",
            NavKind::Csv => "csv",
            NavKind::Json => "json",
            NavKind::Folder => "folder",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "excel" | "xlsx" | "workbook" => NavKind::Excel,
            "csv" | "text" | "txt" => NavKind::Csv,
            "json" => NavKind::Json,
            "folder" => NavKind::Folder,
            _ => return None,
        })
    }

    /// The kind a file's extension says (`None`: an extension no kind reads).
    pub fn from_extension(path: &str) -> Option<Self> {
        let ext = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "xlsx" | "xlsm" => NavKind::Excel,
            "csv" | "tsv" | "txt" | "prn" => NavKind::Csv,
            "json" => NavKind::Json,
            _ => return None,
        })
    }
}

/// Options of [`navigate`].
#[derive(Debug, Clone, Default)]
pub struct NavOptions {
    /// A text file's delimiter; `None`: found from the first line (comma, semicolon, tab or
    /// pipe).
    pub delimiter: Option<char>,
    /// Skip type detection: the queries end at the headers.
    pub no_types: bool,
}

/// One item of a source.
#[derive(Debug, Clone, PartialEq)]
pub struct NavItem {
    /// The item's name, also the table name Desktop gives it.
    pub name: String,
    /// `Sheet`, `Table`, `DefinedName` (a workbook), `File` (a text file), `List`, `Record` (JSON),
    /// `Folder`, `Combined` (a folder's text files appended).
    pub kind: String,
    /// The M query that loads it.
    pub m: String,
    pub hidden: bool,
    /// Why the query does not load, when it does not.
    pub error: Option<String>,
}

/// An item before its query is finished: let steps (name, expression), in order.
struct Draft {
    name: String,
    kind: &'static str,
    steps: Vec<(String, String)>,
    hidden: bool,
}

/// The items of the source at `path` (a file, or a folder for [`NavKind::Folder`]); file access
/// only under `opts.file_roots`. Runs on the evaluator's own thread.
pub fn navigate(
    kind: NavKind,
    path: &str,
    opts: &SourceOptions,
    nav: &NavOptions,
) -> Result<Vec<NavItem>, Error> {
    let (path, opts, nav) = (path.to_string(), opts.clone(), nav.clone());
    super::on_eval_thread(move || run(kind, &path, &opts, &nav))
}

fn run(kind: NavKind, path: &str, opts: &SourceOptions, nav: &NavOptions) -> Result<Vec<NavItem>, Error> {
    let culture = Culture::from_name(opts.culture.as_deref().unwrap_or("en-US"));
    let drafts = match kind {
        NavKind::Excel => excel(path, opts)?,
        NavKind::Csv => vec![csv(path, opts, nav)?],
        NavKind::Json => json(path, opts)?,
        NavKind::Folder => folder(path, opts, nav)?,
    };
    Ok(drafts
        .into_iter()
        .take(MAX_ITEMS)
        .map(|d| finish(d, opts, nav, culture))
        .collect())
}

/// Evaluates `expr` to a value (no shared queries).
fn value_of(expr: &str, opts: &SourceOptions) -> Result<(Interp, Value), Error> {
    let e = super::parser::parse(expr, &opts.limits)?;
    let mut interp = Interp::new(opts, Vec::new());
    match interp.eval(&e, &None) {
        Ok(v) => Ok((interp, v)),
        Err(Flow::Fatal(e)) => Err(e),
        Err(Flow::Err(e)) => Err(Error::Evaluation {
            reason: e.reason.clone(),
            message: e.message.clone(),
        }),
    }
}

fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::Text(t) => Some(t.to_string()),
        _ => None,
    }
}

fn let_text(steps: &[(String, String)], last: &str) -> String {
    let body: Vec<String> = steps.iter().map(|(n, e)| format!("    {n} = {e}")).collect();
    format!("let\n{}\nin\n    {last}", body.join(",\n"))
}

/// The item's query with its type step (when types are detected and the query loads).
fn finish(d: Draft, opts: &SourceOptions, nav: &NavOptions, culture: Culture) -> NavItem {
    let last = d.steps.last().map(|(n, _)| n.clone()).unwrap_or_default();
    let plain = let_text(&d.steps, &last);
    let head = |steps: &[(String, String)], last: &str| {
        let mut s = steps.to_vec();
        s.push((
            "__bicraft_head".into(),
            format!("Table.FirstN({last}, {})", entered::DETECT_ROWS),
        ));
        let_text(&s, "__bicraft_head")
    };
    let mut item = NavItem {
        name: d.name,
        kind: d.kind.to_string(),
        m: plain,
        hidden: d.hidden,
        error: None,
    };
    let data: TableData = match super::evaluate(&head(&d.steps, &last), opts) {
        Ok(data) => data,
        Err(e) => {
            item.error = Some(e.to_string());
            return item;
        }
    };
    if nav.no_types {
        return item;
    }
    let mut columns = Vec::with_capacity(data.columns().len());
    for (i, c) in data.columns().iter().enumerate() {
        let ty = entered::infer_column(c.data_type, data.date_kind(i), &c.values, culture)
            .unwrap_or(EnteredType::Text);
        columns.push((c.name.clone(), ty));
    }
    if columns.is_empty() {
        return item;
    }
    let mut steps = d.steps;
    steps.push((
        "#\"Changed Type\"".into(),
        format!(
            "Table.TransformColumnTypes({last},{})",
            entered::type_list(&columns)
        ),
    ));
    // the detected types must read the first rows; else the query stays untyped
    if super::evaluate(&head(&steps, "#\"Changed Type\""), opts).is_ok() {
        item.m = let_text(&steps, "#\"Changed Type\"");
    }
    item
}

fn contents(path: &str) -> String {
    format!("File.Contents({})", m_literal(path))
}

/// Whether a first row reads as headers: some text, and no number among its values.
fn header_row<'a>(cells: impl IntoIterator<Item = Option<&'a str>>) -> bool {
    let mut any = false;
    for c in cells {
        match c {
            Some(t) if !t.trim().is_empty() => {
                if cv::parse_double(t.trim(), '.').is_some() || cv::parse_double(t.trim(), ',').is_some() {
                    return false;
                }
                any = true;
            }
            Some(_) => {}
            None => return false,
        }
    }
    any
}

fn promoted(steps: &mut Vec<(String, String)>) {
    let last = steps.last().map(|(n, _)| n.clone()).unwrap_or_default();
    steps.push((
        "#\"Promoted Headers\"".into(),
        format!("Table.PromoteHeaders({last}, [PromoteAllScalars=true])"),
    ));
}

fn step_name(item: &str, kind: &str) -> String {
    m_field(&format!("{item}_{kind}"))
}

fn excel(path: &str, opts: &SourceOptions) -> Result<Vec<Draft>, Error> {
    let source = format!("Excel.Workbook({}, null, true)", contents(path));
    let (_, v) = value_of(&source, opts)?;
    let Value::Table(t) = v else {
        return Err(Error::UnsupportedSource(
            "the workbook did not list its sheets".into(),
        ));
    };
    let col = |n: &str| t.index(n);
    let (Some(ci), Some(cd), Some(ck), Some(ch)) = (col("Item"), col("Data"), col("Kind"), col("Hidden"))
    else {
        return Err(Error::UnsupportedSource(
            "the workbook did not list its sheets".into(),
        ));
    };
    let mut out = Vec::new();
    for row in t.rows.iter().take(MAX_ITEMS) {
        let (Some(item), Some(kind)) = (row.get(ci).and_then(text_of), row.get(ck).and_then(text_of)) else {
            continue;
        };
        let kind: &'static str = match kind.as_str() {
            "Sheet" => "Sheet",
            "Table" => "Table",
            "DefinedName" => "DefinedName",
            _ => continue,
        };
        let hidden = matches!(row.get(ch), Some(Value::Logical(true)));
        let step = step_name(&item, kind);
        let mut steps = vec![
            ("Source".to_string(), source.clone()),
            (
                step.clone(),
                format!(
                    "Source{{[Item={},Kind={}]}}[Data]",
                    m_literal(&item),
                    m_literal(kind)
                ),
            ),
        ];
        // a sheet's first row is its headers when it reads as one (a table has its own)
        if kind == "Sheet"
            && let Some(Value::Table(data)) = row.get(cd)
            && let Some(first) = data.rows.first()
            && data.rows.len() > 1
        {
            let texts: Vec<Option<String>> = first
                .iter()
                .map(|v| match v {
                    Value::Text(t) => Some(t.to_string()),
                    Value::Null => Some(String::new()),
                    _ => None,
                })
                .collect();
            if header_row(texts.iter().map(Option::as_deref)) {
                promoted(&mut steps);
            }
        }
        out.push(Draft {
            name: item,
            kind,
            steps,
            hidden,
        });
    }
    Ok(out)
}

/// The start of a file's bytes (at most [`SNIFF_BYTES`]).
fn head_bytes(path: &str, opts: &SourceOptions) -> Result<Vec<u8>, Error> {
    let (_, v) = value_of(&contents(path), opts)?;
    bytes_head(&v)
}

fn bytes_head(v: &Value) -> Result<Vec<u8>, Error> {
    let b = super::lib::misc::binary(v).map_err(|f| match f {
        Flow::Fatal(e) => e,
        Flow::Err(e) => Error::Evaluation {
            reason: e.reason.clone(),
            message: e.message.clone(),
        },
    })?;
    Ok(b.get(..b.len().min(SNIFF_BYTES)).unwrap_or_default().to_vec())
}

/// The delimiter of a first line: the most frequent of comma, semicolon, tab and pipe outside
/// quotes (comma when none is there).
fn sniff_delimiter(head: &str) -> char {
    let line = head.lines().next().unwrap_or("");
    let mut counts = [(',', 0usize), (';', 0), ('\t', 0), ('|', 0)];
    let mut quoted = false;
    for c in line.chars() {
        if c == '"' {
            quoted = !quoted;
        } else if !quoted && let Some(e) = counts.iter_mut().find(|(d, _)| *d == c) {
            e.1 += 1;
        }
    }
    counts
        .iter()
        .filter(|(_, n)| *n > 0)
        .max_by_key(|(_, n)| *n)
        .map_or(',', |(d, _)| *d)
}

/// The code page of a text file: UTF-8 when its start decodes as UTF-8 (a character cut at the
/// end aside), else Windows-1252.
fn encoding(head: &[u8]) -> u32 {
    match std::str::from_utf8(head) {
        Ok(_) => 65001,
        Err(e) if e.error_len().is_none() => 65001,
        Err(_) => 1252,
    }
}

fn csv_options(delimiter: char, code_page: u32) -> String {
    format!(
        "[Delimiter={}, Encoding={code_page}, QuoteStyle=QuoteStyle.Csv]",
        m_literal(&delimiter.to_string())
    )
}

/// Whether the first line of delimited text reads as headers.
fn csv_header(head: &str, delimiter: char) -> bool {
    let line = head.lines().next().unwrap_or("");
    let fields: Vec<&str> = line
        .split(delimiter)
        .map(|f| f.trim().trim_matches('"'))
        .collect();
    header_row(fields.into_iter().map(Some)) && head.lines().nth(1).is_some()
}

fn stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("Query")
        .to_string()
}

fn csv(path: &str, opts: &SourceOptions, nav: &NavOptions) -> Result<Draft, Error> {
    let head = head_bytes(path, opts)?;
    let tab = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("tsv"));
    Ok(csv_draft(&contents(path), &head, stem(path), tab, nav))
}

/// A text source read by `Csv.Document(binary, …)`, from the start of its bytes.
fn csv_draft(binary: &str, head: &[u8], name: String, tab: bool, nav: &NavOptions) -> Draft {
    let code_page = encoding(head);
    let text = String::from_utf8_lossy(head.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(head)).into_owned();
    let delimiter = nav
        .delimiter
        .unwrap_or_else(|| if tab { '\t' } else { sniff_delimiter(&text) });
    let mut steps = vec![(
        "Source".to_string(),
        format!("Csv.Document({binary},{})", csv_options(delimiter, code_page)),
    )];
    if csv_header(&text, delimiter) {
        promoted(&mut steps);
    }
    Draft {
        name,
        kind: "File",
        steps,
        hidden: false,
    }
}

/// The steps turning a JSON list into a table: records become rows, other values one column.
fn list_table(list: &[Value], from: &str) -> (String, String) {
    if !list.is_empty() && list.iter().all(|v| matches!(v, Value::Record(_))) {
        (
            "#\"Converted to Table\"".into(),
            format!("Table.FromRecords({from})"),
        )
    } else {
        (
            "#\"Converted to Table\"".into(),
            format!("Table.FromList({from}, Splitter.SplitByNothing(), {{\"Value\"}})"),
        )
    }
}

fn json(path: &str, opts: &SourceOptions) -> Result<Vec<Draft>, Error> {
    json_drafts(&contents(path), stem(path), opts)
}

/// The items of a JSON document read by `Json.Document(binary)`.
fn json_drafts(binary: &str, name: String, opts: &SourceOptions) -> Result<Vec<Draft>, Error> {
    let source = format!("Json.Document({binary})");
    let (_, v) = value_of(&source, opts)?;
    let base = ("Source".to_string(), source);
    let mut out = Vec::new();
    match v {
        Value::List(list) => {
            out.push(Draft {
                name,
                kind: "List",
                steps: vec![base, list_table(&list, "Source")],
                hidden: false,
            });
        }
        Value::Record(r) => {
            out.push(Draft {
                name: name.clone(),
                kind: "Record",
                steps: vec![
                    base.clone(),
                    ("#\"Converted to Table\"".into(), "Record.ToTable(Source)".into()),
                ],
                hidden: false,
            });
            for (field, value) in r.names.iter().zip(&r.values).take(MAX_ITEMS) {
                let access = format!("Source[{}]", m_field(field));
                let steps = match value {
                    Value::List(list) => vec![
                        base.clone(),
                        (m_field(field), access),
                        list_table(list, &m_field(field)),
                    ],
                    Value::Record(_) => vec![
                        base.clone(),
                        (m_field(field), access),
                        (
                            "#\"Converted to Table\"".into(),
                            format!("Record.ToTable({})", m_field(field)),
                        ),
                    ],
                    _ => continue,
                };
                out.push(Draft {
                    name: field.to_string(),
                    kind: if matches!(value, Value::List(_)) {
                        "List"
                    } else {
                        "Record"
                    },
                    steps,
                    hidden: false,
                });
            }
        }
        _ => {
            return Err(Error::UnsupportedSource(
                "the JSON file holds a single value, not a list or a record".into(),
            ));
        }
    }
    Ok(out)
}

/// The columns of a folder's file list that load (its `Content` and `Attributes` do not).
const FOLDER_COLUMNS: &str =
    "{\"Name\", \"Extension\", \"Date accessed\", \"Date modified\", \"Date created\", \"Folder Path\"}";

fn folder(path: &str, opts: &SourceOptions, nav: &NavOptions) -> Result<Vec<Draft>, Error> {
    let source = format!("Folder.Files({})", m_literal(path));
    let name = Path::new(path.trim_end_matches(['/', '\\']))
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("Folder")
        .to_string();
    folder_drafts(source, name, opts, nav)
}

/// The items of a file list (`Folder.Files`, `SharePoint.Files`): the list's loadable columns,
/// and its CSV files appended, read as the first one is.
fn folder_drafts(
    source: String,
    name: String,
    opts: &SourceOptions,
    nav: &NavOptions,
) -> Result<Vec<Draft>, Error> {
    let (mut interp, v) = value_of(&source, opts)?;
    let v = match v {
        Value::Remote(r) => match super::remote::force(&mut interp, &r) {
            Ok(t) => Value::Table(t),
            Err(Flow::Fatal(e)) => return Err(e),
            Err(Flow::Err(e)) => {
                return Err(Error::Evaluation {
                    reason: e.reason.clone(),
                    message: e.message.clone(),
                });
            }
        },
        v => v,
    };
    let Value::Table(t) = v else {
        return Err(Error::UnsupportedSource(
            "the folder did not list its files".into(),
        ));
    };
    let base = ("Source".to_string(), source);
    let mut out = vec![Draft {
        name: name.clone(),
        kind: "Folder",
        steps: vec![
            base.clone(),
            (
                "#\"Removed Other Columns\"".into(),
                format!("Table.SelectColumns(Source,{FOLDER_COLUMNS})"),
            ),
        ],
        hidden: false,
    }];
    // the text files appended, read as the first one is
    let (Some(ce), Some(cc)) = (t.index("Extension"), t.index("Content")) else {
        return Ok(out);
    };
    let first = t.rows.iter().find(|r| {
        r.get(ce)
            .and_then(text_of)
            .is_some_and(|e| e.eq_ignore_ascii_case(".csv"))
    });
    if let Some(row) = first
        && let Some(content) = row.get(cc)
    {
        let head = bytes_head(content)?;
        let text =
            String::from_utf8_lossy(head.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&head)).into_owned();
        let delimiter = nav.delimiter.unwrap_or_else(|| sniff_delimiter(&text));
        let doc = format!("Csv.Document(_, {})", csv_options(delimiter, encoding(&head)));
        let each = if csv_header(&text, delimiter) {
            format!("Table.PromoteHeaders({doc}, [PromoteAllScalars=true])")
        } else {
            doc
        };
        out.push(Draft {
            name: format!("{name} (combined)"),
            kind: "Combined",
            steps: vec![
                base,
                (
                    "#\"Filtered Rows\"".into(),
                    "Table.SelectRows(Source, each Text.Lower([Extension]) = \".csv\")".into(),
                ),
                (
                    "#\"Combined Files\"".into(),
                    format!("Table.Combine(List.Transform(#\"Filtered Rows\"[Content], each {each}))"),
                ),
            ],
            hidden: false,
        });
    }
    Ok(out)
}

/// The source call of a SQL Server database: `Sql.Database(server, database[, [Query = …]])`.
pub fn sql_source(server: &str, database: &str, query: Option<&str>) -> String {
    match query {
        Some(q) => format!(
            "Sql.Database({}, {}, [Query={}])",
            m_literal(server),
            m_literal(database),
            m_literal(q)
        ),
        None => format!("Sql.Database({}, {})", m_literal(server), m_literal(database)),
    }
}

/// The Navigator of a SQL Server database (docs/formats/tds.md, *Navigation*): its user tables and
/// views, each with a navigation query (`Source = Sql.Database(server, database)`,
/// then the `{[Schema = …, Item = …]}[Data]` step named `schema_item`); with a native `query`, one
/// item `Query1` whose query is the source call itself. The listing runs through the connector
/// `opts.connectors` holds (it must handle `Sql.Database`); nothing is previewed here.
pub fn sql_server(
    server: &str,
    database: &str,
    query: Option<&str>,
    opts: &SourceOptions,
) -> Result<Vec<NavItem>, Error> {
    let source = sql_source(server, database, query);
    if query.is_some() {
        return Ok(vec![NavItem {
            name: "Query1".into(),
            kind: "Query".into(),
            m: let_text(&[("Source".into(), source)], "Source"),
            hidden: false,
            error: None,
        }]);
    }
    let listing = super::load_m_with(
        &format!("Table.SelectColumns({source}, {{\"Name\", \"Schema\", \"Item\", \"Kind\"}})"),
        opts,
    )?;
    let text = |name: &str, i: usize| -> String {
        match listing.column(name).and_then(|c| c.values.get(i)) {
            Some(crate::CellValue::Text(t)) => t.clone(),
            _ => String::new(),
        }
    };
    let mut out = Vec::new();
    for i in 0..listing.row_count().min(MAX_ITEMS) {
        let (schema, item) = (text("Schema", i), text("Item", i));
        let step = m_field(&format!("{schema}_{item}"));
        let steps = vec![
            ("Source".to_string(), source.clone()),
            (
                step.clone(),
                format!(
                    "Source{{[Schema={},Item={}]}}[Data]",
                    m_literal(&schema),
                    m_literal(&item)
                ),
            ),
        ];
        out.push(NavItem {
            name: text("Name", i),
            kind: text("Kind", i),
            m: let_text(&steps, &step),
            hidden: false,
            error: None,
        });
    }
    Ok(out)
}

/// The source call of a web resource: `Web.Contents(url)`.
pub fn web_contents(url: &str) -> String {
    format!("Web.Contents({})", m_literal(url))
}

/// The name Get data gives a web resource: the last segment of its path without extension,
/// else its host.
fn url_stem(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let path = rest.split(['?', '#']).next().unwrap_or(rest);
    let (host, path) = path.split_once('/').unwrap_or((path, ""));
    let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    let stem = Path::new(last)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    if !stem.is_empty() {
        stem
    } else if !host.is_empty() {
        host.to_string()
    } else {
        "Query".to_string()
    }
}

/// The extension of the last segment of a URL's path.
fn url_extension(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last = path.rsplit('/').next()?;
    Path::new(last).extension()?.to_str().map(str::to_string)
}

/// The Navigator of a web resource (Get data › Web, a basic URL; docs/formats/data-sources.md,
/// *Web*): its bytes are read once through `Web.Contents` (the connector `opts.connectors` holds
/// must handle it), then a JSON document lists as a JSON file does
/// (`Json.Document(Web.Contents(…))`) and other text as a delimited text file
/// (`Csv.Document(Web.Contents(…), …)`). An HTML or XML page is refused: bicraft does not read web
/// pages (`Web.Page`, `Web.BrowserContents`).
pub fn web(url: &str, opts: &SourceOptions, nav: &NavOptions) -> Result<Vec<NavItem>, Error> {
    let (url, opts, nav) = (url.to_string(), opts.clone(), nav.clone());
    super::on_eval_thread(move || {
        let culture = Culture::from_name(opts.culture.as_deref().unwrap_or("en-US"));
        let binary = web_contents(&url);
        let (_, v) = value_of(&binary, &opts)?;
        let head = bytes_head(&v)?;
        let body = head.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&head);
        let first = body.iter().find(|b| !b.is_ascii_whitespace()).copied();
        let name = url_stem(&url);
        let drafts = match first {
            Some(b'{' | b'[') => json_drafts(&binary, name, &opts)?,
            Some(b'<') => {
                return Err(Error::UnsupportedSource(format!(
                    "`{url}` is an HTML or XML page: bicraft reads JSON and delimited text from the web, \
                     not web pages (Web.Page)"
                )));
            }
            None => return Err(Error::UnsupportedSource(format!("`{url}` returned no content"))),
            Some(_) => {
                let tab = url_extension(&url).is_some_and(|e| e.eq_ignore_ascii_case("tsv"));
                vec![csv_draft(&binary, &head, name, tab, &nav)]
            }
        };
        Ok(drafts
            .into_iter()
            .take(MAX_ITEMS)
            .map(|d| finish(d, &opts, &nav, culture))
            .collect())
    })
}

/// The source call of a SharePoint folder: `SharePoint.Files(url, [ApiVersion = 15])`
/// (docs/formats/sharepoint.md).
pub fn sharepoint_files_source(url: &str) -> String {
    format!(
        "SharePoint.Files({}, [ApiVersion = 15])",
        m_literal(url.trim_end_matches('/'))
    )
}

/// The source call of a SharePoint list: `SharePoint.Tables(url, [Implementation = "2.0",
/// ViewMode = "All"])`.
pub fn sharepoint_tables_source(url: &str) -> String {
    format!(
        "SharePoint.Tables({}, [Implementation = \"2.0\", ViewMode = \"All\"])",
        m_literal(url.trim_end_matches('/'))
    )
}

/// The Navigator of a SharePoint folder (Get data › SharePoint folder): as a local folder's, the
/// site's file list (`SharePoint.Files`, its loadable columns) and its CSV files appended. The
/// connector `opts.connectors` holds must handle `SharePoint.Files`.
pub fn sharepoint_folder(url: &str, opts: &SourceOptions, nav: &NavOptions) -> Result<Vec<NavItem>, Error> {
    let (url, opts, nav) = (url.to_string(), opts.clone(), nav.clone());
    super::on_eval_thread(move || {
        let culture = Culture::from_name(opts.culture.as_deref().unwrap_or("en-US"));
        let drafts = folder_drafts(sharepoint_files_source(&url), url_stem(&url), &opts, &nav)?;
        Ok(drafts
            .into_iter()
            .take(MAX_ITEMS)
            .map(|d| finish(d, &opts, &nav, culture))
            .collect())
    })
}

/// The Navigator of a SharePoint list source (Get data › SharePoint list): one item per list of
/// the site, named by its title, each `Source = SharePoint.Tables(…)` then the step
/// `#"<id>" = Source{[Id = "<id>"]}[Items]`. Nothing is previewed here.
pub fn sharepoint_list(url: &str, opts: &SourceOptions) -> Result<Vec<NavItem>, Error> {
    let source = sharepoint_tables_source(url);
    let listing = super::load_m_with(
        &format!("Table.SelectColumns({source}, {{\"Id\", \"Title\"}})"),
        opts,
    )?;
    let text = |name: &str, i: usize| -> String {
        match listing.column(name).and_then(|c| c.values.get(i)) {
            Some(crate::CellValue::Text(t)) => t.clone(),
            _ => String::new(),
        }
    };
    let mut out = Vec::new();
    for i in 0..listing.row_count().min(MAX_ITEMS) {
        let (id, title) = (text("Id", i), text("Title", i));
        let step = m_field(&id);
        let steps = vec![
            ("Source".to_string(), source.clone()),
            (step.clone(), format!("Source{{[Id={}]}}[Items]", m_literal(&id))),
        ];
        out.push(NavItem {
            name: title,
            kind: "Table".into(),
            m: let_text(&steps, &step),
            hidden: false,
            error: None,
        });
    }
    Ok(out)
}

/// The source call of an OData feed, as Desktop writes it: `OData.Feed(url, null,
/// [Implementation="2.0"])`.
pub fn odata_source(url: &str) -> String {
    format!("OData.Feed({}, null, [Implementation=\"2.0\"])", m_literal(url))
}

/// The Navigator of an OData feed (docs/formats/odata.md, *Navigation*): the service's entity
/// sets, each with the query `Source = OData.Feed(…)`, then the
/// `{[Name = …, Signature = "table"]}[Data]` step named `name_table`; a feed that is an entity
/// set itself is one item whose query is the source call. The listing runs through the connector
/// `opts.connectors` holds (it must handle `OData.Feed`); nothing is previewed here.
pub fn odata(url: &str, opts: &SourceOptions) -> Result<Vec<NavItem>, Error> {
    let source = odata_source(url);
    let names = super::load_m_with(
        &format!("Table.FromColumns({{Table.ColumnNames({source})}}, {{\"Column\"}})"),
        opts,
    )?;
    let names: Vec<&str> = names
        .column("Column")
        .map(|c| {
            c.values
                .iter()
                .filter_map(|v| match v {
                    crate::CellValue::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let is_nav = ["Name", "Data", "Signature"].iter().all(|c| names.contains(c));
    if !is_nav {
        return Ok(vec![NavItem {
            name: url_stem(url),
            kind: "Table".into(),
            m: let_text(&[("Source".into(), source)], "Source"),
            hidden: false,
            error: None,
        }]);
    }
    let listing = super::load_m_with(
        &format!("Table.SelectColumns({source}, {{\"Name\", \"Signature\", \"Kind\"}})"),
        opts,
    )?;
    let text = |name: &str, i: usize| -> String {
        match listing.column(name).and_then(|c| c.values.get(i)) {
            Some(crate::CellValue::Text(t)) => t.clone(),
            _ => String::new(),
        }
    };
    let mut out = Vec::new();
    for i in 0..listing.row_count().min(MAX_ITEMS) {
        let (name, signature) = (text("Name", i), text("Signature", i));
        let step = m_field(&format!("{name}_{signature}"));
        let steps = vec![
            ("Source".to_string(), source.clone()),
            (
                step.clone(),
                format!(
                    "Source{{[Name={},Signature={}]}}[Data]",
                    m_literal(&name),
                    m_literal(&signature)
                ),
            ),
        ];
        out.push(NavItem {
            name,
            kind: text("Kind", i),
            m: let_text(&steps, &step),
            hidden: false,
            error: None,
        });
    }
    Ok(out)
}
