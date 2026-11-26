//! T-SQL for the queries the evaluator folds ([`RemoteQuery`]) and for navigation.
//!
//! Identifiers are bracket-quoted (`]` doubled inside). A folded filter compares with numbers,
//! logical values (`1`/`0`) and dates (`DATEFROMPARTS(y, m, d)`) written from typed values, so
//! no text of the query reaches the statement; the rare text literal is an `N'…'` literal with
//! `'` doubled. Character columns of a code page (`char`, `varchar`, `text`) are read as
//! `CAST(… AS nvarchar(…))`, so their text arrives in Unicode whatever the collation.

use bicraft_data::connect::{CmpOp, Literal, Pred, RemoteQuery};

use crate::token::Column;

/// `[name]`.
pub fn ident(name: &str) -> String {
    format!("[{}]", name.replace(']', "]]"))
}

/// `[a].[b].[c]`.
pub fn path(names: &[&str]) -> String {
    names.iter().map(|n| ident(n)).collect::<Vec<_>>().join(".")
}

/// `N'text'`.
pub fn text_literal(t: &str) -> String {
    format!("N'{}'", t.replace('\'', "''"))
}

fn literal(v: &Literal) -> String {
    match v {
        Literal::Logical(b) => if *b { "1" } else { "0" }.to_string(),
        Literal::Int(n) => n.to_string(),
        Literal::Text(t) => text_literal(t),
        Literal::Date { year, month, day } => format!("DATEFROMPARTS({year}, {month}, {day})"),
    }
}

/// A predicate in M semantics as T-SQL: a SQL comparison with `NULL` excludes the row, which
/// matches M except for `<>`, where M keeps nulls.
fn predicate(p: &Pred) -> String {
    match p {
        Pred::IsNull(c) => format!("{} IS NULL", ident(c)),
        Pred::NotNull(c) => format!("{} IS NOT NULL", ident(c)),
        Pred::And(v) => {
            let parts: Vec<String> = v.iter().map(predicate).collect();
            format!("({})", parts.join(" AND "))
        }
        Pred::Compare { column, op, value } => {
            let c = ident(column);
            let v = literal(value);
            match op {
                CmpOp::Eq => format!("{c} = {v}"),
                CmpOp::Ne => format!("({c} <> {v} OR {c} IS NULL)"),
                CmpOp::Lt => format!("{c} < {v}"),
                CmpOp::Le => format!("{c} <= {v}"),
                CmpOp::Gt => format!("{c} > {v}"),
                CmpOp::Ge => format!("{c} >= {v}"),
            }
        }
    }
}

/// The select-list item of a column: code-page text cast to `nvarchar`.
fn item(c: &Column) -> String {
    let id = ident(&c.name);
    if c.ty.is_ansi_text() {
        let n = if c.ty.plp || c.ty.len > 4000 || c.ty.len == 0 {
            "max".to_string()
        } else {
            c.ty.len.to_string()
        };
        format!("CAST({id} AS nvarchar({n})) AS {id}")
    } else {
        id
    }
}

/// `SELECT [TOP (n)] … FROM table [WHERE …] [ORDER BY …]` over `table` (a quoted name) whose
/// columns are `columns`.
pub fn select(q: &RemoteQuery, table: &str, columns: &[Column]) -> Result<String, String> {
    let wanted: Vec<&Column> = match &q.columns {
        None => columns.iter().collect(),
        Some(names) if names.is_empty() => columns.iter().collect(),
        Some(names) => names
            .iter()
            .map(|n| {
                columns
                    .iter()
                    .find(|c| c.name == *n)
                    .ok_or_else(|| format!("The column '{n}' of the table wasn't found."))
            })
            .collect::<Result<_, _>>()?,
    };
    let list: Vec<String> = wanted.iter().map(|c| item(c)).collect();
    let mut sql = String::from("SELECT ");
    if let Some(n) = q.limit {
        sql.push_str(&format!("TOP ({n}) "));
    }
    sql.push_str(&if list.is_empty() {
        "*".to_string()
    } else {
        list.join(", ")
    });
    sql.push_str(" FROM ");
    sql.push_str(table);
    if !q.filter.is_empty() {
        fn flat<'p>(p: &'p Pred, out: &mut Vec<&'p Pred>) {
            match p {
                Pred::And(v) => v.iter().for_each(|p| flat(p, out)),
                other => out.push(other),
            }
        }
        let mut atoms = Vec::new();
        q.filter.iter().for_each(|p| flat(p, &mut atoms));
        let parts: Vec<String> = atoms.into_iter().map(predicate).collect();
        sql.push_str(" WHERE ");
        sql.push_str(&parts.join(" AND "));
    }
    if !q.sort.is_empty() {
        let keys: Vec<String> = q
            .sort
            .iter()
            .map(|k| {
                format!(
                    "{} {}",
                    ident(&k.column),
                    if k.descending { "DESC" } else { "ASC" }
                )
            })
            .collect();
        sql.push_str(" ORDER BY ");
        sql.push_str(&keys.join(", "));
    }
    Ok(sql)
}

