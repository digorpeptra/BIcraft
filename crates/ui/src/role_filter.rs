//! The default editor of Manage security roles (Microsoft Learn, "Row-level security (RLS) with
//! Power BI", Define roles): rows of column / condition / value joined by And or Or, turned into
//! the DAX row filter, and a filter read back into rows when it has that shape. Pure functions
//! over text, no engine; any text, never a panic.
//!
//! The rows read left to right: `a || b && c` is written `(a || b) && c`, since DAX's `&&` binds
//! tighter than `||`. The conditions are BIcraft's choice; Desktop's list is not checked.

use crate::dax_text::{self, Class, Tok};
use crate::i18n::Lang;

/// A condition of the default editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Op {
    #[default]
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    Contains,
    NotContains,
    Blank,
    NotBlank,
}

impl Op {
    pub const ALL: [Op; 10] = [
        Op::Eq,
        Op::Ne,
        Op::Gt,
        Op::Ge,
        Op::Lt,
        Op::Le,
        Op::Contains,
        Op::NotContains,
        Op::Blank,
        Op::NotBlank,
    ];

    pub fn label(self, l: Lang) -> &'static str {
        match self {
            Op::Eq => l.t("equals", "est égal à"),
            Op::Ne => l.t("does not equal", "n'est pas égal à"),
            Op::Gt => l.t("is greater than", "est supérieur à"),
            Op::Ge => l.t("is greater than or equal to", "est supérieur ou égal à"),
            Op::Lt => l.t("is less than", "est inférieur à"),
            Op::Le => l.t("is less than or equal to", "est inférieur ou égal à"),
            Op::Contains => l.t("contains", "contient"),
            Op::NotContains => l.t("does not contain", "ne contient pas"),
            Op::Blank => l.t("is blank", "est vide"),
            Op::NotBlank => l.t("is not blank", "n'est pas vide"),
        }
    }

    /// The condition takes a value.
    pub fn has_value(self) -> bool {
        !matches!(self, Op::Blank | Op::NotBlank)
    }

    fn symbol(self) -> Option<&'static str> {
        Some(match self {
            Op::Eq => "=",
            Op::Ne => "<>",
            Op::Gt => ">",
            Op::Ge => ">=",
            Op::Lt => "<",
            Op::Le => "<=",
            _ => return None,
        })
    }

    fn of_symbol(s: &str) -> Option<Op> {
        Some(match s {
            "=" | "==" => Op::Eq,
            "<>" => Op::Ne,
            ">" => Op::Gt,
            ">=" => Op::Ge,
            "<" => Op::Lt,
            "<=" => Op::Le,
            _ => return None,
        })
    }
}

/// How a row joins the rows before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Join {
    #[default]
    And,
    Or,
}

/// One row: `column` `op` `value` (the value as typed: text, a number, true/false, a date
/// `yyyy-mm-dd`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Cond {
    /// How it joins the rows above (the first row's is not used).
    pub join: Join,
    pub column: String,
    pub op: Op,
    pub value: String,
}

/// What a column holds, from its snapshot `data_type`, for the literal written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    Text,
    Number,
    Bool,
    Date,
}

impl Ty {
    pub fn of(data_type: &str) -> Ty {
        match data_type {
            "int64" | "double" | "decimal" => Ty::Number,
            "boolean" => Ty::Bool,
            "dateTime" => Ty::Date,
            _ => Ty::Text,
        }
    }
}

fn quote(v: &str) -> String {
    format!("\"{}\"", v.replace('"', "\"\""))
}

/// `yyyy-mm-dd` as numbers.
fn ymd(v: &str) -> Option<(i32, u32, u32)> {
    let mut it = v.trim().split('-');
    let y = it.next()?.trim().parse().ok()?;
    let m = it.next()?.trim().parse().ok()?;
    let d = it.next()?.trim().parse().ok()?;
    (it.next().is_none() && (1..=12).contains(&m) && (1..=31).contains(&d)).then_some((y, m, d))
}

