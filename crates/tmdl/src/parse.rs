//! TMDL text -> generic object tree.
//!
//! Every non-blank line outside an expression is one *entry*: an object declaration
//! (`measure 'Sales Amount' = ...`), a property (`formatString: 0.00`, `source = ...`), or a
//! boolean shortcut (`isHidden`). Entries nest by indentation, so the tree does not need to know
//! which keywords are objects and which are properties; the mapping step decides that. The one
//! exception is where a multi-line expression starts, which depends on whether its line declares
//! an object (see [`expression_base`]).

use crate::Error;

/// Deepest indentation level accepted. Real models stay below 10.
pub(crate) const MAX_DEPTH: usize = 64;
/// Spaces per indentation level when a document gives no hint.
const DEFAULT_SPACE_UNIT: usize = 4;
pub(crate) const FENCE: &str = "```";

/// One parsed TMDL document: an arena of entries plus the top-level entry ids.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Document {
    pub nodes: Vec<Node>,
    pub roots: Vec<usize>,
}

/// How an entry's value was written.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ValueKind {
    /// No value: `isHidden`, `table Sales`.
    #[default]
    None,
    /// `key: text`, unquoted.
    Text,
    /// `key: "text"`, double-quoted (always a string).
    QuotedText,
    /// `key = expression` or `kind Name = expression`.
    Expression,
}

/// One entry of a TMDL document.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Node {
    /// Object type or property name, lowercased (TMDL keywords are case-insensitive).
    pub kind: String,
    /// The keyword as written, e.g. `formatString`.
    pub keyword: String,
    /// Unquoted object name, for declarations such as `table 'My Table'`.
    pub name: Option<String>,
    /// Value after `:` (text, double quotes stripped) or `=` (expression, verbatim).
    pub value: Option<String>,
    pub value_kind: ValueKind,
    /// The trimmed text after `:` before double quotes are stripped, for values that are object
    /// names or references (`sortByColumn: "x"` names the column `"x"`).
    pub raw: Option<String>,
    /// Text of the `///` lines right above the entry, joined with `\n`.
    pub description: Option<String>,
    /// `ref table X` style references, which only fix collection order.
    pub is_ref: bool,
    /// 1-based line of the entry in its file.
    pub line: usize,
    pub children: Vec<usize>,
}

impl Document {
    pub fn node(&self, id: usize) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// Children of `node` whose kind is `kind` (lowercase).
    pub fn children_of<'a>(&'a self, node: &'a Node, kind: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        node.children
            .iter()
            .filter_map(|&id| self.nodes.get(id))
            .filter(move |n| n.kind == kind)
    }

    /// The value of the first child property `kind`, if any.
    pub fn prop<'a>(&'a self, node: &'a Node, kind: &str) -> Option<&'a str> {
        node.children
            .iter()
            .filter_map(|&id| self.nodes.get(id))
            .find(|n| n.kind == kind)
            .and_then(|n| n.value.as_deref())
    }

    /// A boolean property: `isHidden` alone means true, otherwise `true`/`false`.
    pub fn flag(&self, node: &Node, kind: &str, default: bool) -> bool {
        let Some(child) = node
            .children
            .iter()
            .filter_map(|&id| self.nodes.get(id))
            .find(|n| n.kind == kind)
        else {
            return default;
        };
        match child.value.as_deref().map(str::trim) {
            None => true,
            Some(v) if v.eq_ignore_ascii_case("true") => true,
            Some(v) if v.eq_ignore_ascii_case("false") => false,
            Some(_) => default,
        }
    }
}

enum Rhs<'a> {
    None,
    Colon(&'a str),
    Equals(&'a str),
}

struct Header<'a> {
    is_ref: bool,
    keyword: &'a str,
    name: Option<String>,
    rhs: Rhs<'a>,
}

