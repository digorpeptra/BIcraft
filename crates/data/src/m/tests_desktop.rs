//! Golden tests: each query and its expected rows were checked against Power BI Desktop
//! 2.158.1304.0 (open the PBIP, refresh, `EVALUATE` the table; see
//! `docs/formats/data-sources.md`, *Desktop agreement*). Text values are compared as M produces
//! them: Analysis Services itself stores text case-insensitively and without trailing spaces.
//! Generated from the comparison harness; the data files are our own synthetic samples.

use super::*;

/// Synthetic input files, base64.
const FILES: &[(&str, &str)] = &[
    (
        "sales.csv",
        "SWQsTmFtZSxBbW91bnQsRGF0ZSxOb3RlDQoxLCJTbWl0aCwgSm9obiIsMTIuNSwyMDI2LTAxLTA1LHBsYWluDQoyLCJRdW90ZSAiIlEiIiIsLTMsMDEvMzEvMjAyNiwibXVsdGkNCmxpbmUiDQozLCwwLjI1LDIwMjYtMDItMjgsDQo0LExhc3QsMWUzLDIwMjYtMDMtMDEsZXh0cmEsZmllbGQNCjUsU2hvcnQsNw0KNiwgcGFkZGVkICwxLDIwMjYtMDMtMDIsIHggDQo=",
    ),
    (
        "people.json",
        "W3sibmFtZSI6ICJBbm4iLCAiYWdlIjogMzEsICJ0YWdzIjogWyJhIiwgImIiXSwgImFjdGl2ZSI6IHRydWUsICJqb2luZWQiOiAiMjAyNC0wNS0wMSIsICJzY29yZSI6IDEuMjV9LCB7Im5hbWUiOiAiQm9iIiwgImFnZSI6IG51bGwsICJhY3RpdmUiOiBmYWxzZSwgImpvaW5lZCI6ICIyMDIzLTEyLTMxVDEwOjAwOjAwIiwgInNjb3JlIjogLTN9LCB7Im5hbWUiOiAiQ8Opw6kiLCAiYWdlIjogNDAuMCwgImFjdGl2ZSI6IHRydWUsICJqb2luZWQiOiAiMjAyNi0wMS0wNVQyMzo1OTo1OS41IiwgInNjb3JlIjogMC4wMDF9XQ==",
    ),
    (
        "book.xlsx",
        "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",
    ),
    (
        "cp1252.csv",
        "bmFtZTtjaXR5O24NCmNhZuk7WvxyaWNoOzENCmQ7ZQ0KZjtnO2g7aQ0K",
    ),
    ("extra.csv", "YSxiLGMNCmQsZQ0KZg0K"),
    (
        "book2.xlsx",
        "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",
    ),
];