/// The DAX literal of `v` for a column of type `ty`: a number, TRUE()/FALSE() and DATE(…) when
/// the text reads as one, else a string.
pub fn literal(v: &str, ty: Ty) -> String {
    let t = v.trim();
    match ty {
        Ty::Number if !t.is_empty() && t.parse::<f64>().is_ok_and(f64::is_finite) => t.to_string(),
        Ty::Bool if t.eq_ignore_ascii_case("true") => "TRUE()".into(),
        Ty::Bool if t.eq_ignore_ascii_case("false") => "FALSE()".into(),
        Ty::Date => match ymd(t) {
            Some((y, m, d)) => format!("DATE({y}, {m}, {d})"),
            None => quote(v),
        },
        _ => quote(v),
    }
}

fn condition(c: &Cond, ty: &dyn Fn(&str) -> Ty) -> String {
    let col = dax_text::bracket(&c.column);
    let text = quote(&c.value);
    match c.op {
        Op::Blank => format!("ISBLANK({col})"),
        Op::NotBlank => format!("NOT ISBLANK({col})"),
        Op::Contains => format!("CONTAINSSTRING({col}, {text})"),
        Op::NotContains => format!("NOT CONTAINSSTRING({col}, {text})"),
        op => format!(
            "{col} {} {}",
            op.symbol().unwrap_or("="),
            literal(&c.value, ty(&c.column))
        ),
    }
}

/// The DAX of the rows (`""`: no filter); rows without a column are left out. `ty` gives a
/// column's type.
pub fn to_dax(rows: &[Cond], ty: &dyn Fn(&str) -> Ty) -> String {
    let mut out = String::new();
    let mut has_or = false;
    for c in rows.iter().filter(|c| !c.column.trim().is_empty()) {
        let cond = condition(c, ty);
        if out.is_empty() {
            out = cond;
            continue;
        }
        match c.join {
            Join::Or => {
                has_or = true;
                out = format!("{out} || {cond}");
            }
            Join::And if has_or => {
                has_or = false;
                out = format!("({out}) && {cond}");
            }
            Join::And => out = format!("{out} && {cond}"),
        }
    }
    out
}

/// Most rows read back.
const MAX_ROWS: usize = 64;
/// Longest filter read back (bytes).
const MAX_BYTES: usize = 16 * 1024;

/// The rows of `dax` when it has the default editor's shape (columns of `table` only), else
/// `None`. `""` is no row.
pub fn from_dax(dax: &str, table: &str) -> Option<Vec<Cond>> {
    if dax.len() > MAX_BYTES {
        return None;
    }
    let toks: Vec<Tok> = dax_text::lex(dax)
        .into_iter()
        .filter(|t| t.class != Class::Comment)
        .collect();
    if toks.is_empty() {
        return Some(Vec::new());
    }
    let mut p = Parser {
        text: dax,
        toks: &toks,
        at: 0,
        table,
    };
    let rows = p.chain(0)?;
    (p.at == toks.len() && rows.len() <= MAX_ROWS).then_some(rows)
}

struct Parser<'a> {
    text: &'a str,
    toks: &'a [Tok],
    at: usize,
    table: &'a str,
}

