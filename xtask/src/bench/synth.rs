//! Synthetic inputs for the benchmarks. Everything is generated here from a fixed seed, so runs
//! are comparable across machines; nothing comes from a real file.

use serde_json::{Value, json};

/// xorshift64*: small, fast and deterministic. Not for anything but test data.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `0..n` (`n > 0`; 0 when `n == 0`).
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next_u64() % n }
    }
}

/// Shape of a synthetic semantic model.
#[derive(Debug, Clone, Copy)]
pub struct ModelShape {
    pub tables: usize,
    pub columns: usize,
    /// Measures in total, spread round-robin over the tables.
    pub measures: usize,
    /// Every measure references the previous one in its chain, so chains are this long.
    pub chain: usize,
    /// One 3-measure cycle per this many measures (0: none).
    pub cycle_every: usize,
}

impl ModelShape {
    /// `tables` × `columns` columns × `measures_per_table` measures, chains of 10, no cycles.
    pub fn grid(tables: usize, columns: usize, measures_per_table: usize) -> Self {
        ModelShape {
            tables,
            columns,
            measures: tables * measures_per_table,
            chain: 10,
            cycle_every: 0,
        }
    }
}

pub fn table_name(t: usize) -> String {
    format!("Table {t}")
}

pub fn column_name(c: usize) -> String {
    format!("Col {c}")
}

pub fn measure_name(m: usize) -> String {
    format!("Measure {m}")
}

const DATA_TYPES: [&str; 5] = ["int64", "double", "string", "dateTime", "decimal"];

/// Numeric columns are the ones aggregated by the generated measures.
fn is_numeric(c: usize) -> bool {
    matches!(
        DATA_TYPES.get(c % DATA_TYPES.len()),
        Some(&("int64" | "double" | "decimal"))
    )
}

/// A numeric column index near `c` (column 0 is always int64).
fn numeric_column(c: usize, columns: usize) -> usize {
    (0..columns.max(1))
        .map(|k| (c + k) % columns.max(1))
        .find(|&k| is_numeric(k))
        .unwrap_or(0)
}

/// The DAX of measure `m`: a mix of the shapes real models use (aggregations, CALCULATE with
/// filters, iterators, variables, DIVIDE, time-less IF/SWITCH), referencing columns of its home
/// table and the previous measure of its chain.
pub fn measure_expression(shape: &ModelShape, m: usize) -> String {
    let tables = shape.tables.max(1);
    let t = m % tables;
    let table = table_name(t);
    let col = |k: usize| format!("'{table}'[{}]", column_name(numeric_column(k, shape.columns)));
    let any_col = |k: usize| format!("'{table}'[{}]", column_name(k % shape.columns.max(1)));
    let base = match m % 8 {
        0 => format!("SUM({})", col(m)),
        1 => format!(
            "CALCULATE(SUM({}), FILTER(ALL({}), {} > 10))",
            col(m),
            col(m + 1),
            col(m + 1)
        ),
        2 => format!("SUMX('{table}', {} * {})", col(m), col(m + 2)),
        3 => format!(
            "VAR a = SUM({}) VAR b = COUNTROWS('{table}') RETURN DIVIDE(a, b)",
            col(m)
        ),
        4 => format!("DISTINCTCOUNT({})", any_col(m)),
        5 => format!("IF(SUM({}) > 0, AVERAGE({}), BLANK())", col(m), col(m + 1)),
        6 => format!(
            "CALCULATE(COUNTROWS('{table}'), REMOVEFILTERS({}), KEEPFILTERS({} <> 0))",
            any_col(m + 3),
            col(m)
        ),
        _ => format!(
            "SWITCH(TRUE(), MAX({}) > 100, 1, MIN({}) < 0, -1, 0)",
            col(m),
            col(m + 1)
        ),
    };
    let chained = shape.chain > 1 && !m.is_multiple_of(shape.chain);
    // A measure in a cycle references the next one; the last of the three closes the loop.
    if shape.cycle_every > 0 && m % shape.cycle_every < 3 && m + 3 <= shape.measures {
        let start = m - m % shape.cycle_every;
        let next = if m % shape.cycle_every == 2 { start } else { m + 1 };
        return format!("{base} + [{}]", measure_name(next));
    }
    if chained {
        format!("{base} + [{}]", measure_name(m - 1))
    } else {
        base
    }
}

/// A TMSL database (`model.bim` document) of the given shape: import tables with M partitions,
/// typed columns, measures and a chain of many-to-one relationships on `Col 0`.
pub fn tmsl_model(shape: &ModelShape) -> Value {
    let tables = shape.tables.max(1);
    let mut out = Vec::with_capacity(tables);
    for t in 0..tables {
        let name = table_name(t);
        let columns: Vec<Value> = (0..shape.columns.max(1))
            .map(|c| {
                let dt = DATA_TYPES.get(c % DATA_TYPES.len()).copied().unwrap_or("string");
                let mut col = json!({
                    "name": column_name(c),
                    "dataType": dt,
                    "sourceColumn": column_name(c),
                    "lineageTag": format!("00000000-0000-0000-{t:04x}-{c:012x}"),
                    "summarizeBy": if is_numeric(c) { "sum" } else { "none" },
                });
                if dt == "decimal" {
                    col["formatString"] = json!("#,0.00");
                }
                col
            })
            .collect();
        let measures: Vec<Value> = (t..shape.measures)
            .step_by(tables)
            .map(|m| {
                json!({
                    "name": measure_name(m),
                    "expression": measure_expression(shape, m),
                    "formatString": "#,0",
                    "displayFolder": format!("Folder {}", m % 7),
                    "lineageTag": format!("00000000-0000-{m:04x}-0000-000000000000"),
                })
            })
            .collect();
        let headers: Vec<String> = (0..shape.columns.max(1))
            .map(|c| format!("\"{}\"", column_name(c)))
            .collect();
        let m = format!(
            "let\n    Source = #table({{{}}}, {{}})\nin\n    Source",
            headers.join(", ")
        );
        out.push(json!({
            "name": name,
            "lineageTag": format!("00000000-0000-0000-{t:04x}-000000000000"),
            "columns": columns,
            "partitions": [{"name": name, "mode": "import", "source": {"type": "m", "expression": m}}],
            "measures": measures,
        }));
    }
    let relationships: Vec<Value> = (1..tables)
        .map(|t| {
            json!({
                "name": format!("rel-{t}"),
                "fromTable": table_name(t),
                "fromColumn": column_name(0),
                "toTable": table_name(t - 1),
                "toColumn": column_name(0),
            })
        })
        .collect();
    json!({
        "name": "Bench",
        "compatibilityLevel": 1567,
        "model": {
            "culture": "en-US",
            "dataAccessOptions": {"legacyRedirects": true, "returnErrorValuesAsNull": true},
            "defaultPowerBIDataSourceVersion": "powerBI_V3",
            "tables": out,
            "relationships": relationships,
            "annotations": [{"name": "PBI_QueryOrder", "value": "[]"}],
        }
    })
}