/// Keywords of nameless *objects* whose default property is written after `=`
/// (`formatStringDefinition = ...`). Their multi-line expressions sit two levels deeper, like a
/// named declaration's; a plain property's (`targetExpression =`) sits one level deeper.
pub(crate) fn is_nameless_object(kind: &str) -> bool {
    [
        "source",
        "formatstringdefinition",
        "detailrowsdefinition",
        "defaultdetailrowsdefinition",
        "multipleoremptyselectionexpression",
        "noselectionexpression",
        "linguisticmetadata",
        "changedproperty",
        "options",
        "credential",
        "datacoveragedefinition",
        "calendarcolumngroup",
    ]
    .contains(&kind)
}

/// The indentation level of a multi-line expression's lines, given its entry's level.
fn expression_base(level: usize, named: bool, kind: &str) -> usize {
    if named || is_nameless_object(kind) {
        level.saturating_add(2)
    } else {
        level.saturating_add(1)
    }
}

/// Parses one TMDL document. `file` is only used in error messages.
pub fn parse(file: &str, text: &str) -> Result<Document, Error> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines: Vec<&str> = text.lines().collect();
    let unit = space_unit(&lines);
    let err = |line: usize, message: String| Error::Syntax {
        file: file.to_string(),
        line,
        message,
    };

    let mut doc = Document::default();
    let mut stack: Vec<(usize, usize)> = Vec::new(); // (level, node id)
    let mut description: Vec<String> = Vec::new();
    let mut i = 0;
    while let Some(raw) = lines.get(i) {
        let line_no = i.saturating_add(1);
        i = line_no;
        if raw.trim().is_empty() {
            description.clear();
            continue;
        }
        let level = indent_level(raw, unit);
        if level > MAX_DEPTH {
            return Err(err(
                line_no,
                format!("indentation deeper than {MAX_DEPTH} levels"),
            ));
        }
        let content = raw.trim_start();
        if let Some(desc) = content.strip_prefix("///") {
            // Trailing whitespace is part of the description.
            description.push(desc.strip_prefix(' ').unwrap_or(desc).to_string());
            continue;
        }
        let content = content.trim_end();
        let header = header(content).map_err(|m| err(line_no, m))?;
        let kind = header.keyword.to_ascii_lowercase();
        let mut raw = None;
        let (value, value_kind) = match header.rhs {
            Rhs::None => (None, ValueKind::None),
            Rhs::Colon(v) => {
                raw = Some(v.trim().to_string());
                let (text, quoted) = text_value(v);
                let kind = if quoted {
                    ValueKind::QuotedText
                } else {
                    ValueKind::Text
                };
                (Some(text), kind)
            }
            Rhs::Equals(v) => {
                let base = expression_base(level, header.name.is_some(), &kind);
                let v = v.trim();
                let expr = if v == FENCE {
                    let (expr, next) = fenced(&lines, i)
                        .ok_or_else(|| err(line_no, "unterminated ``` expression".to_string()))?;
                    i = next;
                    expr
                } else if let Some(inner) = v.strip_prefix(FENCE).and_then(|s| s.strip_suffix(FENCE)) {
                    inner.to_string()
                } else if v.starts_with(FENCE) {
                    return Err(err(
                        line_no,
                        "text after an opening ``` must start on the next line".to_string(),
                    ));
                } else {
                    let lenient = header.name.is_none() && is_nameless_object(&kind);
                    let (expr, next) = indented(&lines, i, (level, base, lenient), unit, v);
                    i = next;
                    expr
                };
                (Some(expr), ValueKind::Expression)
            }
        };

        while stack.last().is_some_and(|&(l, _)| l >= level) {
            stack.pop();
        }
        let id = doc.nodes.len();
        doc.nodes.push(Node {
            kind,
            keyword: header.keyword.to_string(),
            name: header.name,
            value,
            value_kind,
            raw,
            description: (!description.is_empty()).then(|| description.join("\n")),
            is_ref: header.is_ref,
            line: line_no,
            children: Vec::new(),
        });
        description.clear();
        match stack.last() {
            Some(&(_, parent)) => match doc.nodes.get_mut(parent) {
                Some(p) => p.children.push(id),
                None => return Err(err(line_no, "internal: dangling parent".to_string())),
            },
            None => doc.roots.push(id),
        }
        stack.push((level, id));
    }
    Ok(doc)
}

