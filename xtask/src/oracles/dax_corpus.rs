//! `dax-corpus`: writes the DAX oracle corpus, `crates/dax/tests/oracle/corpus.json`, from its
//! authoring source `dax_corpus_data.rs` (docs/dax-oracle.md). Entries are numbered per category
//! (`<category>-NNN`) in source order, so new expressions go at the end of their category or in a
//! new one. `--check` only compares with the committed file.

use serde_json::{Map, Value, json};

use super::{dax_corpus_data, pyjson, write_text};

/// The entries in authoring order: (category, role code, expression, helper objects).
#[derive(Default)]
pub struct Corpus(Vec<(&'static str, &'static str, String, Value)>);

impl Corpus {
    pub fn add(&mut self, cat: &'static str, role: &'static str, expr: &str) {
        self.0.push((cat, role, expr.to_string(), json!([])));
    }

    pub fn add_with(&mut self, cat: &'static str, role: &'static str, expr: &str, with: Value) {
        self.0.push((cat, role, expr.to_string(), with));
    }

    pub fn many(&mut self, cat: &'static str, role: &'static str, exprs: &[&str]) {
        for x in exprs {
            self.add(cat, role, x);
        }
    }

    /// Each item inside `COUNTROWS(SUMMARIZECOLUMNS(...))`.
    pub fn many_sc(&mut self, cat: &'static str, role: &'static str, items: &[&str]) {
        for x in items {
            self.add(cat, role, &format!("COUNTROWS(SUMMARIZECOLUMNS({x}))"));
        }
    }
}

fn role_fields(role: &str) -> Result<Map<String, Value>, String> {
    let v = match role {
        "m" => json!({"role": "measure", "table": "Sales"}),
        "t" => json!({"role": "table"}),
        "q" => json!({"role": "query"}),
        _ => {
            let (kind, table) = role.split_once(':').ok_or_else(|| format!("bad role `{role}`"))?;
            let kind = match kind {
                "m" => "measure",
                "c" => "column",
                "r" => "rls",
                _ => return Err(format!("bad role `{role}`")),
            };
            json!({"role": kind, "table": table})
        }
    };
    Ok(v.as_object().cloned().unwrap_or_default())
}

/// The corpus.json text.
pub fn text() -> Result<(String, usize), String> {
    let mut c = Corpus::default();
    dax_corpus_data::fill(&mut c);
    let mut seen = std::collections::HashSet::new();
    let mut counters: std::collections::HashMap<&str, usize> = Default::default();
    let mut lines = Vec::new();
    for (cat, role, expr, with) in &c.0 {
        if !seen.insert((*role, expr.as_str(), with.to_string())) {
            return Err(format!("duplicate entry: ({role}, {expr}, {with})"));
        }
        let n = counters.entry(cat).or_insert(0);
        *n += 1;
        let mut e = Map::new();
        e.insert("id".into(), json!(format!("{cat}-{n:03}")));
        e.insert("category".into(), json!(cat));
        e.extend(role_fields(role)?);
        e.insert("expression".into(), json!(expr));
        if with.as_array().is_some_and(|a| !a.is_empty()) {
            e.insert("with".into(), with.clone());
        }
        lines.push(format!("  {}", pyjson::dumps(&Value::Object(e), None, false)));
    }
    Ok((format!("[\n{}\n]\n", lines.join(",\n")), lines.len()))
}

pub fn cmd(args: &[String]) -> Result<(), String> {
    let path = crate::root()
        .join("crates")
        .join("dax")
        .join("tests")
        .join("oracle")
        .join("corpus.json");
    let (t, n) = text()?;
    if args.iter().any(|a| a == "--check") {
        let old = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if old.replace("\r\n", "\n") != t {
            return Err(format!(
                "{} is out of date: run `cargo xtask dax-corpus`",
                path.display()
            ));
        }
        println!("{n} entries, {} is up to date", path.display());
        return Ok(());
    }
    write_text(&path, &t)?;
    println!("{n} entries -> {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn dax_corpus_matches_the_committed_file() {
        let path = crate::root().join("crates/dax/tests/oracle/corpus.json");
        let committed = std::fs::read_to_string(path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        let (t, _) = super::text().unwrap_or_default();
        assert!(
            t == committed,
            "crates/dax/tests/oracle/corpus.json differs from xtask/src/oracles/dax_corpus_data.rs: \
             run `cargo xtask dax-corpus`"
        );
    }
}