/// `n` DAX expressions over a model of `shape` (valid, plus one in 16 with a deliberate error),
/// for bulk validation.
pub fn dax_corpus(shape: &ModelShape, n: usize) -> Vec<String> {
    let mut rng = Rng::new(0xDA7);
    (0..n)
        .map(|i| {
            let m = rng.below(shape.measures.max(1) as u64) as usize;
            let e = measure_expression(shape, m);
            match i % 16 {
                15 => format!("SUM('{}'[No Such Column])", table_name(m % shape.tables.max(1))),
                7 => format!(
                    "CALCULATE({e}, ALLEXCEPT('{}', '{}'[{}]))",
                    table_name(m % shape.tables.max(1)),
                    table_name(m % shape.tables.max(1)),
                    column_name(1)
                ),
                _ => e,
            }
        })
        .collect()
}

/// `len` bytes shaped like an ABF payload, so XPress9 has realistic work to do: UTF-16LE XML,
/// runs of small integers (bit-packed column data), repeated strings (dictionaries) and some
/// incompressible noise.
pub fn compressible_bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut rng = Rng::new(seed);
    let mut out = Vec::with_capacity(len);
    let words = [
        "Sales",
        "Amount",
        "Quantity",
        "Customer",
        "Product",
        "Region",
        "Date",
        "Category",
        "Subcategory",
        "Store",
        "Channel",
        "Promotion",
    ];
    while out.len() < len {
        match rng.below(4) {
            0 => {
                let w = words
                    .get(rng.below(words.len() as u64) as usize)
                    .copied()
                    .unwrap_or("x");
                let xml = format!(
                    "<Column><Name>{w} {}</Name><Size>{}</Size></Column>",
                    rng.below(500),
                    rng.below(1 << 20)
                );
                out.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
            }
            1 => {
                // small integers, 4 bits each, packed in u64 words
                for _ in 0..64 {
                    out.extend_from_slice(&(rng.next_u64() & 0x0F0F_0F0F_0F0F_0F0F).to_le_bytes());
                }
            }
            2 => {
                for _ in 0..32 {
                    let w = words
                        .get(rng.below(words.len() as u64) as usize)
                        .copied()
                        .unwrap_or("x");
                    out.extend_from_slice(w.as_bytes());
                    out.push(0);
                }
            }
            _ => {
                for _ in 0..16 {
                    out.extend_from_slice(&rng.next_u64().to_le_bytes());
                }
            }
        }
    }
    out.truncate(len);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        assert_eq!(a.next_u64(), b.next_u64());
        assert!(Rng::new(0).below(10) < 10);
        assert_eq!(Rng::new(3).below(0), 0);
    }

    #[test]
    fn model_has_the_requested_shape() {
        let shape = ModelShape::grid(3, 20, 10);
        let v = tmsl_model(&shape);
        let tables = v["model"]["tables"].as_array().unwrap();
        assert_eq!(tables.len(), 3);
        assert!(
            tables
                .iter()
                .all(|t| t["columns"].as_array().unwrap().len() == 20)
        );
        let measures: usize = tables
            .iter()
            .map(|t| t["measures"].as_array().unwrap().len())
            .sum();
        assert_eq!(measures, 30);
        assert_eq!(v["model"]["relationships"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn chains_and_cycles_reference_measures() {
        let shape = ModelShape {
            tables: 2,
            columns: 5,
            measures: 20,
            chain: 5,
            cycle_every: 10,
        };
        // 0..3 is a cycle: 0 -> 1 -> 2 -> 0
        assert!(measure_expression(&shape, 0).ends_with("[Measure 1]"));
        assert!(measure_expression(&shape, 2).ends_with("[Measure 0]"));
        // chained: 4 -> 3; chain start 5 has no reference
        assert!(measure_expression(&shape, 4).ends_with("[Measure 3]"));
        assert!(!measure_expression(&shape, 5).contains("[Measure"));
        // every column reference is to a numeric column of the home table
        assert!(measure_expression(&shape, 9).contains("'Table 1'["));
    }

    #[test]
    fn corpus_and_bytes() {
        let shape = ModelShape::grid(2, 5, 4);
        let c = dax_corpus(&shape, 32);
        assert_eq!(c.len(), 32);
        assert_eq!(c.iter().filter(|e| e.contains("No Such Column")).count(), 2);
        let b = compressible_bytes(10_000, 1);
        assert_eq!(b.len(), 10_000);
        assert_eq!(b, compressible_bytes(10_000, 1));
    }
}