/// Splits an entry line into `[ref] kind [name] [= value | : value]`.
fn header(content: &str) -> Result<Header<'_>, String> {
    let (word, mut rest) = identifier(content)?;
    let mut is_ref = false;
    let mut keyword = word;
    // `ref <objectType> <name>`; `ref = x` and `ref : x` are a property named `ref`.
    if word.eq_ignore_ascii_case("ref")
        && rest.starts_with(char::is_whitespace)
        && rest.trim_start().starts_with(keyword_char)
    {
        is_ref = true;
        let (w, r) = identifier(rest.trim_start())?;
        keyword = w;
        rest = r;
    }
    let trimmed = rest.trim_start();
    let simple = |rhs| {
        Ok(Header {
            is_ref,
            keyword,
            name: None,
            rhs,
        })
    };
    if trimmed.is_empty() {
        return simple(Rhs::None);
    }
    // `key: value` has no space before the colon; `kind :name` (space, then a colon followed by
    // more name characters) declares an object whose name starts with a colon, as Desktop
    // writes it.
    if let Some(v) = trimmed.strip_prefix(':')
        && (!rest.starts_with(char::is_whitespace) || v.is_empty() || v.starts_with(char::is_whitespace))
    {
        return simple(Rhs::Colon(v));
    }
    if let Some(v) = trimmed.strip_prefix('=') {
        return simple(Rhs::Equals(v));
    }
    if !rest.starts_with(char::is_whitespace) {
        return Err(format!("unexpected `{trimmed}` after `{keyword}`"));
    }
    let (name, after) = object_name(trimmed)?;
    let after = after.trim_start();
    let rhs = if after.is_empty() {
        Rhs::None
    } else if let Some(v) = after.strip_prefix('=') {
        Rhs::Equals(v)
    } else {
        return Err(format!(
            "unexpected `{after}` after the name `{name}` (names with spaces or . = ' must be quoted)"
        ));
    };
    Ok(Header {
        is_ref,
        keyword,
        name: Some(name),
        rhs,
    })
}

/// A character of an object type or property name.
fn keyword_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether `s` can be written as an object type or property name: the reader takes a keyword
/// as a non-empty run of letters, digits and `_`.
pub(crate) fn is_keyword(s: &str) -> bool {
    !s.is_empty() && s.chars().all(keyword_char)
}

fn identifier(s: &str) -> Result<(&str, &str), String> {
    let end = s
        .char_indices()
        .find(|&(_, c)| !keyword_char(c))
        .map_or(s.len(), |(i, _)| i);
    if end == 0 {
        let shown: String = s.chars().take(40).collect();
        return Err(format!(
            "expected an object type or property name, found `{shown}`"
        ));
    }
    Ok((s.get(..end).unwrap_or_default(), s.get(end..).unwrap_or_default()))
}

/// Reads one object name: `'quoted '' name'` or a bare run up to whitespace or `=`.
pub(crate) fn object_name(s: &str) -> Result<(String, &str), String> {
    if let Some(body) = s.strip_prefix('\'') {
        let mut name = String::new();
        let mut chars = body.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c == '\'' {
                if chars.peek().is_some_and(|&(_, n)| n == '\'') {
                    chars.next();
                    name.push('\'');
                } else {
                    return Ok((name, body.get(i.saturating_add(1)..).unwrap_or_default()));
                }
            } else {
                name.push(c);
            }
        }
        return Err("unterminated quoted name".to_string());
    }
    let end = s
        .char_indices()
        .find(|&(_, c)| c.is_whitespace() || c == '=')
        .map_or(s.len(), |(i, _)| i);
    if end == 0 {
        return Err("expected a name".to_string());
    }
    Ok((
        s.get(..end).unwrap_or_default().to_string(),
        s.get(end..).unwrap_or_default(),
    ))
}