/// The columns of a table, without rows.
pub fn columns(table: &str) -> String {
    format!("SELECT TOP (0) * FROM {table}")
}

/// The databases the login may open.
pub fn databases() -> &'static str {
    "SELECT name FROM sys.databases WHERE HAS_DBACCESS(name) = 1 ORDER BY name"
}

/// The user tables and views of a database (`None`: the login's), with their schema.
pub fn objects(database: Option<&str>) -> String {
    let p = database.map(|d| format!("{}.", ident(d))).unwrap_or_default();
    format!(
        "SELECT s.name, o.name, o.type FROM {p}sys.objects AS o JOIN {p}sys.schemas AS s \
         ON s.schema_id = o.schema_id WHERE o.type IN ('U', 'V') AND o.is_ms_shipped = 0 \
         ORDER BY s.name, o.name"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TypeInfo;
    use crate::wire::Rd;
    use bicraft_data::connect::SortKey;

    fn col(name: &str, ty: &[u8]) -> Column {
        Column {
            name: name.into(),
            ty: TypeInfo::read(&mut Rd::new(ty)).unwrap(),
        }
    }

    #[test]
    fn select_statements() {
        let cols = vec![
            col("id", &[0x38]),
            col("name]x", &[0xA7, 50, 0, 9, 4, 0xD0, 0, 0x34]),
            col("day", &[0x28]),
            col("note", &[0xA7, 0xFF, 0xFF, 9, 4, 0xD0, 0, 0x34]),
        ];
        let q = RemoteQuery {
            table: vec!["dbo.T".into()],
            columns: None,
            ..Default::default()
        };
        assert_eq!(
            select(&q, "[dbo].[T]", &cols).unwrap(),
            "SELECT [id], CAST([name]]x] AS nvarchar(50)) AS [name]]x], [day], CAST([note] AS nvarchar(max)) AS [note] FROM [dbo].[T]"
        );
        let q = RemoteQuery {
            table: vec![],
            columns: Some(vec!["day".into(), "id".into()]),
            filter: vec![
                Pred::Compare {
                    column: "id".into(),
                    op: CmpOp::Ne,
                    value: Literal::Int(-3),
                },
                Pred::And(vec![
                    Pred::NotNull("day".into()),
                    Pred::Compare {
                        column: "day".into(),
                        op: CmpOp::Ge,
                        value: Literal::Date {
                            year: 2026,
                            month: 1,
                            day: 2,
                        },
                    },
                ]),
            ],
            sort: vec![
                SortKey {
                    column: "day".into(),
                    descending: true,
                },
                SortKey {
                    column: "id".into(),
                    descending: false,
                },
            ],
            limit: Some(10),
        };
        assert_eq!(
            select(&q, "[db].[dbo].[T]", &cols).unwrap(),
            "SELECT TOP (10) [day], [id] FROM [db].[dbo].[T] WHERE ([id] <> -3 OR [id] IS NULL) AND [day] IS NOT NULL \
             AND [day] >= DATEFROMPARTS(2026, 1, 2) ORDER BY [day] DESC, [id] ASC"
        );
        let missing = RemoteQuery {
            columns: Some(vec!["nope".into()]),
            ..Default::default()
        };
        assert!(select(&missing, "[t]", &cols).is_err());
        assert_eq!(text_literal("it's"), "N'it''s'");
        assert_eq!(path(&["a]", "b"]), "[a]]].[b]");
        assert!(objects(Some("S]x")).contains("FROM [S]]x].sys.objects AS o JOIN [S]]x].sys.schemas"));
    }
}
