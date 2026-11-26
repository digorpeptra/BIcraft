//! Databricks SQL for the queries the evaluator folds ([`RemoteQuery`]) and for navigation.
//!
//! Identifiers are quoted with backticks (a backtick doubled inside); every literal is a named
//! parameter marker (`:p0`) sent in the request's `parameters`, never spliced into the text, so
//! no value can change the statement.

use bicraft_data::connect::{CmpOp, Literal, Pred, RemoteQuery};
use serde_json::{Value, json};

/// A statement and its parameters (`{name, value, type}` as the API takes them).
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub sql: String,
    pub parameters: Vec<Value>,
}

impl Statement {
    pub(crate) fn plain(sql: impl Into<String>) -> Self {
        Self {
            sql: sql.into(),
            parameters: Vec::new(),
        }
    }

    fn param(&mut self, value: String, ty: &str) -> String {
        let name = format!("p{}", self.parameters.len());
        self.parameters
            .push(json!({"name": name, "value": value, "type": ty}));
        format!(":{name}")
    }
}

/// `` `name` ``.
pub fn ident(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

/// `` `a`.`b`.`c` ``.
pub(crate) fn path(names: &[String]) -> String {
    names.iter().map(|n| ident(n)).collect::<Vec<_>>().join(".")
}

fn literal(st: &mut Statement, v: &Literal) -> String {
    match v {
        Literal::Logical(b) => st.param(b.to_string(), "BOOLEAN"),
        Literal::Int(n) => st.param(n.to_string(), "BIGINT"),
        Literal::Text(t) => st.param(t.clone(), "STRING"),
        Literal::Date { year, month, day } => st.param(format!("{year:04}-{month:02}-{day:02}"), "DATE"),
    }
}

/// A predicate in M semantics (`Pred`) as SQL: SQL `NULL` comparisons exclude the row, which
/// matches M except for `<>`, where M keeps nulls.
fn predicate(st: &mut Statement, p: &Pred) -> String {
    match p {
        Pred::IsNull(c) => format!("{} IS NULL", ident(c)),
        Pred::NotNull(c) => format!("{} IS NOT NULL", ident(c)),
        Pred::And(v) => {
            let parts: Vec<String> = v.iter().map(|p| predicate(st, p)).collect();
            format!("({})", parts.join(" AND "))
        }
        Pred::Compare { column, op, value } => {
            let c = ident(column);
            let v = literal(st, value);
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

/// `SELECT … FROM catalog.schema.table [WHERE …] [LIMIT n]`.
pub fn select(q: &RemoteQuery) -> Statement {
    let mut st = Statement::plain("");
    let cols = match &q.columns {
        None => "*".to_string(),
        Some(c) if c.is_empty() => "*".to_string(),
        Some(c) => c.iter().map(|n| ident(n)).collect::<Vec<_>>().join(", "),
    };
    let mut sql = format!("SELECT {cols} FROM {}", path(&q.table));
    if !q.filter.is_empty() {
        // Top-level conjunctions are flattened into the WHERE list.
        fn flat<'p>(p: &'p Pred, out: &mut Vec<&'p Pred>) {
            match p {
                Pred::And(v) => v.iter().for_each(|p| flat(p, out)),
                other => out.push(other),
            }
        }
        let mut atoms = Vec::new();
        q.filter.iter().for_each(|p| flat(p, &mut atoms));
        let parts: Vec<String> = atoms.iter().map(|p| predicate(&mut st, p)).collect();
        sql.push_str(" WHERE ");
        sql.push_str(&parts.join(" AND "));
    }
    if let Some(n) = q.limit {
        sql.push_str(&format!(" LIMIT {n}"));
    }
    st.sql = sql;
    st
}

/// A statement as View native query shows it: the SQL, then one comment line per parameter
/// (`-- :p0 = 2024 (BIGINT)`), as the values are sent apart from the text.
pub fn native_text(st: &Statement) -> String {
    let mut out = st.sql.clone();
    for p in &st.parameters {
        let field = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or_default();
        out.push_str(&format!(
            "\n-- :{} = {} ({})",
            field("name"),
            field("value").replace(['\r', '\n'], " "),
            field("type")
        ));
    }
    out
}

/// The columns of a table, without rows (the manifest names and types them).
pub(crate) fn columns(table: &[String]) -> Statement {
    Statement::plain(format!("SELECT * FROM {} LIMIT 0", path(table)))
}

pub(crate) fn catalogs() -> Statement {
    Statement::plain("SHOW CATALOGS")
}

pub(crate) fn schemas(catalog: &str) -> Statement {
    Statement::plain(format!("SHOW SCHEMAS IN {}", ident(catalog)))
}

/// Tables and views of a schema, with their kind (Unity Catalog's `information_schema`).
pub(crate) fn tables(catalog: &str, schema: &str) -> Statement {
    let mut st = Statement::plain("");
    let p = st.param(schema.to_string(), "STRING");
    st.sql = format!(
        "SELECT table_name, table_type FROM {}.information_schema.tables WHERE table_schema = {p} ORDER BY table_name",
        ident(catalog)
    );
    st
}

/// Tables and views of a schema without `information_schema` (`hive_metastore`).
pub(crate) fn show_tables(catalog: &str, schema: &str) -> Statement {
    Statement::plain(format!("SHOW TABLES IN {}.{}", ident(catalog), ident(schema)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(columns: Option<&[&str]>, filter: Vec<Pred>, limit: Option<u64>) -> RemoteQuery {
        RemoteQuery {
            table: vec!["main".into(), "comptes".into(), "v`x".into()],
            columns: columns.map(|c| c.iter().map(|s| s.to_string()).collect()),
            filter,
            limit,
            ..Default::default()
        }
    }

    #[test]
    fn select_statements() {
        let s = select(&q(None, vec![], None));
        assert_eq!(s.sql, "SELECT * FROM `main`.`comptes`.`v``x`");
        assert!(s.parameters.is_empty());
        let s = select(&q(
            Some(&["id", "date"]),
            vec![
                Pred::Compare {
                    column: "statut".into(),
                    op: CmpOp::Ne,
                    value: Literal::Text("x' OR 1=1 --".into()),
                },
                Pred::And(vec![
                    Pred::NotNull("date".into()),
                    Pred::Compare {
                        column: "date".into(),
                        op: CmpOp::Ge,
                        value: Literal::Date {
                            year: 2026,
                            month: 1,
                            day: 2,
                        },
                    },
                ]),
            ],
            Some(10),
        ));
        assert_eq!(
            s.sql,
            "SELECT `id`, `date` FROM `main`.`comptes`.`v``x` WHERE (`statut` <> :p0 OR `statut` IS NULL) \
             AND `date` IS NOT NULL AND `date` >= :p1 LIMIT 10"
        );
        assert_eq!(
            s.parameters,
            vec![
                json!({"name": "p0", "value": "x' OR 1=1 --", "type": "STRING"}),
                json!({"name": "p1", "value": "2026-01-02", "type": "DATE"}),
            ]
        );
    }

    #[test]
    fn native_text_lists_the_parameters_after_the_sql() {
        let s = select(&q(
            Some(&["id"]),
            vec![Pred::Compare {
                column: "id".into(),
                op: CmpOp::Gt,
                value: Literal::Int(5),
            }],
            Some(3),
        ));
        assert_eq!(
            native_text(&s),
            "SELECT `id` FROM `main`.`comptes`.`v``x` WHERE `id` > :p0 LIMIT 3\n-- :p0 = 5 (BIGINT)"
        );
        let s = select(&q(
            None,
            vec![Pred::Compare {
                column: "t".into(),
                op: CmpOp::Eq,
                value: Literal::Text("a\nb".into()),
            }],
            None,
        ));
        assert!(
            native_text(&s).ends_with("-- :p0 = a b (STRING)"),
            "{}",
            native_text(&s)
        );
    }

    #[test]
    fn navigation_statements() {
        assert_eq!(schemas("ma`in").sql, "SHOW SCHEMAS IN `ma``in`");
        let t = tables("main", "comptes");
        assert!(
            t.sql
                .contains("FROM `main`.information_schema.tables WHERE table_schema = :p0")
        );
        assert_eq!(t.parameters[0]["value"], "comptes");
        assert_eq!(
            columns(&["a".into(), "b".into()]).sql,
            "SELECT * FROM `a`.`b` LIMIT 0"
        );
    }
}