/// A `key: value` text value: trimmed, with optional surrounding double quotes removed (`""`
/// inside quotes is one `"`). Returns whether the value was quoted.
fn text_value(v: &str) -> (String, bool) {
    let v = v.trim();
    match v.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        Some(inner) if v.len() >= 2 => (inner.replace("\"\"", "\""), true),
        _ => (v.to_string(), false),
    }
}

/// Collects an indentation-delimited expression starting at line index `start`. `first` is the
/// text already on the declaration line (empty for the usual multi-line form). Lines belong to
/// the expression while they are blank or indented at least `base` levels; that many levels are
/// stripped. With `lenient`, a first line indented less than `base` (but deeper than the entry's
/// `level`) lowers the base: nothing else can follow a nameless object's `=` at that depth.
fn indented(
    lines: &[&str],
    start: usize,
    (level, base, lenient): (usize, usize, bool),
    unit: usize,
    first: &str,
) -> (String, usize) {
    let first_level = lines
        .get(start..)
        .unwrap_or_default()
        .iter()
        .find(|l| !l.trim().is_empty())
        .map(|l| indent_level(l, unit));
    let base = match first_level {
        Some(f) if lenient && first.is_empty() && f > level => base.min(f),
        _ => base,
    };
    let mut out: Vec<String> = Vec::new();
    if !first.is_empty() {
        out.push(first.to_string());
    }
    let mut j = start;
    while let Some(line) = lines.get(j) {
        if line.trim().is_empty() {
            out.push(String::new());
        } else if indent_level(line, unit) >= base && indent_level(line, unit) > level {
            out.push(strip_levels(line, base, unit).to_string());
        } else {
            break;
        }
        j = j.saturating_add(1);
    }
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    let expr = out.join("\n");
    (expr.trim_end().to_string(), j)
}

/// Collects a ``` fenced expression whose body starts at line index `start`. The closing
/// delimiter (a line that is exactly ``` apart from indentation) sets the left margin removed
/// from each line. Returns `None` when unterminated.
fn fenced(lines: &[&str], start: usize) -> Option<(String, usize)> {
    let mut j = start;
    loop {
        let line = lines.get(j)?;
        let body = line.trim_start();
        if body.trim_end() == FENCE {
            let margin = line.len().saturating_sub(body.len());
            let content = lines.get(start..j)?;
            let expr = content
                .iter()
                .map(|l| strip_whitespace(l, margin))
                .collect::<Vec<_>>()
                .join("\n");
            return Some((expr, j.saturating_add(1)));
        }
        j = j.saturating_add(1);
    }
}

/// Removes at most `n` leading whitespace bytes (tabs and spaces are single bytes).
fn strip_whitespace(line: &str, n: usize) -> &str {
    let ws = line
        .bytes()
        .take(n)
        .take_while(|b| *b == b' ' || *b == b'\t')
        .count();
    line.get(ws..).unwrap_or(line)
}

/// Indentation level: each tab is one level, each `unit` spaces are one level.
fn indent_level(line: &str, unit: usize) -> usize {
    let mut tabs = 0usize;
    let mut spaces = 0usize;
    for b in line.bytes() {
        match b {
            b'\t' => tabs = tabs.saturating_add(1),
            b' ' => spaces = spaces.saturating_add(1),
            _ => break,
        }
    }
    tabs.saturating_add(spaces / unit.max(1))
}

/// Removes the first `levels` indentation levels from `line`.
fn strip_levels(line: &str, levels: usize, unit: usize) -> &str {
    let mut done = 0usize;
    let mut spaces = 0usize;
    let mut cut = 0usize;
    for (i, b) in line.bytes().enumerate() {
        if done >= levels {
            break;
        }
        match b {
            b'\t' => {
                done = done.saturating_add(1);
                spaces = 0;
            }
            b' ' => {
                spaces = spaces.saturating_add(1);
                if spaces >= unit.max(1) {
                    done = done.saturating_add(1);
                    spaces = 0;
                }
            }
            _ => break,
        }
        cut = i.saturating_add(1);
    }
    line.get(cut..).unwrap_or(line)
}