impl Parser<'_> {
    fn s(&self, i: usize) -> &str {
        self.toks
            .get(i)
            .and_then(|t| self.text.get(t.range.clone()))
            .unwrap_or("")
    }

    fn class(&self, i: usize) -> Option<Class> {
        self.toks.get(i).map(|t| t.class)
    }

    /// Two operator tokens with nothing between them (`&&`, `<=`).
    fn joined(&self, i: usize) -> bool {
        match (self.toks.get(i), self.toks.get(i + 1)) {
            (Some(a), Some(b)) => a.range.end == b.range.start && b.class == Class::Operator,
            _ => false,
        }
    }

    fn eat(&mut self, s: &str) -> bool {
        if self.s(self.at) == s {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn word(&mut self, w: &str) -> bool {
        if self.s(self.at).eq_ignore_ascii_case(w) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    /// `&&` or `||` here.
    fn join(&mut self) -> Option<Join> {
        let j = match self.s(self.at) {
            "&" if self.joined(self.at) && self.s(self.at + 1) == "&" => Join::And,
            "|" if self.joined(self.at) && self.s(self.at + 1) == "|" => Join::Or,
            _ => return None,
        };
        self.at += 2;
        Some(j)
    }

    /// Rows joined left to right; a `(` group only first (the shape `to_dax` writes), and no
    /// `&&` after a `||` outside it (DAX would read that differently).
    fn chain(&mut self, depth: usize) -> Option<Vec<Cond>> {
        if depth > 8 {
            return None;
        }
        let mut rows = if self.s(self.at) == "(" {
            self.at += 1;
            let r = self.chain(depth + 1)?;
            self.eat(")").then_some(())?;
            r
        } else {
            vec![self.cond()?]
        };
        let mut seen_or = false;
        while let Some(j) = self.join() {
            if j == Join::And && seen_or {
                return None;
            }
            seen_or |= j == Join::Or;
            let mut c = self.cond()?;
            c.join = j;
            rows.push(c);
        }
        Some(rows)
    }

    /// `[Col]` or `Table[Col]` / `'Table'[Col]` of the dialog's table.
    fn column(&mut self) -> Option<String> {
        if self.class(self.at) == Some(Class::Table) {
            let t = dax_text::unquote_table(self.s(self.at));
            if !t.eq_ignore_ascii_case(self.table) {
                return None;
            }
            self.at += 1;
        }
        if self.class(self.at) != Some(Class::Column) {
            return None;
        }
        let s = self.s(self.at);
        let name = s.strip_prefix('[')?.strip_suffix(']')?.replace("]]", "]");
        self.at += 1;
        Some(name)
    }

    fn string(&mut self) -> Option<String> {
        if self.class(self.at) != Some(Class::String) {
            return None;
        }
        let s = self.s(self.at);
        let v = s.strip_prefix('"')?.strip_suffix('"')?.replace("\"\"", "\"");
        self.at += 1;
        Some(v)
    }

    fn number(&mut self) -> Option<String> {
        let neg = self.s(self.at) == "-" && self.class(self.at + 1) == Some(Class::Number);
        if neg {
            self.at += 1;
        }
        if self.class(self.at) != Some(Class::Number) {
            return None;
        }
        let n = self.s(self.at).to_string();
        self.at += 1;
        Some(if neg { format!("-{n}") } else { n })
    }

    fn call0(&mut self, f: &str) -> bool {
        let at = self.at;
        if self.word(f) && self.eat("(") && self.eat(")") {
            return true;
        }
        self.at = at;
        false
    }

    /// A literal as the default editor shows it.
    fn value(&mut self) -> Option<String> {
        if let Some(v) = self.string() {
            return Some(v);
        }
        if self.call0("TRUE") {
            return Some("true".into());
        }
        if self.call0("FALSE") {
            return Some("false".into());
        }
        if self.word("DATE") {
            self.eat("(").then_some(())?;
            let y = self.number()?;
            self.eat(",").then_some(())?;
            let m = self.number()?;
            self.eat(",").then_some(())?;
            let d = self.number()?;
            self.eat(")").then_some(())?;
            let (y, m, d): (i32, u32, u32) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
            return Some(format!("{y:04}-{m:02}-{d:02}"));
        }
        self.number()
    }

    fn cond(&mut self) -> Option<Cond> {
        let not = self.word("NOT");
        if self.word("ISBLANK") {
            self.eat("(").then_some(())?;
            let column = self.column()?;
            self.eat(")").then_some(())?;
            let op = if not { Op::NotBlank } else { Op::Blank };
            return Some(Cond {
                column,
                op,
                ..Cond::default()
            });
        }
        if self.word("CONTAINSSTRING") {
            self.eat("(").then_some(())?;
            let column = self.column()?;
            self.eat(",").then_some(())?;
            let value = self.string()?;
            self.eat(")").then_some(())?;
            let op = if not { Op::NotContains } else { Op::Contains };
            return Some(Cond {
                column,
                op,
                value,
                ..Cond::default()
            });
        }
        if not {
            return None;
        }
        let column = self.column()?;
        let mut sym = self.s(self.at).to_string();
        if self.class(self.at) != Some(Class::Operator) {
            return None;
        }
        if self.joined(self.at) && matches!(self.s(self.at + 1), "=" | ">") {
            sym.push_str(self.s(self.at + 1));
            self.at += 1;
        }
        self.at += 1;
        let op = Op::of_symbol(&sym)?;
        let value = self.value()?;
        Some(Cond {
            column,
            op,
            value,
            ..Cond::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ty(c: &str) -> Ty {
        match c {
            "Amount" => Ty::Number,
            "Open" => Ty::Bool,
            "Day" => Ty::Date,
            _ => Ty::Text,
        }
    }

    fn row(join: Join, column: &str, op: Op, value: &str) -> Cond {
        Cond {
            join,
            column: column.into(),
            op,
            value: value.into(),
        }
    }

    #[test]
    fn rows_become_dax_and_read_back() {
        let rows = vec![
            row(Join::And, "Region", Op::Eq, "E \"x\""),
            row(Join::Or, "Amount", Op::Ge, "-10.5"),
            row(Join::And, "Open", Op::Eq, "TRUE"),
            row(Join::And, "Day", Op::Lt, "2024-02-03"),
            row(Join::Or, "Name", Op::NotContains, "a"),
            row(Join::And, "Na]me", Op::NotBlank, ""),
        ];
        let dax = to_dax(&rows, &ty);
        assert_eq!(
            dax,
            "(([Region] = \"E \"\"x\"\"\" || [Amount] >= -10.5) && [Open] = TRUE() && [Day] < DATE(2024, 2, 3) \
             || NOT CONTAINSSTRING([Name], \"a\")) && NOT ISBLANK([Na]]me])"
        );
        let back = from_dax(&dax, "Sales").unwrap();
        assert_eq!(back.len(), 6);
        assert_eq!(back[0], row(Join::And, "Region", Op::Eq, "E \"x\""));
        assert_eq!(back[1], row(Join::Or, "Amount", Op::Ge, "-10.5"));
        assert_eq!(back[2], row(Join::And, "Open", Op::Eq, "true"));
        assert_eq!(back[3], row(Join::And, "Day", Op::Lt, "2024-02-03"));
        assert_eq!(back[4], row(Join::Or, "Name", Op::NotContains, "a"));
        assert_eq!(back[5], row(Join::And, "Na]me", Op::NotBlank, ""));
        assert_eq!(to_dax(&back, &ty), dax);
    }

    #[test]
    fn filters_of_other_shapes_stay_in_the_dax_editor() {
        assert_eq!(from_dax("", "Sales"), Some(Vec::new()));
        assert_eq!(
            from_dax("'Sales'[Region] <> \"W\"", "Sales").unwrap()[0],
            row(Join::And, "Region", Op::Ne, "W")
        );
        assert_eq!(
            from_dax("Sales[Amount]>3", "sales").unwrap()[0],
            row(Join::And, "Amount", Op::Gt, "3")
        );
        for other in [
            "[Email] = USERPRINCIPALNAME()",
            "Region[Region] = \"E\"",
            "[a] = 1 ||",
            "([a] = 1",
            "[a] = 1 || [b] = 2 && [c] = 3",
            "NOT [a] = 1",
            "TRUE()",
            "[a] = [b]",
        ] {
            assert_eq!(from_dax(other, "Sales"), None, "{other}");
        }
        // any text: no panic
        for t in [
            "(((((((((((",
            "\"",
            "[",
            "&&",
            "NOT",
            "DATE(1,",
            "-",
            "CONTAINSSTRING([a],",
        ] {
            let _ = from_dax(t, "Sales");
        }
    }

    #[test]
    fn a_value_that_is_not_of_the_column_type_is_text() {
        assert_eq!(literal("12", Ty::Number), "12");
        assert_eq!(literal("12a", Ty::Number), "\"12a\"");
        assert_eq!(literal("inf", Ty::Number), "\"inf\"");
        assert_eq!(literal("yes", Ty::Bool), "\"yes\"");
        assert_eq!(literal("2024-13-01", Ty::Date), "\"2024-13-01\"");
        assert_eq!(to_dax(&[row(Join::And, " ", Op::Eq, "x")], &ty), "");
    }
}