fn b64_decode(s: &str) -> Vec<u8> {
    let val = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            _ => 63,
        }
    };
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes().filter(|c| *c != b'=') {
        acc = (acc << 6) | val(c);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

/// A guarded temp folder holding the sample files, removed when the guard drops.
pub(super) fn data_root() -> crate::tempdir::TempDir {
    let d = crate::tempdir::TempDir::new("mq-desktop-samples");
    for (name, data) in FILES {
        std::fs::write(d.join(name), b64_decode(data)).unwrap();
    }
    d
}

fn ole_iso(v: f64) -> String {
    let (day, ms) = crate::value::split_ole(v).unwrap();
    let (y, m, d) = crate::value::civil_from_days(day - 25_569);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

fn show(v: &CellValue) -> String {
    match v {
        CellValue::Null => "<null>".into(),
        CellValue::Int(i) => i.to_string(),
        CellValue::Real(r) => format!("{r:?}"),
        CellValue::Decimal(d) => {
            let a = d.unsigned_abs();
            let frac = format!("{:04}", a % 10_000);
            let frac = frac.trim_end_matches('0');
            let sign = if *d < 0 { "-" } else { "" };
            if frac.is_empty() {
                format!("{sign}{}", a / 10_000)
            } else {
                format!("{sign}{}.{frac}", a / 10_000)
            }
        }
        CellValue::Text(t) => t
            .replace('\\', "\\\\")
            .replace('\t', "\\t")
            .replace('\r', "\\r")
            .replace('\n', "\\n"),
        CellValue::Bool(b) => b.to_string(),
        CellValue::DateTime(d) => ole_iso(*d),
    }
}

fn check(
    m: &str,
    cols: &[(&str, &str)],
    expected: &[&str],
    culture: &str,
    errors_as_null: bool,
    shared: &[(&str, &str)],
    skipped: usize,
) {
    let root = data_root();
    let mut root_text = root.to_str().unwrap().to_string();
    root_text.push(std::path::MAIN_SEPARATOR);
    let m = m.replace("{root}", &root_text.replace('"', "\"\""));
    let columns: Vec<bicraft_model::Column> = cols
        .iter()
        .map(|(n, t)| bicraft_model::Column {
            name: (*n).into(),
            data_type: DataType::from_tmsl(t),
            source_column: Some((*n).into()),
            ..Default::default()
        })
        .collect();
    let p = bicraft_model::Partition {
        name: "P".into(),
        source_type: "m".into(),
        expression: Some(m),
        ..Default::default()
    };
    let opts = SourceOptions {
        file_roots: vec![root.to_path_buf()],
        culture: Some(culture.into()),
        strict_errors: !errors_as_null,
        shared: shared.iter().map(|(a, b)| ((*a).into(), (*b).into())).collect(),
        ..SourceOptions::default()
    };
    let got: Vec<String> = match crate::load_partition_opts(&p, &columns, &opts) {
        Ok(t) => (0..t.row_count())
            .map(|i| {
                t.row(i)
                    .unwrap()
                    .iter()
                    .map(|v| show(v))
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect(),
        Err(e) => vec![format!("!ERROR {e}")],
    };
    let mut got_sorted = got.clone();
    got_sorted.sort();
    let mut want: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    want.sort();
    if expected.iter().any(|e| e.starts_with("!ERROR")) {
        assert!(
            got.iter().any(|g| g.starts_with("!ERROR")),
            "expected an error, got {got:#?}"
        );
        return;
    }
    // Rows left out of `expected` on purpose (`skipped`).
    for w in &want {
        assert!(got_sorted.contains(w), "missing row {w:?}; got {got:#?}");
    }
    assert_eq!(got.len(), want.len() + skipped, "{got:#?}");
}

#[test]
fn desktop_t01_enterdata() {
    check(
        "let\n    Source = Table.FromRows(Json.Document(Binary.Decompress(Binary.FromText(\"XZBNC8IwDIb/Sul5XdN0Ku428QPFmzs5dqhS2aF+4DbQf2+TKQMhTdL2yZumVSWL8GicTKSNy6QT8nqiEXAa0+7Ze1knlVz4jiA1iw5TJIwQBaiAKtfF/rBicuOuV8cUa3nWRW0NadJR+dNc+sCilkkFqRl0Nc6JzWJ+caEd4NWjjfvR6OzIj7r1Ifw5LmBtk6DNfpH77NxNzBLxHXBsUDbDiO50jv5FNVZH+4Jv3zK2vTMlMhQURDp+hVWQlQZyCzkAXdLvCVnXHw==\", BinaryEncoding.Base64), Compression.Deflate)), let _t = ((type nullable text) meta [Serialized.Text = true]) in type table [Name = _t, Qty = _t, Price = _t, When = _t, Flag = _t]),\n    #\"Changed Type\" = Table.TransformColumnTypes(Source,{{\"Name\", type text}, {\"Qty\", Int64.Type}, {\"Price\", type number}, {\"When\", type date}, {\"Flag\", type logical}})\nin\n    #\"Changed Type\"",
        &[
            ("Name", "string"),
            ("Qty", "int64"),
            ("Price", "double"),
            ("When", "dateTime"),
            ("Flag", "boolean"),
        ],
        &[
            "Alpha\t3\t1.5\t2026-01-05T00:00:00.000\ttrue",
            "Beta\t-7\t2.25\t2026-02-03T00:00:00.000\tfalse",
            "Gamma\t2\t1000.0\t2025-12-31T00:00:00.000\ttrue",
            "Delta\t4\t-0.125\t2024-02-29T00:00:00.000\tfalse",
            "Eps\t<null>\t<null>\t<null>\t<null>",
            "Zeta\t<null>\t<null>\t<null>\t<null>",
            "Eta\t1234\t1234.5\t2026-01-07T00:00:00.000\tfalse",
            "Theta\t<null>\t<null>\t<null>\t<null>",
            "Iota\t42\t0.5\t<null>\ttrue",
        ],
        "en-US",
        true,
        &[(
            "Rate",
            "1.1 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
        )],
        0,
    );
}
#[test]
fn desktop_t02_transforms() {
    check(
        "let\n    Source = #table(type table [Id = Int64.Type, Name = text, Cat = text, Qty = Int64.Type, Price = number, When = date], {\n        {1, \"apple\", \"Fruit\", 3, 1.25, #date(2026,1,5)},\n        {2, \"Banana\", \"Fruit\", 12, 0.5, #date(2026,1,31)},\n        {3, \"carrot\", \"Veg\", 7, 0.8, #date(2026,2,28)},\n        {4, \"Daikon\", \"Veg\", 1, 2.75, #date(2025,12,31)},\n        {5, \"eggplant\", \"Veg\", 5, 1.1, #date(2024,2,29)},\n        {6, \"fig\", \"Fruit\", 9, 3.3, #date(2026,3,1)},\n        {7, null, \"Other\", 4, null, null}\n    }),\n    #\"Renamed\" = Table.RenameColumns(Source, {{\"Qty\", \"Quantity\"}, {\"When\", \"Day\"}}),\n    #\"Filtered\" = Table.SelectRows(#\"Renamed\", each [Quantity] > 2 and [Cat] <> \"Other\" or [Name] = null),\n    #\"Amount\" = Table.AddColumn(#\"Filtered\", \"Amount\", each [Quantity] * [Price], type number),\n    #\"Label\" = Table.AddColumn(#\"Amount\", \"Label\", each Text.Upper(Text.Start([Name], 3)) & \"-\" & Text.From([Quantity]) & \"/\" & [Cat], type text),\n    #\"Next\" = Table.AddColumn(#\"Label\", \"NextWeek\", each Date.AddDays([Day], 7), type date),\n    #\"EOM\" = Table.AddColumn(#\"Next\", \"EOM\", each Date.EndOfMonth([Day]), type date),\n    #\"Y\" = Table.AddColumn(#\"EOM\", \"Y\", each Date.Year([Day]) * 100 + Date.Month([Day]), Int64.Type),\n    #\"Dow\" = Table.AddColumn(#\"Y\", \"Dow\", each Date.DayOfWeek([Day], Day.Monday), Int64.Type),\n    #\"MName\" = Table.AddColumn(#\"Dow\", \"MName\", each Date.MonthName([Day]), type text),\n    #\"Big\" = Table.AddColumn(#\"MName\", \"Big\", each if [Amount] = null then \"n/a\" else if [Amount] >= 10 then \"big\" else \"small\", type text),\n    #\"Sorted\" = Table.Sort(#\"Big\", {{\"Cat\", Order.Descending}, {\"Amount\", Order.Ascending}}),\n    #\"Removed\" = Table.RemoveColumns(#\"Sorted\", {\"Price\"}),\n    #\"Indexed\" = Table.AddIndexColumn(#\"Removed\", \"Idx\", 1, 1, Int64.Type)\nin\n    #\"Indexed\"",
        &[
            ("Id", "int64"),
            ("Name", "string"),
            ("Cat", "string"),
            ("Quantity", "int64"),
            ("Day", "dateTime"),
            ("Amount", "double"),
            ("Label", "string"),
            ("NextWeek", "dateTime"),
            ("EOM", "dateTime"),
            ("Y", "int64"),
            ("Dow", "int64"),
            ("MName", "string"),
            ("Big", "string"),
            ("Idx", "int64"),
        ],
        &[
            "5\teggplant\tVeg\t5\t2024-02-29T00:00:00.000\t5.5\tEGG-5/Veg\t2024-03-07T00:00:00.000\t2024-02-29T00:00:00.000\t202402\t3\tFebruary\tsmall\t1",
            "3\tcarrot\tVeg\t7\t2026-02-28T00:00:00.000\t5.6000000000000005\tCAR-7/Veg\t2026-03-07T00:00:00.000\t2026-02-28T00:00:00.000\t202602\t5\tFebruary\tsmall\t2",
            "7\t<null>\tOther\t4\t<null>\t<null>\t<null>\t<null>\t<null>\t<null>\t<null>\t<null>\tn/a\t3",
            "1\tapple\tFruit\t3\t2026-01-05T00:00:00.000\t3.75\tAPP-3/Fruit\t2026-01-12T00:00:00.000\t2026-01-31T00:00:00.000\t202601\t0\tJanuary\tsmall\t4",
            "2\tBanana\tFruit\t12\t2026-01-31T00:00:00.000\t6.0\tBAN-12/Fruit\t2026-02-07T00:00:00.000\t2026-01-31T00:00:00.000\t202601\t5\tJanuary\tsmall\t5",
            "6\tfig\tFruit\t9\t2026-03-01T00:00:00.000\t29.7\tFIG-9/Fruit\t2026-03-08T00:00:00.000\t2026-03-31T00:00:00.000\t202603\t6\tMarch\tbig\t6",
        ],
        "en-US",
        true,
        &[(
            "Rate",
            "1.1 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
        )],
        0,
    );
}
#[test]
fn desktop_t03_group() {
    check(
        "let\n    Source = #table(type table [Cat = text, Region = text, Amount = number], {{\"A\",\"N\",10},{\"B\",\"S\",5.5},{\"A\",\"S\",2.25},{\"a\",\"N\",1},{\"B\",\"N\",null},{\"C\",\"N\",7},{\"A\",\"N\",3}}),\n    Grouped = Table.Group(Source, {\"Cat\", \"Region\"}, {{\"Total\", each List.Sum([Amount]), type number}, {\"Cnt\", each Table.RowCount(_), Int64.Type}, {\"Max\", each List.Max([Amount]), type number}, {\"Avg\", each List.Average([Amount]), type number}, {\"NonNull\", each List.NonNullCount([Amount]), Int64.Type}}),\n    Indexed = Table.AddIndexColumn(Grouped, \"Idx\", 0, 1, Int64.Type)\nin\n    Indexed",
        &[
            ("Cat", "string"),
            ("Region", "string"),
            ("Total", "double"),
            ("Cnt", "int64"),
            ("Max", "double"),
            ("Avg", "double"),
            ("NonNull", "int64"),
            ("Idx", "int64"),
        ],
        &[
            "A\tN\t13.0\t2\t10.0\t6.5\t2\t0",
            "B\tS\t5.5\t1\t5.5\t5.5\t1\t1",
            "A\tS\t2.25\t1\t2.25\t2.25\t1\t2",
            "a\tN\t1.0\t1\t1.0\t1.0\t1\t3",
            "B\tN\t<null>\t1\t<null>\t<null>\t0\t4",
            "C\tN\t7.0\t1\t7.0\t7.0\t1\t5",
        ],
        "en-US",
        true,
        &[(
            "Rate",
            "1.1 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
        )],
        0,
    );
}
#[test]
fn desktop_t04_leftjoin() {
    check(
        "let\n    Orders = #table(type table [OrderId = Int64.Type, Cust = text, Amount = number], {{1,\"c1\",10},{2,\"c2\",20},{3,\"c9\",30},{4,null,40},{5,\"c1\",50},{6,\"C1\",60}}),\n    Customers = #table(type table [Cust = text, CustName = text, Tier = Int64.Type], {{\"c1\",\"One\",1},{\"c2\",\"Two\",2},{\"c2\",\"Two bis\",3},{null,\"Nobody\",0},{\"c3\",\"Three\",3}}),\n    Joined = Table.NestedJoin(Orders, {\"Cust\"}, Customers, {\"Cust\"}, \"C\", JoinKind.LeftOuter),\n    Expanded = Table.ExpandTableColumn(Joined, \"C\", {\"CustName\", \"Tier\"}, {\"CustName\", \"Tier\"})\nin\n    Expanded",
        &[
            ("OrderId", "int64"),
            ("Cust", "string"),
            ("Amount", "double"),
            ("CustName", "string"),
            ("Tier", "int64"),
        ],
        &[
            "1\tc1\t10.0\tOne\t1",
            "2\tc2\t20.0\tTwo\t2",
            "2\tc2\t20.0\tTwo bis\t3",
            "3\tc9\t30.0\t<null>\t<null>",
            "4\t<null>\t40.0\tNobody\t0",
            "5\tc1\t50.0\tOne\t1",
            "6\tC1\t60.0\t<null>\t<null>",
        ],
        "en-US",
        true,
        &[(
            "Rate",
            "1.1 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
        )],
        0,
    );
}
#[test]
fn desktop_t05_innerjoin() {
    check(
        "let\n    Orders = #table(type table [OrderId = Int64.Type, Cust = text, Amount = number], {{1,\"c1\",10},{2,\"c2\",20},{3,\"c9\",30},{4,null,40},{5,\"c1\",50},{6,\"C1\",60}}),\n    Customers = #table(type table [Cust = text, CustName = text, Tier = Int64.Type], {{\"c1\",\"One\",1},{\"c2\",\"Two\",2},{\"c2\",\"Two bis\",3},{null,\"Nobody\",0},{\"c3\",\"Three\",3}}),\n    Joined = Table.NestedJoin(Orders, {\"Cust\"}, Customers, {\"Cust\"}, \"C\", JoinKind.Inner),\n    Expanded = Table.ExpandTableColumn(Joined, \"C\", {\"CustName\"}, {\"Name\"})\nin\n    Expanded",
        &[
            ("OrderId", "int64"),
            ("Cust", "string"),
            ("Amount", "double"),
            ("Name", "string"),
        ],
        &[
            "1\tc1\t10.0\tOne",
            "2\tc2\t20.0\tTwo",
            "2\tc2\t20.0\tTwo bis",
            "4\t<null>\t40.0\tNobody",
            "5\tc1\t50.0\tOne",
        ],
        "en-US",
        true,
        &[(
            "Rate",
            "1.1 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
        )],
        0,
    );
}
#[test]
fn desktop_t05b_antijoin() {
    check(
        "let\n    Orders = #table(type table [OrderId = Int64.Type, Cust = text, Amount = number], {{1,\"c1\",10},{2,\"c2\",20},{3,\"c9\",30},{4,null,40},{5,\"c1\",50},{6,\"C1\",60}}),\n    Customers = #table(type table [Cust = text, CustName = text, Tier = Int64.Type], {{\"c1\",\"One\",1},{\"c2\",\"Two\",2},{\"c2\",\"Two bis\",3},{null,\"Nobody\",0},{\"c3\",\"Three\",3}}),\n    Joined = Table.NestedJoin(Orders, {\"Cust\"}, Customers, {\"Cust\"}, \"C\", JoinKind.LeftAnti),\n    Removed = Table.RemoveColumns(Joined, {\"C\"})\nin\n    Removed",
        &[("OrderId", "int64"), ("Cust", "string"), ("Amount", "double")],
        &["3\tc9\t30.0", "6\tC1\t60.0"],
        "en-US",
        true,
        &[(
            "Rate",
            "1.1 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
        )],
        0,
    );
}
#[test]
fn desktop_t06_distinct() {
    check(
        "let\n    A = #table(type table [K = text, V = Int64.Type], {{\"x\",1},{\"y\",2},{\"x\",1},{\"X\",1},{\"z\",3}}),\n    B = #table(type table [K = text, V = Int64.Type, W = text], {{\"w\",9,\"extra\"}}),\n    D = Table.Distinct(A),\n    C = Table.Combine({D, B}),\n    R1 = Table.ReplaceValue(C, \"x\", \"ex\", Replacer.ReplaceText, {\"K\"}),\n    R2 = Table.ReplaceValue(R1, 2, 22, Replacer.ReplaceValue, {\"V\"}),\n    R3 = Table.ReplaceValue(R2, null, \"none\", Replacer.ReplaceValue, {\"W\"}),\n    F = Table.FirstN(R3, 4),\n    I = Table.AddIndexColumn(F, \"Idx\", 0, 1, Int64.Type)\nin\n    I",
        &[("K", "string"), ("V", "int64"), ("W", "string"), ("Idx", "int64")],
        &[
            "ex\t1\tnone\t0",
            "y\t22\tnone\t1",
            "X\t1\tnone\t2",
            "z\t3\tnone\t3",
        ],
        "en-US",
        true,
        &[(
            "Rate",
            "1.1 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
        )],
        0,
    );
}
#[test]
fn desktop_t07_calendar() {
    check(
        "let\n    Dates = List.Dates(#date(2024, 2, 25), 10, #duration(1, 0, 0, 0)),\n    T = Table.FromList(Dates, Splitter.SplitByNothing(), {\"Date\"}),\n    Typed = Table.TransformColumnTypes(T, {{\"Date\", type date}}),\n    Y = Table.AddColumn(Typed, \"Year\", each Date.Year([Date]), Int64.Type),\n    Q = Table.AddColumn(Y, \"Quarter\", each \"Q\" & Number.ToText(Date.QuarterOfYear([Date])), type text),\n    W = Table.AddColumn(Q, \"Week\", each Date.WeekOfYear([Date]), Int64.Type),\n    D = Table.AddColumn(W, \"DayName\", each Date.DayOfWeekName([Date]), type text),\n    Dn = Table.AddColumn(D, \"Dow\", each Date.DayOfWeek([Date]), Int64.Type),\n    YM = Table.AddColumn(Dn, \"YM\", each Date.ToText([Date], \"yyyy-MM\"), type text),\n    SOW = Table.AddColumn(YM, \"SOW\", each Date.StartOfWeek([Date]), type date),\n    Doy = Table.AddColumn(SOW, \"Doy\", each Date.DayOfYear([Date]), Int64.Type),\n    Leap = Table.AddColumn(Doy, \"Leap\", each Date.IsLeapYear([Date]), type logical)\nin\n    Leap",
        &[
            ("Date", "dateTime"),
            ("Year", "int64"),
            ("Quarter", "string"),
            ("Week", "int64"),
            ("DayName", "string"),
            ("Dow", "int64"),
            ("YM", "string"),
            ("SOW", "dateTime"),
            ("Doy", "int64"),
            ("Leap", "boolean"),
        ],
        &[
            "2024-02-25T00:00:00.000\t2024\tQ1\t9\tSunday\t0\t2024-02\t2024-02-25T00:00:00.000\t56\ttrue",
            "2024-02-26T00:00:00.000\t2024\tQ1\t9\tMonday\t1\t2024-02\t2024-02-25T00:00:00.000\t57\ttrue",
            "2024-02-27T00:00:00.000\t2024\tQ1\t9\tTuesday\t2\t2024-02\t2024-02-25T00:00:00.000\t58\ttrue",
            "2024-02-28T00:00:00.000\t2024\tQ1\t9\tWednesday\t3\t2024-02\t2024-02-25T00:00:00.000\t59\ttrue",
            "2024-02-29T00:00:00.000\t2024\tQ1\t9\tThursday\t4\t2024-02\t2024-02-25T00:00:00.000\t60\ttrue",
            "2024-03-01T00:00:00.000\t2024\tQ1\t9\tFriday\t5\t2024-03\t2024-02-25T00:00:00.000\t61\ttrue",
            "2024-03-02T00:00:00.000\t2024\tQ1\t9\tSaturday\t6\t2024-03\t2024-02-25T00:00:00.000\t62\ttrue",
            "2024-03-03T00:00:00.000\t2024\tQ1\t10\tSunday\t0\t2024-03\t2024-03-03T00:00:00.000\t63\ttrue",
            "2024-03-04T00:00:00.000\t2024\tQ1\t10\tMonday\t1\t2024-03\t2024-03-03T00:00:00.000\t64\ttrue",
            "2024-03-05T00:00:00.000\t2024\tQ1\t10\tTuesday\t2\t2024-03\t2024-03-03T00:00:00.000\t65\ttrue",
        ],
        "en-US",
        true,
        &[(
            "Rate",
            "1.1 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
        )],
        0,
    );
}
#[test]
fn desktop_t08_numbers() {
    check(
        "let\n    N = List.Numbers(-3, 13, 0.5),\n    T = Table.FromList(N, Splitter.SplitByNothing(), {\"N\"}),\n    T1 = Table.TransformColumnTypes(T, {{\"N\", type number}}),\n    R = Table.AddColumn(T1, \"Round\", each Number.Round([N]), type number),\n    RA = Table.AddColumn(R, \"RoundAway\", each Number.Round([N], 0, RoundingMode.AwayFromZero), type number),\n    Dup = Table.DuplicateColumn(RA, \"N\", \"AsInt\"),\n    I = Table.TransformColumnTypes(Dup, {{\"AsInt\", Int64.Type}}),\n    Cur = Table.TransformColumnTypes(Table.AddColumn(I, \"Cur\", each [N] / 3), {{\"Cur\", Currency.Type}}),\n    Mod = Table.AddColumn(Cur, \"Mod\", each Number.Mod([N] * 2, 3), type number),\n    Div = Table.AddColumn(Mod, \"Div\", each Number.IntegerDivide([N] * 2, 3), type number),\n    Try = Table.AddColumn(Div, \"Try\", each try (if [N] = 0 then error \"zero\" else 1 / [N]) otherwise -1, type number),\n    Err = Table.AddColumn(Try, \"Err\", each if [N] > 2 then error \"boom\" else [N] * 10, type number),\n    Safe = Table.AddColumn(Err, \"Safe\", each try Number.FromText(if [N] > 0 then \"x\" else Text.From([N])) otherwise -99, type number),\n    Txt = Table.AddColumn(Safe, \"Txt\", each Number.ToText([N], \"F2\") & \"|\" & Text.From([N] / 3), type text),\n    Pad = Table.AddColumn(Txt, \"Pad\", each Text.PadStart(Text.From(Number.Abs([N] * 2)), 3, \"0\"), type text)\nin\n    Pad",
        &[
            ("N", "double"),
            ("Round", "double"),
            ("RoundAway", "double"),
            ("AsInt", "int64"),
            ("Cur", "decimal"),
            ("Mod", "double"),
            ("Div", "double"),
            ("Try", "double"),
            ("Err", "double"),
            ("Safe", "double"),
            ("Txt", "string"),
            ("Pad", "string"),
        ],
        &[
            "-3.0\t-3.0\t-3.0\t-3\t-1\t-0.0\t-2.0\t-0.3333333333333333\t-30.0\t-3.0\t-3.00|-1\t006",
            "-2.5\t-2.0\t-3.0\t-2\t-0.8333\t-2.0\t-1.0\t-0.4\t-25.0\t-2.5\t-2.50|-0.83333333333333337\t005",
            "-2.0\t-2.0\t-2.0\t-2\t-0.6667\t-1.0\t-1.0\t-0.5\t-20.0\t-2.0\t-2.00|-0.66666666666666663\t004",
            "-1.5\t-2.0\t-2.0\t-2\t-0.5\t-0.0\t-1.0\t-0.6666666666666666\t-15.0\t-1.5\t-1.50|-0.5\t003",
            "-1.0\t-1.0\t-1.0\t-1\t-0.3333\t-2.0\t-0.0\t-1.0\t-10.0\t-1.0\t-1.00|-0.33333333333333331\t002",
            "-0.5\t-0.0\t-1.0\t0\t-0.1667\t-1.0\t-0.0\t-2.0\t-5.0\t-0.5\t-0.50|-0.16666666666666666\t001",
            "0.0\t0.0\t0.0\t0\t0\t0.0\t0.0\t-1.0\t0.0\t0.0\t0.00|0\t000",
            "0.5\t0.0\t1.0\t0\t0.1667\t1.0\t0.0\t2.0\t5.0\t-99.0\t0.50|0.16666666666666666\t001",
            "1.0\t1.0\t1.0\t1\t0.3333\t2.0\t0.0\t1.0\t10.0\t-99.0\t1.00|0.33333333333333331\t002",
            "1.5\t2.0\t2.0\t2\t0.5\t0.0\t1.0\t0.6666666666666666\t15.0\t-99.0\t1.50|0.5\t003",
            "2.0\t2.0\t2.0\t2\t0.6667\t1.0\t1.0\t0.5\t20.0\t-99.0\t2.00|0.66666666666666663\t004",
            "2.5\t2.0\t3.0\t2\t0.8333\t2.0\t1.0\t0.4\t<null>\t-99.0\t2.50|0.83333333333333337\t005",
            "3.0\t3.0\t3.0\t3\t1\t0.0\t2.0\t0.3333333333333333\t<null>\t-99.0\t3.00|1\t006",
        ],
        "en-US",
        true,
        &[(
            "Rate",
            "1.1 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
        )],
        0,
    );
}
#[test]
fn desktop_t10_sorttext() {
    check(
        "let\n    S = #table(type table [S = text], {{\"b\"},{\"B\"},{\"a\"},{\"A\"},{\"é\"},{\"e\"},{\"Z\"},{\"_x\"},{\"10\"},{\"9\"},{\"ab\"},{\"Ab\"}}),\n    Sorted = Table.Sort(S, {{\"S\", Order.Ascending}}),\n    I = Table.AddIndexColumn(Sorted, \"Idx\", 0, 1, Int64.Type)\nin\n    I",
        &[("S", "string"), ("Idx", "int64")],
        &[
            "10\t0", "9\t1", "A\t2", "Ab\t3", "B\t4", "Z\t5", "_x\t6", "a\t7", "ab\t8", "b\t9", "e\t10",
            "é\t11",
        ],
        "en-US",
        true,
        &[(
            "Rate",
            "1.1 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
        )],
        0,
    );
}
#[test]
fn desktop_t11_text() {
    check(
        "let\n    S = #table(type table [S = text], {{\"  Hello World  \"},{\"foo-bar-baz\"},{\"ÉCOLE élève\"},{\"\"},{null},{\"a,b,,c\"}}),\n    A = Table.AddColumn(S, \"Upper\", each Text.Upper([S]), type text),\n    B = Table.AddColumn(A, \"Proper\", each Text.Proper([S]), type text),\n    C = Table.AddColumn(B, \"Trim\", each Text.Trim([S]), type text),\n    D = Table.AddColumn(C, \"Len\", each Text.Length([S]), Int64.Type),\n    E = Table.AddColumn(D, \"Start\", each Text.Start([S], 4), type text),\n    F = Table.AddColumn(E, \"End\", each Text.End([S], 3), type text),\n    G = Table.AddColumn(F, \"Mid\", each try Text.Middle([S], 2, 4) otherwise \"ERR\", type text),\n    H = Table.AddColumn(G, \"Repl\", each Text.Replace([S], \"o\", \"0\"), type text),\n    I = Table.AddColumn(H, \"HasO\", each Text.Contains([S], \"o\"), type logical),\n    J = Table.AddColumn(I, \"Before\", each Text.BeforeDelimiter([S], \"-\"), type text),\n    K = Table.AddColumn(J, \"After\", each Text.AfterDelimiter([S], \"-\", 1), type text),\n    L = Table.AddColumn(K, \"Parts\", each if [S] = null then null else List.Count(Text.Split([S], \",\")), Int64.Type),\n    M = Table.AddColumn(L, \"Combined\", each Text.Combine({[S], \"!\", null, \"?\"}, \"|\"), type text),\n    N = Table.AddColumn(M, \"Pos\", each if [S] = null then null else Text.PositionOf([S], \"o\"), Int64.Type),\n    O = Table.AddColumn(N, \"Lower\", each Text.Lower([S]), type text)\nin\n    O",
        &[
            ("S", "string"),
            ("Upper", "string"),
            ("Proper", "string"),
            ("Trim", "string"),
            ("Len", "int64"),
            ("Start", "string"),
            ("End", "string"),
            ("Mid", "string"),
            ("Repl", "string"),
            ("HasO", "boolean"),
            ("Before", "string"),
            ("After", "string"),
            ("Parts", "int64"),
            ("Combined", "string"),
            ("Pos", "int64"),
            ("Lower", "string"),
        ],
        &[
            "  Hello World  \t  HELLO WORLD  \t  Hello World  \tHello World\t15\t  He\td  \tHell\t  Hell0 W0rld  \ttrue\t  Hello World  \t\t1\t  Hello World  |!|?\t6\t  hello world  ",
            "foo-bar-baz\tFOO-BAR-BAZ\tFoo-Bar-Baz\tfoo-bar-baz\t11\tfoo-\tbaz\to-ba\tf00-bar-baz\ttrue\tfoo\tbaz\t1\tfoo-bar-baz|!|?\t1\tfoo-bar-baz",
            "ÉCOLE élève\tÉCOLE ÉLÈVE\tÉcole Élève\tÉCOLE élève\t11\tÉCOL\tève\tOLE \tÉCOLE élève\tfalse\tÉCOLE élève\t\t1\tÉCOLE élève|!|?\t-1\técole élève",
            "\t\t\t\t0\t\t\t\t\tfalse\t\t\t1\t|!|?\t-1\t",
            "<null>\t<null>\t<null>\t<null>\t<null>\t<null>\t<null>\t<null>\t<null>\t<null>\t\t\t<null>\t!|?\t<null>\t<null>",
            "a,b,,c\tA,B,,C\tA,B,,C\ta,b,,c\t6\ta,b,\t,,c\tb,,c\ta,b,,c\tfalse\ta,b,,c\t\t4\ta,b,,c|!|?\t-1\ta,b,,c",
        ],
        "en-US",
        true,
        &[(
            "Rate",
            "1.1 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
        )],
        0,
    );
}
#[test]
fn desktop_t12_totext() {
    check(
        "let\n    texts = {\n        Text.From(1/3),\n        Text.From(0.1 + 0.2),\n        Text.From(1e20),\n        Text.From(123456789012345678),\n        Text.From(1.5),\n        Text.From(-0.000001),\n        Text.From(2.5e-7),\n        Text.From(12345678.9),\n        Number.ToText(1234.5, \"N2\"),\n        Number.ToText(0.256, \"P1\"),\n        Number.ToText(42, \"D5\"),\n        Number.ToText(3.14159, \"0.00\"),\n        Number.ToText(1234567.891, \"#,##0.0\"),\n        Text.From(#date(2026,1,5)),\n        Text.From(#datetime(2026,1,5,13,4,5)),\n        Text.From(#datetime(2026,1,5,0,0,0)),\n        Text.From(#time(9,5,0)),\n        Text.From(#duration(1,2,3,4)),\n        Text.From(#duration(0,0,0,1.5)),\n        Text.From(true),\n        Text.From(#datetimezone(2026,1,5,13,4,5,2,0)),\n        Date.ToText(#date(2026,1,5), \"dddd d MMMM yyyy\"),\n        DateTime.ToText(#datetime(2026,1,5,13,4,5), \"yyyy-MM-ddTHH:mm:ss\"),\n        Text.From(-1/3),\n        Text.From(1e15),\n        Text.From(1e-5),\n        Text.From(123456789.123),\n        Text.From(100),\n        Number.ToText(-1234.5, \"N0\"),\n        Number.ToText(0.5, \"F0\"),\n        Number.ToText(1.5, \"F0\"),\n        Number.ToText(2.5, \"F0\"),\n        Text.From(#datetime(2026,1,5,0,0,0.25)),\n        Date.ToText(#date(2026,1,5)),\n        Number.ToText(1e21),\n        Text.From(Number.Round(2.675, 2)),\n        Text.From(1/7*7),\n        Text.From(9007199254740993)\n    },\n    T = Table.FromRows(List.Zip({texts, List.Numbers(0, List.Count(texts))}), type table [S = text, I = Int64.Type])\nin\n    T",
        &[("S", "string"), ("I", "int64")],
        &[
            "0.33333333333333331\t0",
            "0.30000000000000004\t1",
            "100000000000000000000\t2",
            "123456789012345678\t3",
            "1.5\t4",
            "-1E-06\t5",
            "2.5E-07\t6",
            "12345678.9\t7",
            "1,234.50\t8",
            "25.6%\t9",
            "00042\t10",
            "3.14\t11",
            "1,234,567.9\t12",
            "1/5/2026\t13",
            "1/5/2026 1:04:05 PM\t14",
            "1/5/2026 12:00:00 AM\t15",
            "9:05 AM\t16",
            "1.02:03:04\t17",
            "00:00:01.5000000\t18",
            "true\t19",
            "1/5/2026 1:04:05 PM +02:00\t20",
            "Monday 5 January 2026\t21",
            "2026-01-05T13:04:05\t22",
            "-0.33333333333333331\t23",
            "1000000000000000\t24",
            "1E-05\t25",
            "123456789.123\t26",
            "100\t27",
            "-1,235\t28",
            "1\t29",
            "2\t30",
            "3\t31",
            "1/5/2026 12:00:00 AM\t32",
            "1/5/2026\t33",
            "1000000000000000000000\t34",
            "2.680\t35",
            "1\t36",
            "9007199254740993\t37",
        ],
        "en-US",
        true,
        &[(
            "Rate",
            "1.1 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
        )],
        0,
    );
}
#[test]
fn desktop_t13_kinds() {
    check(
        "#table(type table [I = Int64.Type, D = duration, Z = datetimezone, T = time, DT = datetime], {\n    {1, #duration(1,12,0,0), #datetimezone(2026,1,5,13,0,0,2,0), #time(6,30,0), #datetime(2026,1,5,6,30,15.25)},\n    {2, #duration(0,0,0,-30), #datetimezone(2026,1,5,0,30,0,-5,0), #time(23,59,59), #datetime(1899,12,30,0,0,0)},\n    {3, #duration(0,0,0,0.5), #datetimezone(1999,12,31,23,0,0,0,0), #time(0,0,0), #datetime(1800,1,1,12,0,0)}\n})",
        &[
            ("I", "int64"),
            ("D", "double"),
            ("Z", "dateTime"),
            ("T", "dateTime"),
            ("DT", "dateTime"),
        ],
        &[
            "1\t1.5\t2026-01-05T13:00:00.000\t1899-12-30T06:30:00.000\t2026-01-05T06:30:15.250",
            "2\t-0.00034722222222222224\t2026-01-05T00:30:00.000\t1899-12-30T23:59:59.000\t1899-12-30T00:00:00.000",
            "3\t5.787037037037037e-6\t1999-12-31T23:00:00.000\t1899-12-30T00:00:00.000\t1800-01-01T12:00:00.000",
        ],
        "en-US",
        true,
        &[(
            "Rate",
            "1.1 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
        )],
        0,
    );
}
#[test]
fn desktop_t14_csvtyped() {
    check(
        "let\n    Source = Csv.Document(File.Contents(\"{root}sales.csv\"),[Delimiter=\",\", Columns=5, Encoding=65001, QuoteStyle=QuoteStyle.Csv]),\n    #\"Promoted Headers\" = Table.PromoteHeaders(Source, [PromoteAllScalars=true]),\n    #\"Changed Type\" = Table.TransformColumnTypes(#\"Promoted Headers\",{{\"Id\", Int64.Type}, {\"Name\", type text}, {\"Amount\", type number}, {\"Date\", type date}, {\"Note\", type text}})\nin\n    #\"Changed Type\"",
        &[
            ("Id", "int64"),
            ("Name", "string"),
            ("Amount", "double"),
            ("Date", "dateTime"),
            ("Note", "string"),
        ],
        &[
            "1\tSmith, John\t12.5\t2026-01-05T00:00:00.000\tplain",
            "2\tQuote \"Q\"\t-3.0\t2026-01-31T00:00:00.000\tmulti\\r\\nline",
            "3\t\t0.25\t2026-02-28T00:00:00.000\t",
            "4\tLast\t1000.0\t2026-03-01T00:00:00.000\textra",
            "5\tShort\t7.0\t<null>\t",
            "6\t padded \t1.0\t2026-03-02T00:00:00.000\t x ",
        ],
        "en-US",
        true,
        &[(
            "Rate",
            "1.1 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
        )],
        0,
    );
}
#[test]
fn desktop_t14b_csvrawnone() {
    check(
        "let\n    Source = Csv.Document(File.Contents(\"{root}sales.csv\"),[Delimiter=\",\", Columns=5, Encoding=65001, QuoteStyle=QuoteStyle.None]),\n    I = Table.AddIndexColumn(Source, \"Idx\", 0, 1, Int64.Type)\nin\n    I",
        &[
            ("Column1", "string"),
            ("Column2", "string"),
            ("Column3", "string"),
            ("Column4", "string"),
            ("Column5", "string"),
            ("Idx", "int64"),
        ],
        &[
            "Id\tName\tAmount\tDate\tNote\t0",
            "1\tSmith, John\t12.5\t2026-01-05\tplain\t1",
            "2\tQuote \"Q\"\t-3\t01/31/2026\tmulti\t2",
            "line\"\t\t\t\t\t3",
            "3\t\t0.25\t2026-02-28\t\t4",
            "4\tLast\t1e3\t2026-03-01\textra\t5",
            "5\tShort\t7\t\t\t6",
            "6\t padded \t1\t2026-03-02\t x \t7",
        ],
        "en-US",
        true,
        &[(
            "Rate",
            "1.1 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
        )],
        0,
    );
}
#[test]
fn desktop_t14c_csvrawcsv() {
    check(
        "let\n    Source = Csv.Document(File.Contents(\"{root}sales.csv\"),[Delimiter=\",\", Columns=5, Encoding=65001, QuoteStyle=QuoteStyle.Csv]),\n    I = Table.AddIndexColumn(Source, \"Idx\", 0, 1, Int64.Type)\nin\n    I",
        &[
            ("Column1", "string"),
            ("Column2", "string"),
            ("Column3", "string"),
            ("Column4", "string"),
            ("Column5", "string"),
            ("Idx", "int64"),
        ],
        &[
            "Id\tName\tAmount\tDate\tNote\t0",
            "1\tSmith, John\t12.5\t2026-01-05\tplain\t1",
            "2\tQuote \"Q\"\t-3\t01/31/2026\tmulti\\r\\nline\t2",
            "3\t\t0.25\t2026-02-28\t\t3",
            "4\tLast\t1e3\t2026-03-01\textra\t4",
            "5\tShort\t7\t\t\t5",
            "6\t padded \t1\t2026-03-02\t x \t6",
        ],
        "en-US",
        true,
        &[(
            "Rate",
            "1.1 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
        )],
        0,
    );
}
#[test]
fn desktop_t15_json() {
    check(
        "let\n    Source = Json.Document(File.Contents(\"{root}people.json\")),\n    #\"Converted to Table\" = Table.FromList(Source, Splitter.SplitByNothing(), null, null, ExtraValues.Error),\n    #\"Expanded\" = Table.ExpandRecordColumn(#\"Converted to Table\", \"Column1\", {\"name\", \"age\", \"active\", \"joined\", \"score\"}, {\"name\", \"age\", \"active\", \"joined\", \"score\"}),\n    #\"Changed Type\" = Table.TransformColumnTypes(#\"Expanded\",{{\"name\", type text}, {\"age\", Int64.Type}, {\"active\", type logical}, {\"joined\", type datetime}, {\"score\", type number}})\nin\n    #\"Changed Type\"",
        &[
            ("name", "string"),
            ("age", "int64"),
            ("active", "boolean"),
            ("joined", "dateTime"),
            ("score", "double"),
        ],
        &[
            "Ann\t31\ttrue\t2024-05-01T00:00:00.000\t1.25",
            "Bob\t<null>\tfalse\t2023-12-31T10:00:00.000\t-3.0",
            "Céé\t40\ttrue\t2026-01-05T23:59:59.500\t0.001",
        ],
        "en-US",
        true,
        &[(
            "Rate",
            "1.1 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
        )],
        0,
    );
}
#[test]
fn desktop_t16_excel() {
    check(
        "let\n    Source = Excel.Workbook(File.Contents(\"{root}book.xlsx\"), null, true),\n    Data_Sheet = Source{[Item=\"Data\",Kind=\"Sheet\"]}[Data],\n    #\"Promoted Headers\" = Table.PromoteHeaders(Data_Sheet, [PromoteAllScalars=true]),\n    #\"Changed Type\" = Table.TransformColumnTypes(#\"Promoted Headers\",{{\"Name\", type text}, {\"Qty\", Int64.Type}, {\"When\", type date}, {\"Ok\", type logical}, {\"Price\", type number}})\nin\n    #\"Changed Type\"",
        &[
            ("Name", "string"),
            ("Qty", "int64"),
            ("When", "dateTime"),
            ("Ok", "boolean"),
            ("Price", "double"),
        ],
        &[
            "Widget\t3\t2026-01-05T00:00:00.000\ttrue\t1.5",
            "Gadget\t12\t2024-12-31T00:00:00.000\tfalse\t24.0",
            "Inline <s>\t<null>\t1900-03-01T00:00:00.000\t<null>\t-0.001",
            "<null>\t<null>\t<null>\t<null>\t<null>",
            "Gizmo & co\t7\t<null>\ttrue\t9.5",
        ],
        "en-US",
        true,
        &[(
            "Rate",
            "1.1 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
        )],
        0,
    );
}
#[test]
fn desktop_t16b_exceloffset() {
    check(
        "let\n    Source = Excel.Workbook(File.Contents(\"{root}book.xlsx\"), null, true),\n    Other = Source{[Item=\"Other\",Kind=\"Sheet\"]}[Data],\n    Cols = Table.FromRows({{Text.Combine(Table.ColumnNames(Other), \",\"), Table.RowCount(Other)}}, type table [Names = text, Rows = Int64.Type])\nin\n    Cols",
        &[("Names", "string"), ("Rows", "int64")],
        &["Column1,Column2,Column3\t2"],
        "en-US",
        true,
        &[(
            "Rate",
            "1.1 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
        )],
        0,
    );
}
#[test]
fn desktop_t17_reshape() {
    check(
        "let\n    S = #table(type table [Region = text, Jan = number, Feb = number, Code = text], {{\"North\", 10, 20, \"A-1\"}, {null, 5, null, \"B-2\"}, {\"South\", null, 7.5, \"C\"}, {null, 1, 2, \"D-4-x\"}}),\n    F = Table.FillDown(S, {\"Region\"}),\n    Sp = Table.SplitColumn(F, \"Code\", Splitter.SplitTextByDelimiter(\"-\", QuoteStyle.Csv), {\"Code.1\", \"Code.2\"}),\n    U = Table.UnpivotOtherColumns(Sp, {\"Region\", \"Code.1\", \"Code.2\"}, \"Month\", \"Value\"),\n    R = Table.AddColumn(U, \"Scaled\", each [Value] * Rate, type number),\n    T = Table.TransformColumnTypes(R, {{\"Value\", type number}})\nin\n    T",
        &[
            ("Region", "string"),
            ("Code.1", "string"),
            ("Code.2", "string"),
            ("Month", "string"),
            ("Value", "double"),
            ("Scaled", "double"),
        ],
        &[
            "North\tA\t1\tJan\t10.0\t11.0",
            "North\tA\t1\tFeb\t20.0\t22.0",
            "North\tB\t2\tJan\t5.0\t5.5",
            "South\tC\t<null>\tFeb\t7.5\t8.25",
            "South\tD\t4\tJan\t1.0\t1.1",
            "South\tD\t4\tFeb\t2.0\t2.2",
        ],
        "en-US",
        true,
        &[(
            "Rate",
            "1.1 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
        )],
        0,
    );
}
#[test]
fn desktop_t18_frculture() {
    check(
        "let\n    S = #table(type table [A = text, B = text], {{\"1,5\", \"05/01/2026\"}, {\"1 234,25\", \"31/12/2025\"}, {\"-0,5\", \"1/2/2026\"}, {\"2.5\", \"2026-03-04\"}}),\n    T = Table.TransformColumnTypes(S, {{\"A\", type number}, {\"B\", type date}}, \"fr-FR\"),\n    V = Table.AddColumn(T, \"BT\", each Date.ToText([B], [Format=\"dd MMMM yyyy\", Culture=\"fr-FR\"]), type text),\n    W = Table.AddColumn(V, \"AT\", each Number.ToText([A], \"F1\", \"fr-FR\"), type text)\nin\n    W",
        &[
            ("A", "double"),
            ("B", "dateTime"),
            ("BT", "string"),
            ("AT", "string"),
        ],
        &[
            "1.5\t2026-01-05T00:00:00.000\t05 janvier 2026\t1,5",
            "<null>\t2025-12-31T00:00:00.000\t31 décembre 2025\t<null>",
            "-0.5\t2026-02-01T00:00:00.000\t01 février 2026\t-0,5",
            "<null>\t2026-03-04T00:00:00.000\t04 mars 2026\t<null>",
        ],
        "en-US",
        true,
        &[(
            "Rate",
            "1.1 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
        )],
        0,
    );
}
#[test]
fn desktop_t19_exprs() {
    check(
        "let\n    r = [a = 1, b = a + 1],\n    items = {\n        {\"rec\", Text.From(r[b])},\n        {\"opt\", Text.From(r[z]? ?? \"none\")},\n        {\"listsum\", Text.From(List.Sum({1, 2, null, 3.5}))},\n        {\"listavg\", Text.From(List.Average({1, 2, 4}))},\n        {\"listmax\", Text.From(List.Max({3, 9, 1}))},\n        {\"range\", Text.From(List.Count({1..10}))},\n        {\"transform\", Text.Combine(List.Transform({1, 2, 3}, each Text.From(_ * _)), \",\")},\n        {\"select\", Text.Combine(List.Transform(List.Select({1..10}, each Number.IsEven(_)), Text.From), \",\")},\n        {\"tryrec\", (try error \"boom\")[Error][Message]},\n        {\"tryreason\", (try Number.FromText(\"x\"))[Error][Reason]},\n        {\"distinct\", Text.Combine(List.Distinct({\"a\", \"A\", \"b\", \"a\"}), \",\")},\n        {\"sortl\", Text.Combine(List.Sort({\"b\", \"a\", \"C\", \"B\"}), \",\")},\n        {\"fmt\", Text.Format(\"#{0} and #{1}\", {\"x\", 2})},\n        {\"accum\", Text.From(List.Accumulate({1, 2, 3, 4}, 0, (s, x) => s + x))},\n        {\"datediff\", Text.From(Duration.Days(#date(2026, 3, 1) - #date(2026, 1, 1)))},\n        {\"dtadd\", Text.From(#datetime(2026, 1, 31, 23, 0, 0) + #duration(0, 2, 0, 0))},\n        {\"addmonths\", Text.From(Date.AddMonths(#date(2026, 1, 31), 1))},\n        {\"proper\", Text.Proper(\"hello o'neil mcdonald-smith 2nd\")},\n        {\"pos\", Text.From(List.PositionOf({\"x\", \"y\", \"z\"}, \"z\"))},\n        {\"zip\", Text.From(List.Count(List.Zip({{1, 2}, {3}})))},\n        {\"recfun\", Text.From(let f = (n) => if n <= 1 then 1 else n * @f(n - 1) in f(10))},\n        {\"numfromtext\", Text.From(Number.FromText(\"  12.5  \"))},\n        {\"datefromtext\", Text.From(Date.FromText(\"2026-07-04\"))},\n        {\"int64\", Text.From(Int64.From(2.5))},\n        {\"int64b\", Text.From(Int64.From(3.5))},\n        {\"round2\", Text.From(Number.Round(1.005, 2))},\n        {\"roundneg\", Text.From(Number.Round(-2.5))},\n        {\"mod\", Text.From(Number.Mod(-7, 3))},\n        {\"idiv\", Text.From(Number.IntegerDivide(-7, 2))},\n        {\"textnull\", Text.From(null & \"x\" = null)},\n        {\"cmpnull\", Text.From((null < 1) = null)},\n        {\"weekofyear\", Text.From(Date.WeekOfYear(#date(2026, 1, 3)))},\n        {\"logicaltext\", Text.From(Logical.FromText(\"TRUE\"))},\n        {\"before_nf\", Text.BeforeDelimiter(\"abc\", \"-\")},\n        {\"after_nf\", Text.AfterDelimiter(\"abc\", \"-\")},\n        {\"trimchars\", Text.Trim(\"xxhixx\", \"x\")},\n        {\"split\", Text.Combine(Text.Split(\"a--b\", \"-\"), \"|\")},\n        {\"splitter\", Text.Combine(Splitter.SplitTextByDelimiter(\",\", QuoteStyle.Csv)(\"a,\"\"b,c\"\",d\"), \"|\")},\n        {\"combiner\", Combiner.CombineTextByDelimiter(\"-\")({\"a\", null, \"c\"})},\n        {\"lsum_empty\", Text.From(List.Sum({}))},\n        {\"lsum_nulls\", Text.From(List.Sum({null, null}))},\n        {\"lmin_empty\", Text.From(List.Min({}))},\n        {\"lavg_nulls\", Text.From(List.Average({null}))},\n        {\"dow_name\", Date.DayOfWeekName(#date(2026,1,5))},\n        {\"dur_text\", Duration.ToText(#duration(-1,-2,0,0))},\n        {\"time_text\", Time.ToText(#time(13,5,9))},\n        {\"number_from_date\", Text.From(Number.From(#date(2026,1,5)))},\n        {\"number_from_dt\", Text.From(Number.From(#datetime(2026,1,5,12,0,0)))},\n        {\"date_from_num\", Text.From(Date.From(46027.75))},\n        {\"text_from_list_err\", try Text.From({1}) otherwise \"err\"},\n        {\"value_is\", Text.From(1 is number)},\n        {\"int_div\", Text.From(7 / 2)},\n        {\"neg_zero\", Text.From(-0.0)},\n        {\"big_mult\", Text.From(99999999 * 99999999)}\n    },\n    T = Table.FromRows(items, type table [K = text, V = text])\nin\n    T",
        &[("K", "string"), ("V", "string")],
        &[
            "rec\t2",
            "opt\tnone",
            "listsum\t6.5",
            "listavg\t2.333333333333333",
            "listmax\t9",
            "range\t10",
            "transform\t1,4,9",
            "select\t2,4,6,8,10",
            "tryrec\tboom",
            "tryreason\tDataFormat.Error",
            "distinct\ta,A,b",
            "sortl\tB,C,a,b",
            "fmt\tx and 2",
            "accum\t10",
            "datediff\t59",
            "dtadd\t2/1/2026 1:00:00 AM",
            "addmonths\t2/28/2026",
            "proper\tHello O'Neil Mcdonald-Smith 2Nd",
            "pos\t2",
            "zip\t2",
            "recfun\t3628800",
            "numfromtext\t12.5",
            "datefromtext\t7/4/2026",
            "int64\t2",
            "int64b\t4",
            "round2\t1.000",
            "roundneg\t-2.0",
            "mod\t-1",
            "idiv\t-3",
            "textnull\ttrue",
            "cmpnull\ttrue",
            "weekofyear\t1",
            "logicaltext\ttrue",
            "before_nf\tabc",
            "after_nf\t",
            "trimchars\thi",
            "split\ta||b",
            "splitter\ta|b,c|d",
            "combiner\ta--c",
            "lsum_empty\t<null>",
            "lsum_nulls\t<null>",
            "lmin_empty\t<null>",
            "lavg_nulls\t<null>",
            "dow_name\tMonday",
            "dur_text\t-1.02:00:00",
            "time_text\t1:05 PM",
            "number_from_date\t46027",
            "number_from_dt\t46027.5",
            "date_from_num\t1/5/2026",
            "text_from_list_err\terr",
            "value_is\ttrue",
            "int_div\t3.5",
            "neg_zero\t0",
            "big_mult\t9999999800000000",
        ],
        "en-US",
        true,
        &[(
            "Rate",
            "1.1 meta [IsParameterQuery=true, Type=\"Number\", IsParameterQueryRequired=true]",
        )],
        0,
    );
}
#[test]
fn desktop_r01_frdefault() {
    check(
        "let\n    Source = Table.FromRows(Json.Document(Binary.Decompress(Binary.FromText(\"i45WMtQxVdJRMjDVNzDUNzIwMgNySopKU5VidaKVdI10jECyxob6hkYgWRAnLTGnGCJtaGBgABQA6dI1MNI1MAZyQoJCXcGSBjqGYOVAU2HmuiF0HlpgZGwCttkQaLM5SIUJ3OZYAA==\", BinaryEncoding.Base64), Compression.Deflate)), let _t = ((type nullable text) meta [Serialized.Text = true]) in type table [A = _t, B = _t, C = _t]),\n    Typed = Table.TransformColumnTypes(Source, {{\"A\", type number}, {\"B\", type date}, {\"C\", type logical}})\nin\n    Typed",
        &[("A", "double"), ("B", "dateTime"), ("C", "boolean")],
        &[
            "1.5\t2026-01-05T00:00:00.000\ttrue",
            "-2.25\t2025-12-31T00:00:00.000\tfalse",
            "1000.0\t2026-02-03T00:00:00.000\ttrue",
            "0.125\t2026-02-01T00:00:00.000\tfalse",
            "<null>\t2024-07-15T00:00:00.000\ttrue",
        ],
        "fr-FR",
        true,
        &[],
        0,
    );
}
#[test]
fn desktop_r02_frtext() {
    check(
        "let\n    texts = {\n        Text.From(1.5),\n        Text.From(#date(2026,1,5)),\n        Text.From(#datetime(2026,1,5,13,4,5)),\n        Text.From(#time(9,5,0)),\n        Date.DayOfWeekName(#date(2026,1,5)),\n        Date.MonthName(#date(2026,8,5)),\n        Text.From(Date.DayOfWeek(#date(2026,1,4))),\n        Text.From(Date.WeekOfYear(#date(2026,1,4))),\n        Text.From(Date.WeekOfYear(#date(2026,1,5))),\n        Number.ToText(1234.5, \"N2\"),\n        Number.ToText(0.256, \"P1\"),\n        Text.From(1e-5),\n        Text.From(0.1 + 0.2),\n        Number.ToText(2.675, \"F2\"),\n        Number.ToText(-1234.5, \"F0\"),\n        Date.ToText(#date(2026,1,5), \"dddd d MMMM yyyy\"),\n        Text.From(true),\n        Text.From(#duration(1,2,3,4)),\n        Text.From(#datetimezone(2026,1,5,13,4,5,2,0)),\n        Text.From(-0.5),\n        Text.From(Date.StartOfWeek(#date(2026,1,7))),\n        Text.From(Date.EndOfMonth(#datetime(2024,2,10,8,0,0))),\n        Date.ToText(#date(2026,1,5), \"MMM\"),\n        Date.ToText(#date(2026,1,5), \"ddd\"),\n        Time.ToText(#time(13,5,9)),\n        Number.ToText(1234567.891, \"#,##0.00\"),\n        Number.ToText(0.5, \"0%\")\n    },\n    T = Table.FromRows(List.Zip({texts, List.Numbers(0, List.Count(texts))}), type table [S = text, I = Int64.Type])\nin\n    T",
        &[("S", "string"), ("I", "int64")],
        &[
            "1,5\t0",
            "05/01/2026\t1",
            "05/01/2026 13:04:05\t2",
            "09:05\t3",
            "lundi\t4",
            "août\t5",
            "6\t6",
            "1\t7",
            "2\t8",
            "1 234,50\t9",
            "25,6 %\t10",
            "1E-05\t11",
            "0,30000000000000004\t12",
            "2,68\t13",
            "-1235\t14",
            "lundi 5 janvier 2026\t15",
            "true\t16",
            "1.02:03:04\t17",
            "05/01/2026 13:04:05 +02:00\t18",
            "-0,5\t19",
            "05/01/2026\t20",
            "29/02/2024 23:59:59\t21",
            "janv.\t22",
            "lun.\t23",
            "13:05\t24",
            "1 234 567,89\t25",
            "50%\t26",
        ],
        "fr-FR",
        true,
        &[],
        0,
    );
}
#[test]
fn desktop_r03_errorfails() {
    check(
        "Table.TransformColumnTypes(#table({\"A\"}, {{\"1\"}, {\"x\"}}), {{\"A\", Int64.Type}})",
        &[("A", "int64")],
        &["1", "<null>"],
        "fr-FR",
        true,
        &[],
        0,
    );
}
#[test]
fn desktop_r04_pivot() {
    check(
        "let\n    S = #table(type table [K = text, C = text, V = number], {{\"a\",\"x\",1},{\"a\",\"y\",2},{\"b\",\"x\",3},{\"b\",\"x\",4},{\"c\",\"z\",5}}),\n    P = Table.Pivot(S, {\"x\",\"y\",\"z\"}, \"C\", \"V\", List.Sum),\n    I = Table.AddIndexColumn(P, \"Idx\", 0, 1, Int64.Type)\nin\n    I",
        &[
            ("K", "string"),
            ("x", "double"),
            ("y", "double"),
            ("z", "double"),
            ("Idx", "int64"),
        ],
        &[
            "a\t1.0\t2.0\t<null>\t0",
            "b\t7.0\t<null>\t<null>\t1",
            "c\t<null>\t<null>\t5.0\t2",
        ],
        "fr-FR",
        true,
        &[],
        0,
    );
}
#[test]
fn desktop_r05_lists() {
    check(
        "let\n    S = #table(type table [Id = Int64.Type, L = list], {{1, {\"a\",\"b\"}}, {2, {}}, {3, {\"c\"}}}),\n    E = Table.ExpandListColumn(S, \"L\"),\n    T = Table.TransformColumns(E, {{\"L\", each if _ = null then \"none\" else Text.Upper(_), type text}}),\n    F = Table.AddColumn(T, \"Fmt\", each Text.Format(\"#{0}:#{1}\", {[Id], [L]}), type text),\n    I = Table.AddIndexColumn(F, \"Idx\", 0, 1, Int64.Type)\nin\n    I",
        &[
            ("Id", "int64"),
            ("L", "string"),
            ("Fmt", "string"),
            ("Idx", "int64"),
        ],
        &[
            "1\tA\t1:A\t0",
            "1\tB\t1:B\t1",
            "2\tnone\t2:none\t2",
            "3\tC\t3:C\t3",
        ],
        "fr-FR",
        true,
        &[],
        0,
    );
}
#[test]
fn desktop_r06_distinctci() {
    check(
        "let\n    S = #table(type table [K = text, V = Int64.Type], {{\"a\",1},{\"A\",2},{\"b\",3},{\"a\",4},{\"B\",5}}),\n    D = Table.Distinct(S, {\"K\", Comparer.OrdinalIgnoreCase}),\n    I = Table.AddIndexColumn(D, \"Idx\", 0, 1, Int64.Type)\nin\n    I",
        &[("K", "string"), ("V", "int64"), ("Idx", "int64")],
        &["a\t1\t0", "b\t3\t1"],
        "fr-FR",
        true,
        &[],
        0,
    );
}
#[test]
fn desktop_r07_grouplocal() {
    check(
        "let\n    S = #table(type table [K = text, V = Int64.Type], {{\"a\",1},{\"a\",2},{\"b\",3},{\"a\",4},{\"b\",5}}),\n    G = Table.Group(S, {\"K\"}, {{\"Sum\", each List.Sum([V]), type number}}, GroupKind.Local),\n    I = Table.AddIndexColumn(G, \"Idx\", 0, 1, Int64.Type)\nin\n    I",
        &[("K", "string"), ("Sum", "double"), ("Idx", "int64")],
        &["a\t3.0\t0", "b\t3.0\t1", "a\t4.0\t2", "b\t5.0\t3"],
        "fr-FR",
        true,
        &[],
        0,
    );
}
#[test]
fn desktop_r08_joinflat() {
    check(
        "let\n    A = #table(type table [K = text, X = Int64.Type], {{\"a\",1},{\"b\",2},{\"c\",3}}),\n    B = #table(type table [K2 = text, Y = text], {{\"a\",\"A1\"},{\"a\",\"A2\"},{\"c\",\"C\"}}),\n    J = Table.Join(A, \"K\", B, \"K2\", JoinKind.LeftOuter)\nin\n    J",
        &[("K", "string"), ("X", "int64"), ("K2", "string"), ("Y", "string")],
        &["a\t1\ta\tA1", "a\t1\ta\tA2", "b\t2\t<null>\t<null>", "c\t3\tc\tC"],
        "fr-FR",
        true,
        &[],
        0,
    );
}
#[test]
fn desktop_r09_errorhandling() {
    check(
        "let\n    S = Table.TransformColumnTypes(#table({\"A\",\"B\"}, {{\"1\",\"x\"},{\"x\",\"2\"},{\"3\",\"4\"}}), {{\"A\", Int64.Type}, {\"B\", Int64.Type}}),\n    R = Table.ReplaceErrorValues(S, {{\"B\", -1}}),\n    Rm = Table.RemoveRowsWithErrors(R, {\"A\"}),\n    C = Table.AddColumn(Rm, \"Bad\", each try Number.FromText(\"?\") otherwise 0, Int64.Type)\nin\n    C",
        &[("A", "int64"), ("B", "int64"), ("Bad", "int64")],
        &["1\t-1\t0", "3\t4\t0"],
        "fr-FR",
        true,
        &[],
        0,
    );
}
#[test]
fn desktop_r10_csv1252() {
    check(
        "let\n    Source = Csv.Document(File.Contents(\"{root}cp1252.csv\"), [Delimiter=\";\", Encoding=1252]),\n    I = Table.AddIndexColumn(Source, \"Idx\", 0, 1, Int64.Type)\nin\n    I",
        &[
            ("Column1", "string"),
            ("Column2", "string"),
            ("Column3", "string"),
            ("Column4", "string"),
            ("Idx", "int64"),
        ],
        &["!ERROR column `Column4` (source `Column4`) not found in the source data"],
        "fr-FR",
        true,
        &[],
        0,
    );
}
#[test]
fn desktop_r11_csvextradefault() {
    check(
        "let\n    Source = Csv.Document(File.Contents(\"{root}extra.csv\"), [Delimiter=\",\", Columns=2, Encoding=65001, QuoteStyle=QuoteStyle.None]),\n    I = Table.AddIndexColumn(Source, \"Idx\", 0, 1, Int64.Type)\nin\n    I",
        &[("Column1", "string"), ("Column2", "string"), ("Idx", "int64")],
        &["a\tb\t0", "d\te\t1", "f\t\t2"],
        "fr-FR",
        true,
        &[],
        0,
    );
}
#[test]
fn desktop_r12_exceltable() {
    check(
        "let\n    Source = Excel.Workbook(File.Contents(\"{root}book2.xlsx\"), null, true),\n    Sales = Source{[Item=\"Sales\",Kind=\"Table\"]}[Data],\n    Typed = Table.TransformColumnTypes(Sales, {{\"Item\", type text}, {\"Units\", Int64.Type}})\nin\n    Typed",
        &[("Item", "string"), ("Units", "int64")],
        &["Pen\t3", "Book\t12"],
        "fr-FR",
        true,
        &[],
        0,
    );
}
#[test]
fn desktop_r13_promoteplain() {
    check(
        "let\n    S = #table(3, {{\"A\", 1, null}, {\"x\", 2, 3}, {\"y\", 4, 5}}),\n    P = Table.PromoteHeaders(S),\n    N = Table.FromRows({{Text.Combine(Table.ColumnNames(P), \"|\"), Table.RowCount(P)}}, type table [Names = text, Rows = Int64.Type])\nin\n    N",
        &[("Names", "string"), ("Rows", "int64")],
        &["A|1|Column3\t2"],
        "fr-FR",
        true,
        &[],
        0,
    );
}
#[test]
fn desktop_r14_exprs() {
    check(
        "let\n    items = {\n        {\"fromrecords\", Text.Combine(List.Transform(Table.ToRows(Table.FromRecords({[a=1,b=2],[a=3]}, {\"a\",\"b\"}, MissingField.UseNull)), each Text.Combine(List.Transform(_, each Text.From(_) ?? \"null\"), \",\")), \";\")},\n        {\"lastn\", Text.From(Table.RowCount(Table.LastN(#table({\"A\"},{{1},{2},{3}}), 2)))},\n        {\"sortdesc\", Text.Combine(List.Sort({\"b\",\"a\",\"c\"}, Order.Descending), \",\")},\n        {\"positions\", Text.Combine(Splitter.SplitTextByPositions({0, 2, 4})(\"abcdefg\"), \"|\")},\n        {\"lengths\", Text.Combine(Splitter.SplitTextByLengths({1, 2})(\"abcdefg\"), \"|\")},\n        {\"combinecols\", Table.CombineColumns(#table({\"A\",\"B\"},{{\"x\",\"y\"}}), {\"A\",\"B\"}, Combiner.CombineTextByDelimiter(\"-\"), \"M\"){0}[M]},\n        {\"replacetype\", Text.Combine(Table.ColumnNames(Value.ReplaceType(#table({\"A\",\"B\"},{{1,2}}), type table [X = number, Y = number])), \",\")},\n        {\"rectotable\", Text.From(Table.RowCount(Record.ToTable([a=1,b=2])))},\n        {\"datefmt\", Text.From(Date.FromText(\"05.01.2026\", [Format=\"dd.MM.yyyy\", Culture=\"de-DE\"]))},\n        {\"dtfromtext\", Text.From(DateTime.FromText(\"2026-01-05 13:04\"))},\n        {\"timefromtext\", Text.From(Time.FromText(\"1:05 PM\", \"en-US\"))},\n        {\"numfromtext_fr\", Text.From(Number.FromText(\"1,5\"))},\n        {\"numfromtext_en\", Text.From(Number.FromText(\"1.5\", \"en-US\"))},\n        {\"between\", Text.BetweenDelimiters(\"a[b]c[d]\", \"[\", \"]\", 1)},\n        {\"posany\", Text.From(Text.PositionOfAny(\"hello\", {\"l\",\"o\"}))},\n        {\"padend\", Text.PadEnd(\"ab\", 4, \".\")},\n        {\"listdiff\", Text.Combine(List.Difference({\"a\",\"b\",\"a\",\"c\"}, {\"a\"}), \",\")},\n        {\"listunion\", Text.Combine(List.Union({{\"a\",\"b\"},{\"b\",\"c\"}}), \",\")},\n        {\"listintersect\", Text.Combine(List.Intersect({{\"a\",\"b\",\"b\"},{\"b\",\"b\",\"c\"}}), \",\")},\n        {\"modneg\", Text.From(Number.Mod(7, -3))},\n        {\"rounddown\", Text.From(Number.RoundDown(-1.25, 1))},\n        {\"roundup\", Text.From(Number.RoundUp(1.21, 1))},\n        {\"roundaway\", Text.From(Number.RoundAwayFromZero(-1.25, 1))},\n        {\"roundtz\", Text.From(Number.RoundTowardZero(-1.29, 1))},\n        {\"int64from\", Text.From(Int64.From(\"2,5\"))},\n        {\"currency\", Text.From(Currency.From(1.23456))},\n        {\"durfrom\", Text.From(Duration.From(1.5))},\n        {\"durtotal\", Text.From(Duration.TotalHours(#duration(1,2,30,0)))},\n        {\"textrange_err\", try Text.Range(\"abc\", 5) otherwise \"err\"},\n        {\"listgen\", Text.Combine(List.Generate(() => 1, each _ < 20, each _ * 3, each Text.From(_)), \",\")},\n        {\"listalternate\", Text.Combine(List.Transform(List.Alternate({1..10}, 1, 2, 1), Text.From), \",\")},\n        {\"listmode\", Text.From(List.Mode({1,2,2,3}))},\n        {\"listmedian\", Text.From(List.Median({1,2,3,4}))},\n        {\"textinsert\", Text.Insert(\"abc\", 1, \"X\")},\n        {\"textremove\", Text.Remove(\"a-b_c\", {\"-\",\"_\"})},\n        {\"textselect\", Text.Select(\"a1b2c3\", {\"1\",\"2\",\"3\"})},\n        {\"recfields\", Text.Combine(Record.FieldNames(Record.RemoveFields([a=1,b=2,c=3], \"b\")), \",\")},\n        {\"reccombine\", Text.From(Record.Combine({[a=1],[b=2],[a=3]})[a])},\n        {\"tablemax\", Text.From(Table.Max(#table({\"A\",\"B\"},{{1,\"x\"},{3,\"y\"},{2,\"z\"}}), \"A\")[B])},\n        {\"posof\", Text.From(Table.PositionOf(#table({\"A\"},{{1},{2}}), [A=2]))},\n        {\"tcontains\", Text.From(Table.Contains(#table({\"A\"},{{1},{2}}), [A=3]))},\n        {\"logical0\", Text.From(Logical.From(0))},\n        {\"dtz_text\", DateTimeZone.ToText(#datetimezone(2026,1,5,13,0,0,-5,-30), [Format=\"yyyy-MM-dd HH:mm zzz\"])},\n        {\"addq\", Text.From(Date.AddQuarters(#date(2025,11,30), 1))},\n        {\"proper2\", Text.Proper(\"ABC123def ghi-JKL\")},\n        {\"valuefromtext\", Text.From(Value.FromText(\"12,5\"))},\n        {\"splitcsv\", Text.Combine(Splitter.SplitTextByDelimiter(\",\", QuoteStyle.None)(\"a,\"\"b,c\"\",d\"), \"|\")},\n        {\"fillup\", Text.Combine(List.Transform(Table.FillUp(#table({\"A\"},{{null},{1},{null},{2}}), {\"A\"})[A], each Text.From(_) ?? \"n\"), \",\")},\n        {\"transpose\", Text.Combine(List.Transform(Table.ToRows(Table.Transpose(#table({\"A\",\"B\"},{{1,2},{3,4}}))), each Text.Combine(List.Transform(_, Text.From), \",\")), \";\")},\n        {\"expandrec\", Text.Combine(Table.ColumnNames(Table.ExpandRecordColumn(#table({\"R\"},{{[a=1,b=2]}}), \"R\", {\"b\"}, {\"R.b\"})), \",\")},\n        {\"splitcolumn\", Text.Combine(Table.ColumnNames(Table.SplitColumn(#table({\"A\"},{{\"x-y-z\"}}), \"A\", Splitter.SplitTextByDelimiter(\"-\"))), \",\")},\n        {\"unpivot_nulls\", Text.From(Table.RowCount(Table.Unpivot(#table({\"K\",\"A\",\"B\"},{{1,null,2}}), {\"A\",\"B\"}, \"Att\", \"Val\")))},\n        {\"removecols_missing\", try Text.From(Table.ColumnCount(Table.RemoveColumns(#table({\"A\"},{{1}}), {\"Z\"}))) otherwise \"err\"},\n        {\"removecols_ignore\", Text.From(Table.ColumnCount(Table.RemoveColumns(#table({\"A\"},{{1}}), {\"Z\"}, MissingField.Ignore)))},\n        {\"selectcols_null\", Text.Combine(Table.ColumnNames(Table.SelectColumns(#table({\"A\"},{{1}}), {\"A\",\"Z\"}, MissingField.UseNull)), \",\")},\n        {\"reorder\", Text.Combine(Table.ColumnNames(Table.ReorderColumns(#table({\"A\",\"B\",\"C\"},{{1,2,3}}), {\"C\",\"A\"})), \",\")},\n        {\"text_from_dt_ms\", Text.From(#datetime(2026,1,5,13,4,5.678))},\n        {\"number_e\", Number.ToText(12345.678, \"E2\")},\n        {\"number_d\", Number.ToText(-42, \"D4\")},\n        {\"number_x\", Number.ToText(255, \"X4\")},\n        {\"number_g\", Number.ToText(1234.5678, \"G5\")},\n        {\"number_r\", Number.ToText(0.1, \"R\")}\n    },\n    T = Table.FromRows(items, type table [K = text, V = text])\nin\n    T",
        &[("K", "string"), ("V", "string")],
        &[
            "fromrecords\t1,2;3,null",
            "lastn\t2",
            "sortdesc\tc,b,a",
            "positions\tab|cd|efg",
            "lengths\ta|bc",
            "combinecols\tx-y",
            "replacetype\tX,Y",
            "rectotable\t2",
            "datefmt\t05/01/2026",
            "dtfromtext\t05/01/2026 13:04:00",
            "timefromtext\t13:05",
            "numfromtext_fr\t1,5",
            "numfromtext_en\t1,5",
            "between\td",
            "posany\t2",
            "padend\tab..",
            "listdiff\tb,a,c",
            "listunion\ta,b,c",
            "listintersect\tb,b",
            "modneg\t1",
            "rounddown\t-1,3",
            "roundup\t1,3",
            "roundaway\t-1,3",
            "roundtz\t-1,2",
            "int64from\t2",
            "currency\t1,2346",
            "durfrom\t1.12:00:00",
            "durtotal\t26,5",
            "textrange_err\terr",
            "listgen\t1,3,9",
            "listalternate\t1,3,4,6,7,9,10",
            "listmode\t2",
            "listmedian\t2,5",
            "textinsert\taXbc",
            "textremove\tabc",
            "textselect\t123",
            "recfields\ta,c",
            "reccombine\t3",
            "tablemax\ty",
            "posof\t1",
            "tcontains\tfalse",
            "logical0\tfalse",
            "dtz_text\t2026-01-05 13:00 -05:30",
            "addq\t28/02/2026",
            "proper2\tAbc123Def Ghi-Jkl",
            "valuefromtext\t12,5",
            "splitcsv\ta|\"b|c\"|d",
            "fillup\t1,1,2,2",
            "transpose\t1,3;2,4",
            "expandrec\tR.b",
            "splitcolumn\tA.1,A.2,A.3",
            "unpivot_nulls\t1",
            "removecols_missing\terr",
            "removecols_ignore\t1",
            "selectcols_null\tA,Z",
            "reorder\tC,B,A",
            "text_from_dt_ms\t05/01/2026 13:04:05",
            "number_e\t1,23E+004",
            "number_d\t-0042",
            "number_x\t00FF",
            "number_g\t1234,6",
            "number_r\t0,1",
        ],
        "fr-FR",
        true,
        &[],
        0,
    );
}
/// Decimal and double numbers (`docs/formats/data-sources.md`, *Numbers*): literals, rounding,
/// averages, precision, conversions and formats.
#[test]
fn desktop_d01_decimals() {
    check(
        "let\n    items = {\n        {\"lit_long\", Text.From(0.1234567890123456789)},\n        {\"lit_29dec\", Text.From(0.12345678901234567890123456789)},\n        {\"lit_e16\", Text.From(1e-16)},\n        {\"lit_e15\", Text.From(1e-15)},\n        {\"lit_e30\", Text.From(1e30)},\n        {\"lit_7e28\", Text.From(7e28)},\n        {\"add_big\", Text.From(123456789012345678 + 1)},\n        {\"cmp_big\", Text.From(123456789012345678 = 123456789012345679)},\n        {\"neg_big\", Text.From(-123456789012345678)},\n        {\"r_25\", Text.From(Number.Round(2.5))},\n        {\"r_225_1\", Text.From(Number.Round(2.25, 1))},\n        {\"r_2674\", Text.From(Number.Round(2.674, 2))},\n        {\"r_10_4\", Text.From(Number.Round(10 / 4))},\n        {\"r_2_3\", Text.From(Number.Round(2 / 3, 2))},\n        {\"r_1_8\", Text.From(Number.Round(1 / 8, 2))},\n        {\"r_up\", Text.From(Number.Round(2.675, 2, RoundingMode.Up))},\n        {\"r_down\", Text.From(Number.Round(2.675, 2, RoundingMode.Down))},\n        {\"r_tz\", Text.From(Number.Round(2.675, 2, RoundingMode.TowardZero))},\n        {\"r_up_nomid\", Text.From(Number.Round(2.671, 2, RoundingMode.Up))},\n        {\"r_m2\", Text.From(Number.Round(1250, -2))},\n        {\"r_m1\", Text.From(Number.Round(15, -1))},\n        {\"r_big\", Text.From(Number.Round(123456789012345678.5))},\n        {\"r_e30\", Text.From(Number.Round(1e30))},\n        {\"r_29\", Text.From(Number.Round(12345678901234567890123456789))},\n        {\"r_neg05\", Text.From(Number.Round(-0.5))},\n        {\"roundup_big\", Text.From(Number.RoundUp(123456789012345678.1))},\n        {\"avg_neg\", Text.From(List.Average({-1, 2, 10}))},\n        {\"avg_shift\", Text.From(List.Average({-33, 3, 62}))},\n        {\"avg_decp\", Text.From(List.Average({1, 2, 4}, Precision.Decimal))},\n        {\"sum_dbl\", Text.From(List.Sum({1.10, 2.20}))},\n        {\"sum_decp\", Text.From(List.Sum({1.10, 2.20}, Precision.Decimal))},\n        {\"vdiv_dec\", Text.From(Value.Divide(1, 3, Precision.Decimal))},\n        {\"vmul_scale\", Text.From(Value.Multiply(Number.Round(2.675, 2), 10, Precision.Decimal))},\n        {\"vadd_ovf\", Text.From(Value.Add(79228162514264337593543950335, 1, Precision.Decimal))},\n        {\"dec_from\", Text.From(Decimal.From(1 / 3))},\n        {\"cur_from\", Text.From(Currency.From(2.67549))},\n        {\"int64_from\", Text.From(Int64.From(9007199254740993))},\n        {\"nft_big\", Text.From(Number.FromText(\"123456789012345678\"))},\n        {\"nft_small\", Text.From(Number.FromText(\"0.00001\"))},\n        {\"nft_paren\", Text.From(Number.FromText(\"(2.5)\"))},\n        {\"json_big\", Text.From(Json.Document(\"[123456789012345678]\"){0})},\n        {\"tct_big\", Text.From(Table.TransformColumnTypes(#table({\"a\"}, {{\"123456789012345678\"}}), {{\"a\", type number}}){0}[a])},\n        {\"totext_n0\", Number.ToText(123456789012345678, \"N0\")},\n        {\"totext_f20\", Number.ToText(0.1234567890123456789, \"F20\")},\n        {\"totext_e3\", Number.ToText(123456789012345678, \"E3\")},\n        {\"totext_g5\", Number.ToText(123456789012345678, \"G5\")},\n        {\"totext_x\", Number.ToText(123456789012345678, \"X\")},\n        {\"totext_g2\", Number.ToText(0.125, \"G2\")},\n        {\"totext_rg\", Number.ToText(Number.Round(2.675, 2), \"G\")},\n        {\"power\", Text.From(Number.Power(2, 60))},\n        {\"mod_big\", Text.From(Number.Mod(123456789012345678, 10))},\n        {\"index_big\", Text.From(Table.AddIndexColumn(#table({\"a\"}, {{1}}), \"i\", 123456789012345678, 1){0}[i])}\n    },\n    T = Table.FromRows(items, type table [K = text, V = text])\nin\n    T",
        &[("K", "string"), ("V", "string")],
        &[
            "lit_long\t0.1234567890123456789",
            "lit_29dec\t0.1234567890123456789012345679",
            "lit_e16\t0.0000000000000001",
            "lit_e15\t1E-15",
            "lit_e30\t1E+30",
            "lit_7e28\t70000000000000000000000000000",
            "add_big\t123456789012345680",
            "cmp_big\ttrue",
            "neg_big\t-123456789012345678",
            "r_25\t2.0",
            "r_225_1\t2.20",
            "r_2674\t2.67",
            "r_10_4\t2.0",
            "r_2_3\t0.67",
            "r_1_8\t0.120",
            "r_up\t2.680",
            "r_down\t2.670",
            "r_tz\t2.67",
            "r_up_nomid\t2.67",
            "r_m2\t1200",
            "r_m1\t20",
            "r_big\t123456789012345678.0",
            "r_e30\t1E+30",
            "r_29\t12345678901234567890123456790",
            "r_neg05\t0.0",
            "roundup_big\t123456789012345679",
            "avg_neg\t3.666666666666667",
            "avg_shift\t10.666666666666664",
            "avg_decp\t2.3333333333333333333333333333",
            "sum_dbl\t3.3000000000000003",
            "sum_decp\t3.3",
            "vdiv_dec\t0.3333333333333333333333333333",
            "vmul_scale\t26.800",
            "vadd_ovf\t∞",
            "dec_from\t0.333333333333333",
            "cur_from\t2.6755",
            "int64_from\t9007199254740993",
            "nft_big\t123456789012345678",
            "nft_small\t1E-05",
            "nft_paren\t-2.5",
            "json_big\t123456789012345678",
            "tct_big\t123456789012345678",
            "totext_n0\t123,456,789,012,345,678",
            "totext_f20\t0.12345678901234567890",
            "totext_e3\t1.235E+017",
            "totext_g5\t1.2346E+17",
            "totext_x\t1B69B4BA630F34E",
            "totext_g2\t0.13",
            "totext_rg\t2.680",
            "power\t1152921504606846976",
            "mod_big\t0",
            "index_big\t123456789012345678",
        ],
        "en-US",
        true,
        &[],
        0,
    );
}

/// Decimals loaded into double, whole-number and currency columns. The doubles were compared
/// with Desktop's by value.
#[test]
fn desktop_d02_loaded_decimals() {
    check(
        "#table(type table [K = text, D = number, I = Int64.Type, C = Currency.Type], {\n    {\"big\", 123456789012345678, 123456789012345678, 123456789012345.67894},\n    {\"q29_7\", Value.Divide(29, 7, Precision.Decimal), 9007199254740993, 2.67545},\n    {\"lit\", 0.1234567890123456789, Int64.From(2.5), Value.Divide(1, 3, Precision.Decimal)},\n    {\"round\", Number.Round(2.675, 2), Number.Round(2.5), Number.Round(2.675, 2)},\n    {\"dbl\", 1/3, 3, 1/3},\n    {\"tie2\", 2.3333333333333333333333333333, -9007199254740993, -2.67545},\n    {\"third\", 0.3333333333333333333333333333, 1, 2.67535},\n    {\"q11_3\", Value.Divide(11, 3, Precision.Decimal), 2, 0.00005},\n    {\"q2_3\", Value.Divide(2, 3, Precision.Decimal), 4, 0.00015},\n    {\"q1_7\", Value.Divide(1, 7, Precision.Decimal), 5, -0.00005},\n    {\"e\", 2.7182818284590452353602874714, 6, 1.00005}\n})",
        &[("K", "string"), ("D", "double"), ("I", "int64"), ("C", "decimal")],
        &[
            "big\t1.2345678901234568e17\t123456789012345678\t123456789012345.6789",
            "q29_7\t4.142857142857142\t9007199254740993\t2.6754",
            "lit\t0.12345678901234568\t2\t0.3333",
            "round\t2.68\t2\t2.68",
            "dbl\t0.3333333333333333\t3\t0.3333",
            "tie2\t2.3333333333333335\t-9007199254740993\t-2.6754",
            "third\t0.3333333333333333\t1\t2.6754",
            "q11_3\t3.6666666666666665\t2\t0",
            "q2_3\t0.6666666666666667\t4\t0.0002",
            "q1_7\t0.14285714285714288\t5\t0",
            "e\t2.718281828459045\t6\t1",
        ],
        "en-US",
        true,
        &[],
        0,
    );
}