/// Spaces per level for space-indented documents: the smallest indentation of an entry line
/// indented only with spaces. Fenced bodies (verbatim) and the bodies of multi-line expressions
/// (indented relative to their entry, two levels for objects) are not entries; when the entries
/// give no hint, the expression bodies do: (body indentation - entry indentation) / levels.
fn space_unit(lines: &[&str]) -> usize {
    fn smaller(a: Option<usize>, b: usize) -> Option<usize> {
        Some(a.map_or(b, |a| a.min(b)))
    }
    let mut entries: Option<usize> = None;
    let mut bodies: Option<usize> = None;
    let mut in_fence = false;
    // Open multi-line expression: (entry indentation, levels to its body, smallest body indent).
    let mut open: Option<(usize, usize, Option<usize>)> = None;
    let close = |open: Option<(usize, usize, Option<usize>)>, bodies: &mut Option<usize>| {
        if let Some((entry, levels, Some(body))) = open
            && let Some(unit) = body.checked_sub(entry).and_then(|d| d.checked_div(levels))
            && unit > 0
        {
            *bodies = smaller(*bodies, unit);
        }
    };
    for line in lines {
        let body = line.trim_start();
        if in_fence {
            in_fence = body.trim_end() != FENCE;
            continue;
        }
        if body.is_empty() {
            continue;
        }
        let indent = line.len().saturating_sub(body.len());
        let spaces = indent > 0 && line.bytes().take(indent).all(|b| b == b' ');
        if let Some((entry, levels, min)) = open {
            if indent > entry {
                if spaces {
                    open = Some((entry, levels, smaller(min, indent)));
                }
                continue;
            }
            close(open, &mut bodies);
            open = None;
        }
        let content = body.trim_end();
        if content
            .strip_suffix(FENCE)
            .is_some_and(|s| s.trim_end().ends_with('='))
        {
            in_fence = true;
        }
        if spaces {
            entries = smaller(entries, indent);
        }
        if !content.starts_with("///")
            && let Ok(h) = header(content)
            && matches!(h.rhs, Rhs::Equals(v) if v.trim().is_empty())
        {
            let levels = expression_base(0, h.name.is_some(), &h.keyword.to_ascii_lowercase());
            open = Some((indent, levels, None));
        }
    }
    close(open, &mut bodies);
    entries.or(bodies).unwrap_or(DEFAULT_SPACE_UNIT)
}

/// Parses a dotted object reference such as `Sales.'Product Key'` into its name parts.
pub(crate) fn reference(s: &str) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    let mut rest = s.trim();
    loop {
        let (name, after) = if rest.starts_with('\'') {
            object_name(rest).ok()?
        } else {
            let end = rest.find('.').unwrap_or(rest.len());
            let name = rest.get(..end)?.trim();
            if name.is_empty() {
                return None;
            }
            (name.to_string(), rest.get(end..)?)
        };
        parts.push(name);
        let after = after.trim_start();
        if after.is_empty() {
            return Some(parts);
        }
        rest = after.strip_prefix('.')?.trim_start();
        if parts.len() >= 8 {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(doc: &Document, ids: &[usize]) -> Vec<String> {
        ids.iter().map(|&i| doc.nodes[i].kind.clone()).collect()
    }

    #[test]
    fn nests_by_indentation() {
        let doc = parse(
            "t",
            "table A\n\tisHidden\n\tcolumn B\n\t\tdataType: int64\ntable C\n",
        )
        .unwrap();
        assert_eq!(kinds(&doc, &doc.roots), ["table", "table"]);
        let a = &doc.nodes[doc.roots[0]];
        assert_eq!(kinds(&doc, &a.children), ["ishidden", "column"]);
        assert_eq!(doc.nodes[a.children[0]].keyword, "isHidden");
        let b = doc.children_of(a, "column").next().unwrap();
        assert_eq!(doc.prop(b, "datatype"), Some("int64"));
        assert!(doc.flag(a, "ishidden", false));
    }

    #[test]
    fn quoted_names_and_escapes() {
        let (n, rest) = object_name("'It''s a name' = 1").unwrap();
        assert_eq!(n, "It's a name");
        assert_eq!(rest, " = 1");
        assert!(object_name("'open").is_err());
        assert_eq!(object_name("a:b = 1").unwrap().0, "a:b");
        assert_eq!(
            reference("'Sales Table'.'Unit''s Price'").unwrap(),
            ["Sales Table", "Unit's Price"]
        );
        assert_eq!(reference("Sales.Key").unwrap(), ["Sales", "Key"]);
        assert_eq!(reference("'a.b'.c").unwrap(), ["a.b", "c"]);
        assert!(reference("Sales.").is_none());
        assert!(reference("").is_none());
    }

    #[test]
    fn text_values_strip_double_quotes() {
        assert_eq!(
            text_value(r#" " My ""Amazing"" Folder" "#),
            (r#" My "Amazing" Folder"#.to_string(), true)
        );
        assert_eq!(text_value(" $ #,##0 "), ("$ #,##0".to_string(), false));
        assert_eq!(text_value("\""), ("\"".to_string(), false));
        assert_eq!(text_value("a\"b"), ("a\"b".to_string(), false));
    }

    #[test]
    fn multi_line_expression_strips_outer_indent() {
        let src =
            "table T\n\tmeasure M =\n\t\t\tVAR x = 1\n\n\t\t\tRETURN\n\t\t\t\tx\n\t\t\n\t\tformatString: 0\n";
        let doc = parse("t", src).unwrap();
        let t = &doc.nodes[doc.roots[0]];
        let m = doc.children_of(t, "measure").next().unwrap();
        assert_eq!(m.value.as_deref(), Some("VAR x = 1\n\nRETURN\n\tx"));
        assert_eq!(m.value_kind, ValueKind::Expression);
        assert_eq!(doc.prop(m, "formatstring"), Some("0"));
    }

    #[test]
    fn source_expressions_sit_two_levels_deeper_but_one_is_accepted() {
        // Canonical: `source` is an object, so its expression is two levels deeper.
        let canonical = "table T\n\tpartition P = m\n\t\tmode: import\n\t\tsource =\n\t\t\t\tlet\n\t\t\t\t\tx = 1\n\t\t\t\tin\n\t\t\t\t\tx\n\t\tqueryGroup: G\n";
        // Lenient: one level deeper.
        let lenient = canonical.replace("\t\t\t\t", "\t\t\t");
        for src in [canonical.to_string(), lenient] {
            let doc = parse("t", &src).unwrap();
            let t = &doc.nodes[doc.roots[0]];
            let p = doc.children_of(t, "partition").next().unwrap();
            assert_eq!(p.value.as_deref(), Some("m"));
            assert_eq!(doc.prop(p, "source"), Some("let\n\tx = 1\nin\n\tx"), "{src}");
            assert_eq!(doc.prop(p, "querygroup"), Some("G"));
        }
    }

    #[test]
    fn property_expressions_sit_one_level_deeper() {
        let src = "kpi\n\tstatusExpression =\n\t\tVAR x = 1\n\t\tRETURN\n\t\t\tx\n\tstatusGraphic: Shapes\n";
        let doc = parse("t", src).unwrap();
        let k = &doc.nodes[doc.roots[0]];
        assert_eq!(doc.prop(k, "statusexpression"), Some("VAR x = 1\nRETURN\n\tx"));
        assert_eq!(doc.prop(k, "statusgraphic"), Some("Shapes"));
    }

    #[test]
    fn fenced_expressions_keep_relative_indent() {
        let src = "table T\n\tmeasure M = ```\n\t\t\t\tVAR a = 1  \n\n\t\t\t\tRETURN a\n\t\t\t\t```x\n\t\t\t```\n\t\tformatString: 0\n\tmeasure N = ```SUM(T[x])```\n";
        let doc = parse("t", src).unwrap();
        let t = &doc.nodes[doc.roots[0]];
        let mut ms = doc.children_of(t, "measure");
        let m = ms.next().unwrap();
        assert_eq!(m.value.as_deref(), Some("\tVAR a = 1  \n\n\tRETURN a\n\t```x"));
        assert_eq!(doc.prop(m, "formatstring"), Some("0"));
        assert_eq!(ms.next().unwrap().value.as_deref(), Some("SUM(T[x])"));
        let err = parse("f.tmdl", "table T\n\tmeasure M = ```\n\t\tx\n").unwrap_err();
        assert!(err.to_string().starts_with("f.tmdl:2:"), "{err}");
    }

    #[test]
    fn descriptions_attach_to_the_next_entry() {
        let doc = parse(
            "t",
            "/// Line one\n///  Line two \n///\ntable T\n\n/// dropped\n\ntable U\n",
        )
        .unwrap();
        assert_eq!(
            doc.nodes[doc.roots[0]].description.as_deref(),
            Some("Line one\n Line two \n")
        );
        assert_eq!(doc.nodes[doc.roots[1]].description, None);
    }

    #[test]
    fn spaces_and_crlf_and_bom() {
        let src = "\u{feff}table T\r\n    measure M =\r\n            1 +\r\n              2\r\n        formatString: 0\r\n";
        let doc = parse("t", src).unwrap();
        let t = &doc.nodes[doc.roots[0]];
        let m = doc.children_of(t, "measure").next().unwrap();
        assert_eq!(m.value.as_deref(), Some("1 +\n  2"));
        assert_eq!(doc.prop(m, "formatstring"), Some("0"));
    }

    #[test]
    fn ref_entries_are_marked() {
        let doc = parse(
            "t",
            "model Model\n\tculture: en-US\n\nref table Sales\nref table 'My T'\n",
        )
        .unwrap();
        let refs: Vec<_> = doc
            .roots
            .iter()
            .map(|&i| &doc.nodes[i])
            .filter(|n| n.is_ref)
            .collect();
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[1].kind, "table");
        assert_eq!(refs[1].name.as_deref(), Some("My T"));

        // `ref` followed by `=` or `:` is a property named `ref`.
        let doc = parse("t", "block\n\tref = 1\n\tref: 2\n").unwrap();
        let block = &doc.nodes[doc.roots[0]];
        let props: Vec<_> = block.children.iter().map(|&i| &doc.nodes[i]).collect();
        assert!(props.iter().all(|n| !n.is_ref && n.kind == "ref"));
        assert_eq!(props[0].value.as_deref(), Some("1"));
        assert_eq!(props[1].value.as_deref(), Some("2"));
    }

    #[test]
    fn syntax_errors_carry_line_numbers() {
        for (src, line) in [
            ("table T\n\t= oops\n", 2),
            ("table My Table\n", 1),
            ("table 'Open\n", 1),
            ("table T\n\tcolumn C\n\t\tdataType: x\n\t\t'bad'\n", 4),
            ("table\u{1}x\n", 1),
        ] {
            match parse("f", src) {
                Err(Error::Syntax { line: l, .. }) => assert_eq!(l, line, "{src:?}"),
                other => panic!("{src:?}: {other:?}"),
            }
        }
    }

    #[test]
    fn hostile_input_never_panics() {
        let deep = format!("{}x\n", "\t".repeat(MAX_DEPTH + 1));
        assert!(parse("f", &deep).is_err());
        let mut nested = String::new();
        for d in 0..MAX_DEPTH {
            nested.push_str(&"\t".repeat(d));
            nested.push_str("table T\n");
        }
        assert!(parse("f", &nested).is_ok());
        for src in [
            "",
            "\n\n",
            "'",
            "a:",
            "a =",
            "a = ```",
            "a = ``````",
            "ref",
            "ref ref",
            "ref table",
            "a '",
            "\u{feff}",
            "x\r",
            "é ü",
            "a\t= ```\n```",
            "table T = ```\n \t```",
            "a =\n\n\n\tb",
            "t X =\n\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\tx",
        ] {
            let _ = parse("f", src);
        }
    }
}
