//! Interpreter for BPA rule expressions over a [`ModelView`].
//!
//! Every failure (an unknown property, a type mismatch, a null dereference, an unsupported
//! method or regular expression, the step budget) is an `Err` with a reason: the rule is then
//! reported as not evaluated for that object, never as "no violation".

use std::collections::HashMap;
use std::rc::Rc;

use bicraft_dax::ast::Expr;
use bicraft_dax::types::{ScalarType, ValueType};
use bicraft_dax::{ExprKind, Validator};
use bicraft_model::{Annotation, Properties};
use bicraft_pbir::Regex;
use serde_json::Value;

use crate::linq::{self, BinOp, Ex, UnOp};
use crate::objects::{
    ColumnKind, ModelView, Obj, Scope, bracket, calculation_items, column_kind, cultures, data_sources,
    hierarchies, is_calculated_table, levels, perspectives, quote_table,
};
use crate::tokens::{self, DaxToken};
use crate::{Analysis, Finding, Rule, RuleOutcome, severity_name};

/// Evaluation steps allowed for one object.
#[cfg(not(test))]
const OBJECT_BUDGET: u64 = 5_000_000;
/// Evaluation steps allowed for one rule over all its objects.
#[cfg(not(test))]
const RULE_BUDGET: u64 = 100_000_000;
// Smaller budgets keep the runaway-rule test fast.
#[cfg(test)]
const OBJECT_BUDGET: u64 = 300_000;
#[cfg(test)]
const RULE_BUDGET: u64 = 3_000_000;
/// Compiled regular expressions kept between evaluations (the cache is emptied beyond that).
const MAX_CACHED_REGEXES: usize = 4096;

type R<T> = Result<T, String>;

/// `DependsOn` entries: each referenced object, and whether each reference is fully qualified.
type DepList = Rc<Vec<(Obj, Vec<bool>)>>;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum V {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    /// A TOM enumeration value (compared case-insensitively with strings and other enums).
    Enum(String),
    Obj(Obj),
    List(Vec<V>),
    /// `ReferencedBy` (true) or `DependsOn` (false) of an object, before it is enumerated.
    Refs(Obj, bool),
    /// One `DependsOn` entry: the referenced object (`Key`) and whether each of its references
    /// is fully qualified (`Value`).
    Dep(Obj, Vec<bool>),
    /// One reference of a `DependsOn` entry (`FullyQualified`).
    Reference(bool),
    /// `InPerspective` of an object, indexed by perspective name.
    InPerspective(Obj),
    /// `TranslatedNames` / `TranslatedDescriptions` / `TranslatedDisplayFolders` of an object:
    /// indexed by culture (object or name), enumerated over the model's cultures. Holds the TMSL
    /// key of the translated property.
    Translations(Obj, &'static str),
    /// One token of a `Tokenize()` list: the list and the token's position in it.
    Token(Rc<Vec<DaxToken>>, usize),
}

impl V {
    fn kind(&self) -> &'static str {
        match self {
            V::Null => "null",
            V::Bool(_) => "a boolean",
            V::Num(_) => "a number",
            V::Str(_) => "a string",
            V::Enum(_) => "an enumeration value",
            V::Obj(_) => "an object",
            V::List(_) | V::Refs(..) => "a collection",
            V::Dep(..) => "a dependency entry",
            V::Reference(_) => "a reference",
            V::InPerspective(_) => "a perspective indexer",
            V::Translations(..) => "a translation indexer",
            V::Token(..) => "a DAX token",
        }
    }
}

/// Enumeration type names usable as `Type.Member` literals.
const ENUM_TYPES: &[&str] = &[
    "DataType",
    "AggregateFunction",
    "CrossFilteringBehavior",
    "RelationshipEndCardinality",
    "ColumnType",
    "PartitionSourceType",
    "ModeType",
    "SecurityFilteringBehavior",
    "ObjectType",
    "StringComparison",
    "RegexOptions",
    "EncodingHintType",
    "ModelPermission",
    "MetadataPermission",
    "DateTimeRelationshipBehavior",
    "ExpressionKind",
    "Alignment",
    "ObjectState",
    "DataViewType",
    "HierarchyHideMembersType",
    "RefreshPolicyType",
    "RelationshipType",
    "PowerBIDataSourceVersion",
    "DataSourceType",
];

fn is_enum_type(n: &str) -> bool {
    ENUM_TYPES.iter().any(|t| t.eq_ignore_ascii_case(n))
}

/// `camelCase` → `PascalCase` (TMSL enum spelling → TOM enum spelling).
fn pascal(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

fn enum_v(s: &str) -> V {
    V::Enum(pascal(s))
}

fn text(s: Option<&String>) -> V {
    V::Str(s.cloned().unwrap_or_default())
}

/// A JSON string, or an array of lines joined (TMSL splits long text into lines).
fn json_string(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Array(lines) => Some(
            lines
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    }
}

/// A JSON property value as a rule value. Identifier-like strings (TMSL enum values such as
/// `bothDirections` or `powerBI_V3`) become enum values.
fn json_v(v: &Value, name: &str) -> R<V> {
    Ok(match v {
        Value::Null => V::Null,
        Value::Bool(b) => V::Bool(*b),
        Value::Number(n) => V::Num(n.as_f64().unwrap_or(0.0)),
        Value::String(s) => {
            let enumish = s.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if enumish { enum_v(s) } else { V::Str(s.clone()) }
        }
        // TMSL splits long text into an array of lines; other arrays are collections rules
        // cannot read through a generic property.
        Value::Array(a) if a.iter().all(Value::is_string) => V::Str(json_string(Some(v)).unwrap_or_default()),
        Value::Array(_) => {
            return Err(format!(
                "property {name} is a collection bicraft does not expose to rules"
            ));
        }
        Value::Object(_) => return Err(format!("property {name} is a structure, which rules cannot read")),
    })
}

/// Looks `name` up among the object's unmodelled properties (TMSL names, case-insensitive),
/// then among its typed fields as serde writes them (`snake_case`), then in the TOM defaults of
/// properties a model may leave out.
fn fallback(extra: &Properties, typed: Option<Value>, name: &str) -> R<V> {
    if let Some((k, v)) = extra.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
        return json_v(v, k);
    }
    if let Some(Value::Object(o)) = typed
        && let Some((k, v)) = o
            .iter()
            .find(|(k, _)| k.as_str() != "extra" && k.replace('_', "").eq_ignore_ascii_case(name))
    {
        return json_v(v, k);
    }
    let d = match name.to_ascii_lowercase().as_str() {
        "iskey"
        | "isunique"
        | "isdefaultlabel"
        | "isdefaultimage"
        | "keepuniquerows"
        | "isprivate"
        | "isdatatypeinferred"
        | "excludefrommodelrefresh"
        | "relyonreferentialintegrity"
        | "discourageimplicitmeasures"
        | "isnameinferred"
        | "issimplelevel" => V::Bool(false),
        "isavailableinmdx" | "isnullable" => V::Bool(true),
        "encodinghint" => V::Enum("Default".into()),
        "securityfilteringbehavior" => V::Enum("OneDirection".into()),
        "joinondatebehavior" => V::Enum("DateAndTime".into()),
        "defaultpowerbidatasourceversion" => V::Enum("PowerBI_V1".into()),
        "alternateof" | "kpi" | "refreshpolicy" | "detailrowsdefinition" => V::Null,
        "datacategory" | "lineagetag" | "sourcelineagetag" | "description" | "displayfolder"
        | "sourceexpression" | "formatstring" | "connectionstring" | "provider" => V::Str(String::new()),
        _ => return Err(format!("property {name} is not known to bicraft")),
    };
    Ok(d)
}

fn find_annotation(list: &[Annotation], name: &str) -> Option<String> {
    list.iter().find(|a| a.name == name).map(|a| a.value.clone())
}

fn json_annotation(v: Option<&Value>, name: &str) -> Option<String> {
    v?.get("annotations")?.as_array()?.iter().find_map(|a| {
        (a.get("name")?.as_str()? == name).then(|| match a.get("value") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(_)) => json_string(a.get("value")).unwrap_or_default(),
            Some(other) => other.to_string(),
            None => String::new(),
        })
    })
}

fn nested_expr(v: Option<&Value>, key: &str) -> V {
    V::Str(json_string(v.and_then(|o| o.get(key)).and_then(|d| d.get("expression"))).unwrap_or_default())
}

fn json_text(v: Option<&Value>, key: &str) -> V {
    V::Str(json_string(v.and_then(|o| o.get(key))).unwrap_or_default())
}

fn json_bool(v: Option<&Value>, key: &str, default: bool) -> bool {
    v.and_then(|o| o.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(default)
}

fn json_fallback(v: Option<&Value>, name: &str) -> R<V> {
    match v {
        Some(Value::Object(o)) => fallback(o, None, name),
        _ => fallback(&Properties::new(), None, name),
    }
}

/// Every node of `e`, depth-first, iteratively.
fn walk(e: &Expr) -> Vec<&Expr> {
    let mut out = Vec::new();
    let mut stack = vec![e];
    while let Some(x) = stack.pop() {
        out.push(x);
        match x {
            Expr::Call { args, .. } => stack.extend(args.iter().flatten().rev()),
            Expr::Unary { operand, .. } => stack.push(operand),
            Expr::Binary { lhs, rhs, .. } => {
                stack.push(rhs);
                stack.push(lhs);
            }
            Expr::Var { bindings, body, .. } => {
                stack.push(body);
                stack.extend(bindings.iter().rev().map(|b| &b.value));
            }
            Expr::TableCtor { rows, .. } => stack.extend(rows.iter().rev()),
            Expr::Tuple { items, .. } => stack.extend(items.iter().rev()),
            Expr::Number { .. }
            | Expr::Str { .. }
            | Expr::Bool { .. }
            | Expr::Name { .. }
            | Expr::Column { .. } => {}
        }
    }
    out
}

#[derive(Clone)]
struct Env {
    it: V,
    outer: Option<V>,
    current: V,
    params: Vec<(String, V)>,
}

pub(crate) struct Interp<'a, 'm> {
    view: &'a ModelView<'m>,
    steps: u64,
    regexes: HashMap<(String, bool), Option<Rc<Regex>>>,
    /// `DependsOn` entries per object, from the DAX parser.
    deps: HashMap<Obj, DepList>,
    /// `Tokenize()` lists per object.
    tokens: HashMap<Obj, Rc<Vec<DaxToken>>>,
    /// The DAX validator that infers measure data types (built on first use).
    validator: Option<Validator<'m>>,
    /// `DataType` of measures, inferred once each.
    measure_types: HashMap<Obj, R<V>>,
}

impl<'a, 'm> Interp<'a, 'm> {
    pub(crate) fn new(view: &'a ModelView<'m>) -> Self {
        Interp {
            view,
            steps: 0,
            regexes: HashMap::new(),
            deps: HashMap::new(),
            tokens: HashMap::new(),
            validator: None,
            measure_types: HashMap::new(),
        }
    }

    /// Evaluates `ex` with `o` as `it`.
    pub(crate) fn eval_on(&mut self, ex: &Ex, o: Obj) -> R<V> {
        self.steps = 0;
        let env = Env {
            it: V::Obj(o),
            outer: None,
            current: V::Obj(o),
            params: Vec::new(),
        };
        self.eval(ex, &env)
    }

    fn tick(&mut self) -> R<()> {
        self.steps = self.steps.saturating_add(1);
        if self.steps > OBJECT_BUDGET {
            Err("the evaluation needs more steps than allowed".into())
        } else {
            Ok(())
        }
    }

    fn eval(&mut self, ex: &Ex, env: &Env) -> R<V> {
        self.tick()?;
        match ex {
            Ex::Null => Ok(V::Null),
            Ex::Bool(b) => Ok(V::Bool(*b)),
            Ex::Num(n) => Ok(V::Num(*n)),
            Ex::Str(s) => Ok(V::Str(s.clone())),
            Ex::It => Ok(env.it.clone()),
            Ex::OuterIt => env
                .outer
                .clone()
                .ok_or_else(|| "outerIt is used outside a lambda".into()),
            Ex::Current => Ok(env.current.clone()),
            Ex::Lambda(..) => Err("a lambda is only allowed as a method argument".into()),
            Ex::Name(n) => {
                if let Some((_, v)) = env.params.iter().rev().find(|(p, _)| p == n) {
                    return Ok(v.clone());
                }
                // `DIV`, `INTEGER_LITERAL`, ...: DAX token types, compared with a token's `Type`.
                if tokens::is_type_name(n) {
                    return Ok(V::Enum(n.clone()));
                }
                self.member(&env.it, n)
            }
            Ex::Member(base, m) => {
                if let Ex::Name(t) = base.as_ref()
                    && is_enum_type(t)
                    && !env.params.iter().any(|(p, _)| p == t)
                {
                    return Ok(V::Enum(m.clone()));
                }
                let b = self.eval(base, env)?;
                self.member(&b, m)
            }
            Ex::Index(base, i) => {
                let b = self.eval(base, env)?;
                let i = self.eval(i, env)?;
                self.index(&b, &i)
            }
            Ex::Call(target, name, args) => self.call(target.as_deref(), name, args, env),
            Ex::Unary(op, e) => {
                let v = self.eval(e, env)?;
                match (op, v) {
                    (UnOp::Not, V::Bool(b)) => Ok(V::Bool(!b)),
                    (UnOp::Neg, V::Num(n)) => Ok(V::Num(-n)),
                    (UnOp::Not, v) => Err(format!("`!` applied to {}", v.kind())),
                    (UnOp::Neg, v) => Err(format!("`-` applied to {}", v.kind())),
                }
            }
            Ex::Cond(c, a, b) => match self.eval(c, env)? {
                V::Bool(true) => self.eval(a, env),
                V::Bool(false) => self.eval(b, env),
                v => Err(format!("the condition of `?:` is {}, not a boolean", v.kind())),
            },
            Ex::Binary(op, l, r) => self.binary(*op, l, r, env),
        }
    }

    fn truth(v: V, what: &str) -> R<bool> {
        match v {
            V::Bool(b) => Ok(b),
            v => Err(format!("`{what}` applied to {}", v.kind())),
        }
    }

    fn binary(&mut self, op: BinOp, l: &Ex, r: &Ex, env: &Env) -> R<V> {
        match op {
            BinOp::And => {
                if !Self::truth(self.eval(l, env)?, "and")? {
                    return Ok(V::Bool(false));
                }
                return Ok(V::Bool(Self::truth(self.eval(r, env)?, "and")?));
            }
            BinOp::Or => {
                if Self::truth(self.eval(l, env)?, "or")? {
                    return Ok(V::Bool(true));
                }
                return Ok(V::Bool(Self::truth(self.eval(r, env)?, "or")?));
            }
            BinOp::Coalesce => {
                let a = self.eval(l, env)?;
                return if a == V::Null { self.eval(r, env) } else { Ok(a) };
            }
            _ => {}
        }
        let a = self.eval(l, env)?;
        let b = self.eval(r, env)?;
        match op {
            BinOp::Eq => Ok(V::Bool(equal(&a, &b)?)),
            BinOp::Ne => Ok(V::Bool(!equal(&a, &b)?)),
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let ord = match (&a, &b) {
                    (V::Null, _) | (_, V::Null) => return Ok(V::Bool(false)),
                    (V::Num(x), V::Num(y)) => x.partial_cmp(y),
                    (V::Str(x), V::Str(y)) => Some(x.cmp(y)),
                    _ => return Err(format!("cannot order {} and {}", a.kind(), b.kind())),
                };
                let Some(ord) = ord else {
                    return Ok(V::Bool(false));
                };
                use std::cmp::Ordering::{Greater, Less};
                Ok(V::Bool(match op {
                    BinOp::Lt => ord == Less,
                    BinOp::Le => ord != Greater,
                    BinOp::Gt => ord == Greater,
                    _ => ord != Less,
                }))
            }
            BinOp::Add | BinOp::Concat
                if op == BinOp::Concat || matches!(a, V::Str(_)) || matches!(b, V::Str(_)) =>
            {
                Ok(V::Str(format!("{}{}", to_text(&a)?, to_text(&b)?)))
            }
            _ => match (&a, &b) {
                (V::Num(x), V::Num(y)) => {
                    if matches!(op, BinOp::Div | BinOp::Mod) && *y == 0.0 {
                        return Err("division by zero".into());
                    }
                    Ok(V::Num(match op {
                        BinOp::Add => x + y,
                        BinOp::Sub => x - y,
                        BinOp::Mul => x * y,
                        BinOp::Div => x / y,
                        _ => x % y,
                    }))
                }
                _ => Err(format!("arithmetic on {} and {}", a.kind(), b.kind())),
            },
        }
    }

    /// The elements of a collection value, `None` for other values.
    fn items(&mut self, v: &V) -> Option<Vec<V>> {
        match v {
            V::List(l) => Some(l.clone()),
            V::Refs(o, true) => Some(self.view.direct(*o, true).into_iter().map(V::Obj).collect()),
            V::Refs(o, false) => Some(
                self.depends_on(*o)
                    .iter()
                    .map(|(k, refs)| V::Dep(*k, refs.clone()))
                    .collect(),
            ),
            V::Translations(o, key) => Some(
                cultures(self.view.m)
                    .iter()
                    .map(|c| V::Str(self.translation(*o, c, key)))
                    .collect(),
            ),
            _ => None,
        }
    }

    fn index(&mut self, b: &V, i: &V) -> R<V> {
        if let V::InPerspective(o) = b {
            let V::Str(p) = i else {
                return Err("InPerspective is indexed by a perspective name".into());
            };
            return self.in_perspective(*o, p);
        }
        if let V::Translations(o, key) = b {
            let all = cultures(self.view.m);
            let culture = match i {
                V::Obj(Obj::Culture(ci)) => all.get(*ci),
                V::Str(name) => all.iter().find(|c| {
                    c.get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|n| n.eq_ignore_ascii_case(name))
                }),
                other => {
                    return Err(format!(
                        "translations are indexed by a culture, not {}",
                        other.kind()
                    ));
                }
            };
            return match culture {
                Some(c) => Ok(V::Str(self.translation(*o, c, key))),
                None => Err(format!(
                    "the model has no culture {}",
                    to_text(i).unwrap_or_default()
                )),
            };
        }
        if let V::Str(s) = b {
            // `Name[0]`: one character (a Unicode scalar value), as a one-character string.
            let V::Num(n) = i else {
                return Err(format!("cannot index a string with {}", i.kind()));
            };
            if *n < 0.0 || n.fract() != 0.0 {
                return Err(format!("string index {n} is not a valid position"));
            }
            let k = *n as usize;
            return s.chars().nth(k).map(|c| V::Str(c.to_string())).ok_or_else(|| {
                format!(
                    "index {k} is outside the string `{s}` ({} characters)",
                    s.chars().count()
                )
            });
        }
        let Some(items) = self.items(b) else {
            return Err(format!("cannot index {}", b.kind()));
        };
        match i {
            V::Num(n) if *n >= 0.0 && n.fract() == 0.0 => {
                let k = *n as usize;
                items
                    .get(k)
                    .cloned()
                    .ok_or_else(|| format!("index {k} is out of range"))
            }
            V::Str(s) => items
                .into_iter()
                .find(|x| matches!(x, V::Obj(o) if self.view.name(*o).eq_ignore_ascii_case(s)))
                .ok_or_else(|| format!("no object named {s} in the collection")),
            _ => Err(format!("cannot index a collection with {}", i.kind())),
        }
    }

    fn in_perspective(&self, o: Obj, perspective: &str) -> R<V> {
        let view = self.view;
        let Some(p) = perspectives(view.m).iter().find(|p| {
            p.get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| n.eq_ignore_ascii_case(perspective))
        }) else {
            return Err(format!("no perspective named {perspective}"));
        };
        let table_name = view
            .table_of(o)
            .and_then(|ti| view.table(ti))
            .map(|t| t.name.as_str())
            .unwrap_or_default();
        let named = |v: &Value, n: &str| {
            v.get("name")
                .and_then(Value::as_str)
                .is_some_and(|x| x.eq_ignore_ascii_case(n))
        };
        let pt = p
            .get("tables")
            .and_then(Value::as_array)
            .and_then(|ts| ts.iter().find(|t| named(t, table_name)));
        let Some(pt) = pt else {
            return Ok(V::Bool(false));
        };
        let member = |key: &str| {
            json_bool(Some(pt), "includeAll", false)
                || pt
                    .get(key)
                    .and_then(Value::as_array)
                    .is_some_and(|l| l.iter().any(|x| named(x, &view.name(o))))
        };
        Ok(V::Bool(match o {
            Obj::Table(_) => true,
            Obj::Column(..) => member("columns"),
            Obj::Measure(..) => member("measures"),
            Obj::Hierarchy(..) => member("hierarchies"),
            _ => return Err(format!("InPerspective is not available on {}", view.type_name(o))),
        }))
    }

    /// `base.name`.
    fn member(&mut self, base: &V, name: &str) -> R<V> {
        let lower = name.to_ascii_lowercase();
        match base {
            V::Obj(o) => self.prop(*o, &lower, name),
            V::Str(s) if lower == "length" => Ok(V::Num(s.encode_utf16().count() as f64)),
            V::List(_) | V::Refs(..) | V::Translations(..) if lower == "count" => {
                Ok(V::Num(self.items(base).map_or(0, |l| l.len()) as f64))
            }
            V::Refs(o, by) => {
                let (deep, kind) = match lower.as_str() {
                    "measures" => (false, "m"),
                    "columns" => (false, "c"),
                    "tables" => (false, "t"),
                    "allmeasures" => (true, "m"),
                    "allcolumns" => (true, "c"),
                    "alltables" => (true, "t"),
                    "deep" | "all" => (true, ""),
                    _ => return Err(format!("{name} is not a property of a dependency list")),
                };
                let objs = if deep {
                    self.view.transitive(*o, *by)
                } else {
                    self.view.direct(*o, *by)
                };
                Ok(V::List(
                    objs.into_iter()
                        .filter(|x| match kind {
                            "m" => matches!(x, Obj::Measure(..)),
                            "c" => matches!(x, Obj::Column(..)),
                            "t" => matches!(x, Obj::Table(..)),
                            _ => true,
                        })
                        .map(V::Obj)
                        .collect(),
                ))
            }
            V::Dep(k, refs) => match lower.as_str() {
                "key" => Ok(V::Obj(*k)),
                "value" => Ok(V::List(refs.iter().map(|b| V::Reference(*b)).collect())),
                _ => Err(format!("{name} is not a property of a dependency entry")),
            },
            V::Reference(fq) => match lower.as_str() {
                "fullyqualified" => Ok(V::Bool(*fq)),
                _ => Err(format!("{name} is not a property of a reference")),
            },
            V::Token(list, i) => {
                let at = |k: Option<usize>| {
                    k.filter(|k| *k < list.len())
                        .map_or(V::Null, |k| V::Token(list.clone(), k))
                };
                let t = list.get(*i).ok_or("the token is outside its list")?;
                match lower.as_str() {
                    "type" => Ok(V::Enum(t.ty.into())),
                    "text" => Ok(V::Str(t.text.clone())),
                    "next" => Ok(at(i.checked_add(1))),
                    "previous" => Ok(at(i.checked_sub(1))),
                    "index" => Ok(V::Num(*i as f64)),
                    _ => Err(format!("{name} is not a property of a DAX token")),
                }
            }
            V::Null => Err(format!("{name} read on a null value")),
            other => Err(format!("{name} is not a property of {}", other.kind())),
        }
    }

    /// The DAX expressions of an object, with their home table.
    fn dax_texts(&self, o: Obj) -> Vec<(String, Option<String>)> {
        let view = self.view;
        let home = view
            .table_of(o)
            .and_then(|ti| view.table(ti))
            .map(|t| t.name.clone());
        let mut out = Vec::new();
        match o {
            Obj::Measure(ti, mi) => {
                if let Some(x) = view.table(ti).and_then(|t| t.measures.get(mi)) {
                    out.push(x.expression.clone());
                }
            }
            Obj::Column(ti, ci) => {
                if let Some(e) = view.column(ti, ci).and_then(|(_, c)| c.expression.clone()) {
                    out.push(e);
                }
            }
            Obj::Table(ti) => {
                if let Some(e) = view
                    .table(ti)
                    .and_then(bicraft_dax::deps::calculated_table_expression)
                {
                    out.push(e.to_string());
                }
            }
            Obj::CalculationItem(..) => {
                out.extend(json_string(view.json(o).and_then(|v| v.get("expression"))))
            }
            Obj::Kpi(..) => {
                for k in ["targetExpression", "statusExpression", "trendExpression"] {
                    out.extend(json_string(view.json(o).and_then(|v| v.get(k))));
                }
            }
            Obj::TablePermission(ri, pi) => {
                if let Some(f) = view
                    .m
                    .roles
                    .get(ri)
                    .and_then(|r| r.table_permissions.get(pi))
                    .and_then(|p| p.filter_expression.clone())
                {
                    out.push(f);
                }
            }
            _ => {}
        }
        out.into_iter().map(|t| (t, home.clone())).collect()
    }

    /// `DependsOn` of an object: each referenced table, column or measure with one entry per
    /// reference (`true` when it names the table).
    fn depends_on(&mut self, o: Obj) -> DepList {
        if let Some(d) = self.deps.get(&o) {
            return d.clone();
        }
        let view = self.view;
        let mut out: Vec<(Obj, Vec<bool>)> = Vec::new();
        let mut add = |k: Obj, fq: bool| match out.iter_mut().find(|(x, _)| *x == k) {
            Some((_, refs)) => refs.push(fq),
            None => out.push((k, vec![fq])),
        };
        for (text, home) in self.dax_texts(o) {
            let (Some(tree), _) = bicraft_dax::parse_expression(&text) else {
                continue;
            };
            let nodes = walk(&tree);
            let vars: Vec<String> = nodes
                .iter()
                .filter_map(|x| match x {
                    Expr::Var { bindings, .. } => {
                        Some(bindings.iter().map(|b| b.name.to_lowercase()).collect::<Vec<_>>())
                    }
                    _ => None,
                })
                .flatten()
                .collect();
            for x in nodes {
                match x {
                    Expr::Column {
                        table: Some(t), name, ..
                    } => {
                        if let Some(c) = view.column_obj(t, name) {
                            add(c, true);
                        } else if let Some(m) = view.measure_obj(name) {
                            add(m, true);
                        }
                    }
                    Expr::Column {
                        table: None, name, ..
                    } => {
                        if let Some(m) = view.measure_obj(name) {
                            add(m, false);
                        } else if let Some(c) = home.as_deref().and_then(|h| view.column_obj(h, name)) {
                            add(c, false);
                        }
                    }
                    Expr::Name { name, .. } if !vars.contains(&name.to_lowercase()) => {
                        if let Some(ti) = view.table_index(name) {
                            add(Obj::Table(ti), true);
                        }
                    }
                    _ => {}
                }
            }
        }
        let rc = Rc::new(out);
        self.deps.insert(o, rc.clone());
        rc
    }

    fn annotation(&self, o: Obj, name: &str) -> Option<String> {
        let view = self.view;
        let m = view.m;
        match o {
            Obj::Model | Obj::Database => find_annotation(&m.annotations, name),
            Obj::Table(ti) => find_annotation(&view.table(ti)?.annotations, name),
            Obj::Column(ti, ci) => find_annotation(&view.column(ti, ci)?.1.annotations, name),
            Obj::Measure(ti, mi) => find_annotation(&view.table(ti)?.measures.get(mi)?.annotations, name),
            Obj::Relationship(ri) => find_annotation(&m.relationships.get(ri)?.annotations, name),
            Obj::Expression(ei) => find_annotation(&m.expressions.get(ei)?.annotations, name),
            Obj::Role(ri) => find_annotation(&m.roles.get(ri)?.annotations, name),
            Obj::Partition(ti, pi) => {
                let p = view.table(ti)?.partitions.get(pi)?;
                json_annotation(Some(&Value::Object(p.extra.clone())), name)
            }
            Obj::TablePermission(ri, pi) => {
                let p = m.roles.get(ri)?.table_permissions.get(pi)?;
                json_annotation(Some(&Value::Object(p.extra.clone())), name)
            }
            _ => json_annotation(view.json(o), name),
        }
    }

    fn objs(list: impl IntoIterator<Item = Obj>) -> V {
        V::List(list.into_iter().map(V::Obj).collect())
    }

    /// Non-empty row filters of the roles on table `ti` (`RowLevelSecurity` of a table).
    fn table_rls(&self, ti: usize) -> V {
        let Some(t) = self.view.table(ti) else {
            return V::List(Vec::new());
        };
        V::List(
            self.view
                .m
                .roles
                .iter()
                .flat_map(|r| r.table_permissions.iter())
                .filter(|p| p.table.eq_ignore_ascii_case(&t.name))
                .filter_map(|p| p.filter_expression.clone())
                .filter(|f| !f.trim().is_empty())
                .map(V::Str)
                .collect(),
        )
    }

    /// `ObjectLevelSecurity` of a table or column: its metadata permission in every role.
    fn ols(&self, ti: usize, column: Option<&str>) -> V {
        let Some(t) = self.view.table(ti) else {
            return V::List(Vec::new());
        };
        let mut out = Vec::new();
        for r in &self.view.m.roles {
            let p = r
                .table_permissions
                .iter()
                .find(|p| p.table.eq_ignore_ascii_case(&t.name));
            let perm = p.and_then(|p| match column {
                None => p.extra.get("metadataPermission").and_then(Value::as_str),
                Some(c) => p
                    .extra
                    .get("columnPermissions")
                    .and_then(Value::as_array)
                    .and_then(|l| {
                        l.iter().find(|x| {
                            x.get("name")
                                .and_then(Value::as_str)
                                .is_some_and(|n| n.eq_ignore_ascii_case(c))
                        })
                    })
                    .and_then(|x| x.get("metadataPermission"))
                    .and_then(Value::as_str),
            });
            out.push(perm.map_or(V::Enum("Default".into()), enum_v));
        }
        V::List(out)
    }

    /// Property `lower` (lowercase; `name` as written) of object `o`.
    fn prop(&mut self, o: Obj, lower: &str, name: &str) -> R<V> {
        let view = self.view;
        let m = view.m;
        // Properties every object has.
        match lower {
            "name" => return Ok(V::Str(view.name(o))),
            "objecttypename" => return Ok(V::Str(view.type_name(o).into())),
            "objecttype" => {
                return Ok(V::Enum(
                    match o {
                        Obj::Model => "Model",
                        Obj::Database => "Database",
                        Obj::Table(_) => "Table",
                        Obj::Column(..) => "Column",
                        Obj::Measure(..) => "Measure",
                        Obj::Kpi(..) => "KPI",
                        Obj::Relationship(_) => "Relationship",
                        Obj::Partition(..) => "Partition",
                        Obj::Expression(_) => "Expression",
                        Obj::Role(_) => "Role",
                        Obj::TablePermission(..) => "TablePermission",
                        Obj::Hierarchy(..) => "Hierarchy",
                        Obj::Level(..) => "Level",
                        Obj::CalculationItem(..) => "CalculationItem",
                        Obj::DataSource(_) => "DataSource",
                        Obj::Perspective(_) => "Perspective",
                        Obj::Culture(_) => "Culture",
                    }
                    .into(),
                ));
            }
            "model" => return Ok(V::Obj(Obj::Model)),
            "referencedby" => return Ok(V::Refs(o, true)),
            "dependson" => return Ok(V::Refs(o, false)),
            "daxobjectname" => {
                return Ok(V::Str(match o {
                    Obj::Table(_) => quote_table(&view.name(o)),
                    Obj::Column(..) | Obj::Measure(..) => bracket(&view.name(o)),
                    _ => view.name(o),
                }));
            }
            "daxobjectfullname" => {
                let t = view
                    .table_of(o)
                    .and_then(|ti| view.table(ti))
                    .map(|t| quote_table(&t.name));
                return Ok(V::Str(match (o, t) {
                    (Obj::Column(..) | Obj::Measure(..), Some(t)) => format!("{t}{}", bracket(&view.name(o))),
                    (Obj::Table(_), Some(t)) => t,
                    _ => view.name(o),
                }));
            }
            "daxtablename" => {
                return view
                    .table_of(o)
                    .and_then(|ti| view.table(ti))
                    .map(|t| V::Str(quote_table(&t.name)))
                    .ok_or_else(|| format!("{name} is not a property of {}", view.type_name(o)));
            }
            "inperspective"
                if matches!(
                    o,
                    Obj::Table(_) | Obj::Column(..) | Obj::Measure(..) | Obj::Hierarchy(..)
                ) =>
            {
                return Ok(V::InPerspective(o));
            }
            "translatednames" | "translateddescriptions" | "translateddisplayfolders" => {
                let key = match lower {
                    "translatednames" => "translatedCaption",
                    "translateddescriptions" => "translatedDescription",
                    _ => "translatedDisplayFolder",
                };
                let ok = match o {
                    Obj::Column(..) | Obj::Measure(..) | Obj::Hierarchy(..) => true,
                    _ if key == "translatedDisplayFolder" => false,
                    Obj::Model
                    | Obj::Table(_)
                    | Obj::Level(..)
                    | Obj::Perspective(_)
                    | Obj::Role(_)
                    | Obj::Expression(_) => true,
                    _ => false,
                };
                return if ok {
                    Ok(V::Translations(o, key))
                } else {
                    Err(format!("{name} is not a property of {}", view.type_name(o)))
                };
            }
            "table" => {
                if let Some(ti) = view.table_of(o)
                    && !matches!(o, Obj::Table(_))
                {
                    return Ok(V::Obj(Obj::Table(ti)));
                }
            }
            _ => {}
        }
        let unknown = || format!("{name} is not a property bicraft knows on {}", view.type_name(o));
        match o {
            Obj::Model => match lower {
                "description" => Ok(text(m.description.as_ref())),
                "culture" => Ok(text(m.culture.as_ref())),
                "database" => Ok(V::Obj(Obj::Database)),
                "compatibilitylevel" => Ok(m.compatibility_level.map_or(V::Null, |c| V::Num(f64::from(c)))),
                "tables" | "alltables" => Ok(Self::objs((0..m.tables.len()).map(Obj::Table))),
                "allcolumns" => Ok(Self::objs(
                    view.objects(Scope::DataColumn)
                        .into_iter()
                        .chain(view.objects(Scope::CalculatedColumn))
                        .chain(view.objects(Scope::CalculatedTableColumn)),
                )),
                "allmeasures" => Ok(Self::objs(view.objects(Scope::Measure))),
                "allhierarchies" => Ok(Self::objs(view.objects(Scope::Hierarchy))),
                "alllevels" => Ok(Self::objs(view.objects(Scope::Level))),
                "allpartitions" => Ok(Self::objs(view.objects(Scope::Partition))),
                "allcalculationitems" => Ok(Self::objs(view.objects(Scope::CalculationItem))),
                "relationships" => Ok(Self::objs((0..m.relationships.len()).map(Obj::Relationship))),
                "roles" => Ok(Self::objs((0..m.roles.len()).map(Obj::Role))),
                "expressions" => Ok(Self::objs((0..m.expressions.len()).map(Obj::Expression))),
                "datasources" => Ok(Self::objs((0..data_sources(m).len()).map(Obj::DataSource))),
                "perspectives" => Ok(Self::objs((0..perspectives(m).len()).map(Obj::Perspective))),
                "cultures" => Ok(Self::objs((0..cultures(m).len()).map(Obj::Culture))),
                "defaultmode" => match m.extra.get("defaultMode").and_then(Value::as_str) {
                    Some(s) => Ok(enum_v(s)),
                    None => Ok(V::Enum("Import".into())),
                },
                _ => fallback(&m.extra, None, name),
            },
            Obj::Database => match lower {
                "compatibilitylevel" => Ok(m.compatibility_level.map_or(V::Null, |c| V::Num(f64::from(c)))),
                _ => fallback(&m.database_extra, None, name),
            },
            Obj::Table(ti) => {
                let t = view.table(ti).ok_or_else(unknown)?;
                match lower {
                    "description" => Ok(text(t.description.as_ref())),
                    "ishidden" => Ok(V::Bool(t.is_hidden)),
                    "isvisible" => Ok(V::Bool(!t.is_hidden)),
                    "lineagetag" => Ok(text(t.lineage_tag.as_ref())),
                    "columns" => Ok(Self::objs(
                        t.columns
                            .iter()
                            .enumerate()
                            .filter(|(_, c)| !c.is_row_number())
                            .map(|(ci, _)| Obj::Column(ti, ci)),
                    )),
                    "measures" => Ok(Self::objs((0..t.measures.len()).map(|mi| Obj::Measure(ti, mi)))),
                    "partitions" => Ok(Self::objs(
                        (0..t.partitions.len()).map(|pi| Obj::Partition(ti, pi)),
                    )),
                    "hierarchies" => Ok(Self::objs(
                        (0..hierarchies(t).len()).map(|hi| Obj::Hierarchy(ti, hi)),
                    )),
                    "calculationitems" => Ok(Self::objs(
                        (0..calculation_items(t).len()).map(|i| Obj::CalculationItem(ti, i)),
                    )),
                    "expression" => Ok(V::Str(
                        bicraft_dax::deps::calculated_table_expression(t)
                            .unwrap_or_default()
                            .to_string(),
                    )),
                    "iscalculated" | "iscalculatedtable" => Ok(V::Bool(is_calculated_table(t))),
                    "rowlevelsecurity" => Ok(self.table_rls(ti)),
                    "objectlevelsecurity" => Ok(self.ols(ti, None)),
                    "usedinrelationships" => Ok(Self::objs(m.relationships.iter().enumerate().filter_map(
                        |(ri, r)| {
                            (r.from_table.eq_ignore_ascii_case(&t.name)
                                || r.to_table.eq_ignore_ascii_case(&t.name))
                            .then_some(Obj::Relationship(ri))
                        },
                    ))),
                    _ => fallback(&t.extra, serde_json::to_value(t).ok(), name),
                }
            }
            Obj::Column(ti, ci) => {
                let (t, c) = view.column(ti, ci).ok_or_else(unknown)?;
                let names_this = |v: Option<&Value>| {
                    v.and_then(Value::as_str)
                        .is_some_and(|x| x.eq_ignore_ascii_case(&c.name))
                };
                match lower {
                    "description" => Ok(text(c.description.as_ref())),
                    "ishidden" => Ok(V::Bool(c.is_hidden)),
                    "isvisible" => Ok(V::Bool(!c.is_hidden)),
                    "datatype" => Ok(enum_v(c.data_type.as_tmsl().unwrap_or("unknown"))),
                    "expression" => Ok(text(c.expression.as_ref())),
                    "sourcecolumn" => Ok(text(c.source_column.as_ref())),
                    "formatstring" => Ok(text(c.format_string.as_ref())),
                    "displayfolder" => Ok(text(c.display_folder.as_ref())),
                    "datacategory" => Ok(text(c.data_category.as_ref())),
                    "lineagetag" => Ok(text(c.lineage_tag.as_ref())),
                    "summarizeby" => Ok(c
                        .summarize_by
                        .as_deref()
                        .map_or(V::Enum("Default".into()), enum_v)),
                    "sortbycolumn" => Ok(c
                        .sort_by_column
                        .as_deref()
                        .and_then(|s| view.column_obj(&t.name, s))
                        .map_or(V::Null, V::Obj)),
                    "type" => Ok(V::Enum(
                        match column_kind(t, c) {
                            ColumnKind::Data => "Data",
                            ColumnKind::Calculated => "Calculated",
                            ColumnKind::CalculatedTableColumn => "CalculatedTableColumn",
                            ColumnKind::RowNumber => "RowNumber",
                        }
                        .into(),
                    )),
                    "usedinrelationships" => {
                        let is = |tb: &str, cl: &str| {
                            tb.eq_ignore_ascii_case(&t.name) && cl.eq_ignore_ascii_case(&c.name)
                        };
                        Ok(Self::objs(m.relationships.iter().enumerate().filter_map(
                            |(ri, r)| {
                                (is(&r.from_table, &r.from_column) || is(&r.to_table, &r.to_column))
                                    .then_some(Obj::Relationship(ri))
                            },
                        )))
                    }
                    "usedinsortby" => Ok(Self::objs(t.columns.iter().enumerate().filter_map(
                        |(k, other)| {
                            other
                                .sort_by_column
                                .as_deref()
                                .is_some_and(|s| s.eq_ignore_ascii_case(&c.name))
                                .then_some(Obj::Column(ti, k))
                        },
                    ))),
                    "usedinhierarchies" => Ok(Self::objs(hierarchies(t).iter().enumerate().filter_map(
                        |(hi, h)| {
                            levels(h)
                                .iter()
                                .any(|l| names_this(l.get("column")))
                                .then_some(Obj::Hierarchy(ti, hi))
                        },
                    ))),
                    "usedinlevels" => {
                        let mut out = Vec::new();
                        for (hi, h) in hierarchies(t).iter().enumerate() {
                            for (li, l) in levels(h).iter().enumerate() {
                                if names_this(l.get("column")) {
                                    out.push(Obj::Level(ti, hi, li));
                                }
                            }
                        }
                        Ok(Self::objs(out))
                    }
                    "usedinvariations" => {
                        // Variations (anywhere in the model) whose default column is this one.
                        let mut out = Vec::new();
                        for other in &m.tables {
                            for col in &other.columns {
                                for v in col.variations() {
                                    if let Some((vt, vc)) = &v.default_column
                                        && vt.eq_ignore_ascii_case(&t.name)
                                        && vc.eq_ignore_ascii_case(&c.name)
                                    {
                                        out.push(V::Str(v.name.clone()));
                                    }
                                }
                            }
                        }
                        Ok(V::List(out))
                    }
                    "objectlevelsecurity" => Ok(self.ols(ti, Some(&c.name))),
                    _ => fallback(&c.extra, serde_json::to_value(c).ok(), name),
                }
            }
            Obj::Measure(ti, mi) => {
                let x = view
                    .table(ti)
                    .and_then(|t| t.measures.get(mi))
                    .ok_or_else(unknown)?;
                let extra = Value::Object(x.extra.clone());
                match lower {
                    "description" => Ok(text(x.description.as_ref())),
                    "ishidden" => Ok(V::Bool(x.is_hidden)),
                    "isvisible" => Ok(V::Bool(!x.is_hidden)),
                    "expression" => Ok(V::Str(x.expression.clone())),
                    "formatstring" => Ok(text(x.format_string.as_ref())),
                    "displayfolder" => Ok(text(x.display_folder.as_ref())),
                    "lineagetag" => Ok(text(x.lineage_tag.as_ref())),
                    "formatstringexpression" => Ok(nested_expr(Some(&extra), "formatStringDefinition")),
                    "detailrowsexpression" => Ok(nested_expr(Some(&extra), "detailRowsDefinition")),
                    "kpi" => Ok(if x.extra.get("kpi").is_some_and(Value::is_object) {
                        V::Obj(Obj::Kpi(ti, mi))
                    } else {
                        V::Null
                    }),
                    "datatype" => self.measure_type(o),
                    _ => fallback(&x.extra, serde_json::to_value(x).ok(), name),
                }
            }
            Obj::Kpi(ti, mi) => {
                let k = view.json(o);
                match lower {
                    "measure" => Ok(V::Obj(Obj::Measure(ti, mi))),
                    "description" => Ok(json_text(k, "description")),
                    "targetexpression" => Ok(json_text(k, "targetExpression")),
                    "statusexpression" => Ok(json_text(k, "statusExpression")),
                    "trendexpression" => Ok(json_text(k, "trendExpression")),
                    "targetformatstring" => Ok(json_text(k, "targetFormatString")),
                    _ => json_fallback(k, name),
                }
            }
            Obj::Relationship(ri) => {
                let r = m.relationships.get(ri).ok_or_else(unknown)?;
                let col = |t: &str, c: &str| {
                    view.column_obj(t, c)
                        .map(V::Obj)
                        .ok_or_else(|| format!("the relationship's column {t}[{c}] does not exist"))
                };
                let tab = |t: &str| {
                    view.table_index(t)
                        .map(|ti| V::Obj(Obj::Table(ti)))
                        .ok_or_else(|| format!("the relationship's table {t} does not exist"))
                };
                match lower {
                    "fromtable" => tab(&r.from_table),
                    "totable" => tab(&r.to_table),
                    "fromcolumn" => col(&r.from_table, &r.from_column),
                    "tocolumn" => col(&r.to_table, &r.to_column),
                    "isactive" => Ok(V::Bool(r.is_active)),
                    "fromcardinality" => Ok(enum_v(r.from_cardinality.as_deref().unwrap_or("many"))),
                    "tocardinality" => Ok(enum_v(r.to_cardinality.as_deref().unwrap_or("one"))),
                    "crossfilteringbehavior" => Ok(enum_v(
                        r.cross_filtering_behavior.as_deref().unwrap_or("oneDirection"),
                    )),
                    "type" => Ok(V::Enum("SingleColumn".into())),
                    _ => fallback(&r.extra, serde_json::to_value(r).ok(), name),
                }
            }
            Obj::Partition(ti, pi) => {
                let p = view
                    .table(ti)
                    .and_then(|t| t.partitions.get(pi))
                    .ok_or_else(unknown)?;
                match lower {
                    "sourcetype" => Ok(enum_v(&p.source_type)),
                    "mode" => Ok(p.mode.as_deref().map_or(V::Enum("Default".into()), enum_v)),
                    "expression" | "query" => Ok(text(p.expression.as_ref())),
                    "datasource" => Ok(p
                        .source_extra
                        .get("dataSource")
                        .and_then(Value::as_str)
                        .and_then(|n| view.data_source_obj(n))
                        .map_or(V::Null, V::Obj)),
                    _ => fallback(&p.extra, serde_json::to_value(p).ok(), name),
                }
            }
            Obj::Expression(ei) => {
                let e = m.expressions.get(ei).ok_or_else(unknown)?;
                match lower {
                    "expression" => Ok(V::Str(e.expression.clone())),
                    "kind" => Ok(enum_v(&e.kind)),
                    "description" => Ok(text(e.description.as_ref())),
                    "lineagetag" => Ok(text(e.lineage_tag.as_ref())),
                    _ => fallback(&e.extra, serde_json::to_value(e).ok(), name),
                }
            }
            Obj::Role(ri) => {
                let r = m.roles.get(ri).ok_or_else(unknown)?;
                match lower {
                    "modelpermission" => Ok(enum_v(r.model_permission.as_deref().unwrap_or("read"))),
                    "tablepermissions" => Ok(Self::objs(
                        (0..r.table_permissions.len()).map(|pi| Obj::TablePermission(ri, pi)),
                    )),
                    "rowlevelsecurity" => Ok(V::List(
                        r.table_permissions
                            .iter()
                            .filter_map(|p| p.filter_expression.clone())
                            .filter(|f| !f.trim().is_empty())
                            .map(V::Str)
                            .collect(),
                    )),
                    "members" => Ok(V::List(
                        r.extra
                            .get("members")
                            .and_then(Value::as_array)
                            .map(|l| {
                                l.iter()
                                    .map(|x| {
                                        V::Str(
                                            x.get("memberName")
                                                .and_then(Value::as_str)
                                                .unwrap_or_default()
                                                .to_string(),
                                        )
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                    )),
                    _ => fallback(&r.extra, serde_json::to_value(r).ok(), name),
                }
            }
            Obj::TablePermission(ri, pi) => {
                let p = m
                    .roles
                    .get(ri)
                    .and_then(|r| r.table_permissions.get(pi))
                    .ok_or_else(unknown)?;
                match lower {
                    "table" => view
                        .table_index(&p.table)
                        .map(|ti| V::Obj(Obj::Table(ti)))
                        .ok_or_else(|| format!("the permission's table {} does not exist", p.table)),
                    "role" => Ok(V::Obj(Obj::Role(ri))),
                    "filterexpression" | "expression" => Ok(text(p.filter_expression.as_ref())),
                    _ => fallback(&p.extra, serde_json::to_value(p).ok(), name),
                }
            }
            Obj::Hierarchy(ti, hi) => {
                let h = view.json(o);
                match lower {
                    "levels" => Ok(Self::objs(
                        (0..h.map_or(0, |h| levels(h).len())).map(|li| Obj::Level(ti, hi, li)),
                    )),
                    "ishidden" => Ok(V::Bool(json_bool(h, "isHidden", false))),
                    "isvisible" => Ok(V::Bool(!json_bool(h, "isHidden", false))),
                    "displayfolder" => Ok(json_text(h, "displayFolder")),
                    "description" => Ok(json_text(h, "description")),
                    _ => json_fallback(h, name),
                }
            }
            Obj::Level(ti, hi, li) => {
                let t = view.table(ti).ok_or_else(unknown)?;
                let l = view.json(o);
                match lower {
                    "hierarchy" => Ok(V::Obj(Obj::Hierarchy(ti, hi))),
                    "column" => l
                        .and_then(|l| l.get("column"))
                        .and_then(Value::as_str)
                        .and_then(|c| view.column_obj(&t.name, c))
                        .map(V::Obj)
                        .ok_or_else(|| "the level's column does not exist".into()),
                    "ordinal" => Ok(V::Num(
                        l.and_then(|l| l.get("ordinal"))
                            .and_then(Value::as_f64)
                            .unwrap_or(li as f64),
                    )),
                    "description" => Ok(json_text(l, "description")),
                    _ => json_fallback(l, name),
                }
            }
            Obj::CalculationItem(ti, _) => {
                let c = view.json(o);
                match lower {
                    "expression" => Ok(json_text(c, "expression")),
                    "ordinal" => Ok(V::Num(
                        c.and_then(|c| c.get("ordinal"))
                            .and_then(Value::as_f64)
                            .unwrap_or(-1.0),
                    )),
                    "formatstringexpression" => Ok(nested_expr(c, "formatStringDefinition")),
                    "calculationgroup" => Ok(V::Obj(Obj::Table(ti))),
                    "description" => Ok(json_text(c, "description")),
                    _ => json_fallback(c, name),
                }
            }
            Obj::DataSource(_) => {
                let d = view.json(o);
                match lower {
                    "type" => Ok(V::Enum(
                        if d.is_some_and(crate::objects::is_structured) {
                            "Structured"
                        } else {
                            "Provider"
                        }
                        .into(),
                    )),
                    "description" => Ok(json_text(d, "description")),
                    "usedbypartitions" => {
                        let me = view.name(o);
                        let mut out = Vec::new();
                        for (ti, t) in m.tables.iter().enumerate() {
                            for (pi, p) in t.partitions.iter().enumerate() {
                                if p.source_extra
                                    .get("dataSource")
                                    .and_then(Value::as_str)
                                    .is_some_and(|n| n.eq_ignore_ascii_case(&me))
                                {
                                    out.push(Obj::Partition(ti, pi));
                                }
                            }
                        }
                        Ok(Self::objs(out))
                    }
                    _ => json_fallback(d, name),
                }
            }
            Obj::Perspective(_) => {
                let p = view.json(o);
                match lower {
                    "description" => Ok(json_text(p, "description")),
                    _ => json_fallback(p, name),
                }
            }
            Obj::Culture(_) => json_fallback(view.json(o), name),
        }
    }

    /// The translation (TMSL key `key`) of object `o` in `culture` (a `cultures[]` entry); empty
    /// when the culture does not translate it.
    fn translation(&self, o: Obj, culture: &Value, key: &str) -> String {
        let view = self.view;
        let model = culture.get("translations").and_then(|t| t.get("model"));
        let find = |parent: Option<&Value>, list: &str, name: &str| -> Option<Value> {
            parent?
                .get(list)?
                .as_array()?
                .iter()
                .find(|x| {
                    x.get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|n| n.eq_ignore_ascii_case(name))
                })
                .cloned()
        };
        let table = |ti: usize| find(model, "tables", &view.name(Obj::Table(ti)));
        let entry: Option<Value> = match o {
            Obj::Model => model.cloned(),
            Obj::Table(ti) => table(ti),
            Obj::Column(ti, _) => find(table(ti).as_ref(), "columns", &view.name(o)),
            Obj::Measure(ti, _) => find(table(ti).as_ref(), "measures", &view.name(o)),
            Obj::Hierarchy(ti, _) => find(table(ti).as_ref(), "hierarchies", &view.name(o)),
            Obj::Level(ti, hi, _) => find(
                find(
                    table(ti).as_ref(),
                    "hierarchies",
                    &view.name(Obj::Hierarchy(ti, hi)),
                )
                .as_ref(),
                "levels",
                &view.name(o),
            ),
            Obj::Perspective(_) => find(model, "perspectives", &view.name(o)),
            Obj::Role(_) => find(model, "roles", &view.name(o)),
            Obj::Expression(_) => find(model, "expressions", &view.name(o)),
            _ => None,
        };
        json_string(entry.as_ref().and_then(|e| e.get(key))).unwrap_or_default()
    }

    /// `DataType` of a measure: the result type the DAX type checker infers, with TOM's names.
    fn measure_type(&mut self, o: Obj) -> R<V> {
        if let Some(v) = self.measure_types.get(&o) {
            return v.clone();
        }
        let view = self.view;
        let found = match o {
            Obj::Measure(ti, mi) => view.table(ti).and_then(|t| Some((t, t.measures.get(mi)?))),
            _ => None,
        };
        let r = match found {
            None => Err("the measure does not exist".to_string()),
            Some((_, x)) if x.expression.trim().is_empty() => Err("the measure has no expression".into()),
            Some((t, x)) => {
                let validator = self.validator.get_or_insert_with(|| Validator::new(view.m));
                let v = validator.validate_as(&x.expression, Some(&t.name), ExprKind::Measure);
                // A measure that is only `BLANK()` is an Int64 measure for the engine.
                let only_blank = matches!(
                    bicraft_dax::parse_expression(&x.expression).0,
                    Some(Expr::Call { ref name, ref args, .. }) if args.is_empty() && name.eq_ignore_ascii_case("BLANK")
                );
                let result = if only_blank {
                    Some(ValueType::Scalar(ScalarType::Integer))
                } else {
                    v.result_type
                };
                match result {
                    Some(ValueType::Scalar(s)) => Ok(V::Enum(
                        match s {
                            ScalarType::Integer => "Int64",
                            ScalarType::Decimal => "Double",
                            ScalarType::Currency => "Decimal",
                            ScalarType::Boolean => "Boolean",
                            ScalarType::String => "String",
                            ScalarType::DateTime => "DateTime",
                            ScalarType::Variant => "Variant",
                        }
                        .into(),
                    )),
                    _ => Err("Measure.DataType: the result type of the measure cannot be inferred".into()),
                }
            }
        };
        self.measure_types.insert(o, r.clone());
        r
    }

    /// `Tokenize()` of an object with DAX.
    fn tokenize(&mut self, o: Obj) -> R<V> {
        if !matches!(
            o,
            Obj::Measure(..)
                | Obj::Column(..)
                | Obj::Table(_)
                | Obj::CalculationItem(..)
                | Obj::Kpi(..)
                | Obj::TablePermission(..)
        ) {
            return Err(format!("Tokenize is not available on {}", self.view.type_name(o)));
        }
        let list = match self.tokens.get(&o) {
            Some(l) => l.clone(),
            None => {
                let mut all = Vec::new();
                for (text, home) in self.dax_texts(o) {
                    all.extend(tokens::tokenize(self.view, &text, home.as_deref())?);
                }
                let rc = Rc::new(all);
                self.tokens.insert(o, rc.clone());
                rc
            }
        };
        Ok(V::List(
            (0..list.len()).map(|i| V::Token(list.clone(), i)).collect(),
        ))
    }

    /// Applies `f` (a lambda or an expression over `it`) to `item`.
    fn apply(&mut self, f: &Ex, item: &V, env: &Env) -> R<V> {
        let mut inner = Env {
            it: item.clone(),
            outer: Some(env.it.clone()),
            current: env.current.clone(),
            params: env.params.clone(),
        };
        match f {
            Ex::Lambda(p, body) => {
                inner.params.push((p.clone(), item.clone()));
                self.eval(body, &inner)
            }
            _ => self.eval(f, &inner),
        }
    }

    fn predicate(&mut self, f: &Ex, item: &V, env: &Env) -> R<bool> {
        match self.apply(f, item, env)? {
            V::Bool(b) => Ok(b),
            v => Err(format!("the predicate returned {}, not a boolean", v.kind())),
        }
    }

    fn call(&mut self, target: Option<&Ex>, name: &str, args: &[Ex], env: &Env) -> R<V> {
        let lower = name.to_ascii_lowercase();
        if let Some(Ex::Name(t)) = target
            && !env.params.iter().any(|(p, _)| p == t)
            && let Some(v) = self.static_call(t, &lower, name, args, env)
        {
            return v;
        }
        if target.is_none() && lower == "char" {
            // `char(9)`: a conversion to a character.
            return match args {
                [a] => match self.eval(a, env)? {
                    V::Num(n) if n >= 0.0 && n.fract() == 0.0 && n <= f64::from(u32::MAX) => {
                        char::from_u32(n as u32)
                            .map(|c| V::Str(c.to_string()))
                            .ok_or_else(|| format!("char({n}) is not a character"))
                    }
                    V::Str(s) if s.chars().count() == 1 => Ok(V::Str(s)),
                    v => Err(format!("char() of {}", v.kind())),
                },
                _ => Err("char() takes one argument".into()),
            };
        }
        let recv = match target {
            Some(t) => self.eval(t, env)?,
            None => env.it.clone(),
        };
        if let Some(items) = self.items(&recv) {
            return self.collection_call(items, &lower, name, args, env);
        }
        // Checked before the arguments are evaluated: a predicate run on the wrong receiver
        // would report a misleading error (`it` / `outerIt` out of place).
        let collection_only = matches!(
            lower.as_str(),
            "any"
                | "all"
                | "where"
                | "select"
                | "count"
                | "first"
                | "firstordefault"
                | "last"
                | "lastordefault"
        ) || (lower == "contains" && !matches!(recv, V::Str(_)));
        if collection_only {
            return Err(format!("{name} needs a collection, not {}", recv.kind()));
        }
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(self.eval(a, env)?);
        }
        if lower == "tostring" && vals.is_empty() {
            return Ok(V::Str(to_text(&recv)?));
        }
        if lower == "equals"
            && let [v] = vals.as_slice()
        {
            return Ok(V::Bool(equal(&recv, v)?));
        }
        match &recv {
            V::Str(s) => string_method(s, &lower, name, &vals),
            V::Obj(o) => match (lower.as_str(), vals.as_slice()) {
                ("getannotation", [V::Str(n)]) => Ok(self.annotation(*o, n).map_or(V::Null, V::Str)),
                ("hasannotation", [V::Str(n)]) => Ok(V::Bool(self.annotation(*o, n).is_some())),
                ("tokenize", []) => self.tokenize(*o),
                _ => Err(format!(
                    "method {name} is not supported on {}",
                    self.view.type_name(*o)
                )),
            },
            V::Null => Err(format!("{name} called on a null value")),
            other => Err(format!("method {name} is not supported on {}", other.kind())),
        }
    }

    fn collection_call(&mut self, items: Vec<V>, lower: &str, name: &str, args: &[Ex], env: &Env) -> R<V> {
        if args.len() > 1 {
            return Err(format!("{name} with {} arguments is not supported", args.len()));
        }
        let f = args.first();
        match (lower, f) {
            ("any", None) => Ok(V::Bool(!items.is_empty())),
            ("count" | "longcount", None) => Ok(V::Num(items.len() as f64)),
            ("tolist" | "toarray" | "asenumerable", None) => Ok(V::List(items)),
            ("any", Some(f)) => {
                for x in &items {
                    if self.predicate(f, x, env)? {
                        return Ok(V::Bool(true));
                    }
                }
                Ok(V::Bool(false))
            }
            ("all", Some(f)) => {
                for x in &items {
                    if !self.predicate(f, x, env)? {
                        return Ok(V::Bool(false));
                    }
                }
                Ok(V::Bool(true))
            }
            ("count" | "longcount", Some(f)) => {
                let mut n = 0usize;
                for x in &items {
                    if self.predicate(f, x, env)? {
                        n += 1;
                    }
                }
                Ok(V::Num(n as f64))
            }
            ("where", Some(f)) => {
                let mut out = Vec::new();
                for x in items {
                    if self.predicate(f, &x, env)? {
                        out.push(x);
                    }
                }
                Ok(V::List(out))
            }
            ("select", Some(f)) => {
                let mut out = Vec::with_capacity(items.len());
                for x in &items {
                    out.push(self.apply(f, x, env)?);
                }
                Ok(V::List(out))
            }
            ("first" | "firstordefault" | "last" | "lastordefault", f) => {
                let ordered: Vec<&V> = if lower.starts_with("last") {
                    items.iter().rev().collect()
                } else {
                    items.iter().collect()
                };
                for x in ordered {
                    let ok = match f {
                        Some(f) => self.predicate(f, x, env)?,
                        None => true,
                    };
                    if ok {
                        return Ok(x.clone());
                    }
                }
                if lower.ends_with("ordefault") {
                    Ok(V::Null)
                } else {
                    Err(format!("{name} found no element"))
                }
            }
            ("sum" | "min" | "max" | "average", f) => {
                let mut nums = Vec::with_capacity(items.len());
                for x in &items {
                    let v = match f {
                        Some(f) => self.apply(f, x, env)?,
                        None => x.clone(),
                    };
                    match v {
                        V::Num(n) => nums.push(n),
                        other => return Err(format!("{name} over {}", other.kind())),
                    }
                }
                if nums.is_empty() {
                    return if lower == "sum" {
                        Ok(V::Num(0.0))
                    } else {
                        Err(format!("{name} of an empty collection"))
                    };
                }
                Ok(V::Num(match lower {
                    "sum" => nums.iter().sum(),
                    "min" => nums.iter().copied().fold(f64::INFINITY, f64::min),
                    "max" => nums.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                    _ => nums.iter().sum::<f64>() / nums.len() as f64,
                }))
            }
            ("contains", Some(f)) => {
                let v = self.eval(f, env)?;
                for x in &items {
                    if equal(x, &v)? {
                        return Ok(V::Bool(true));
                    }
                }
                Ok(V::Bool(false))
            }
            ("distinct", None) => {
                let mut out: Vec<V> = Vec::new();
                for x in items {
                    if !out.iter().any(|y| equal(&x, y).unwrap_or(false)) {
                        out.push(x);
                    }
                }
                Ok(V::List(out))
            }
            _ => Err(format!("collection method {name} is not supported")),
        }
    }

    /// `Type.Method(args)` for the static types rules use; `None` when `t` is not one.
    fn static_call(&mut self, t: &str, lower: &str, name: &str, args: &[Ex], env: &Env) -> Option<R<V>> {
        let ty = t.to_ascii_lowercase();
        if !matches!(ty.as_str(), "string" | "regex" | "math" | "convert" | "char") {
            return None;
        }
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            match self.eval(a, env) {
                Ok(v) => vals.push(v),
                Err(e) => return Some(Err(e)),
            }
        }
        Some(match (ty.as_str(), lower, vals.as_slice()) {
            ("string", "isnullorempty", [v]) => match v {
                V::Null => Ok(V::Bool(true)),
                V::Str(s) => Ok(V::Bool(s.is_empty())),
                v => Err(format!("string.IsNullOrEmpty on {}", v.kind())),
            },
            ("string", "isnullorwhitespace", [v]) => match v {
                V::Null => Ok(V::Bool(true)),
                V::Str(s) => Ok(V::Bool(s.chars().all(char::is_whitespace))),
                v => Err(format!("string.IsNullOrWhiteSpace on {}", v.kind())),
            },
            ("string", "concat", vs) => {
                let mut s = String::new();
                for v in vs {
                    let parts = match v {
                        V::List(l) => l.clone(),
                        other => vec![other.clone()],
                    };
                    for p in &parts {
                        match to_text(p) {
                            Ok(x) => s.push_str(&x),
                            Err(e) => return Some(Err(e)),
                        }
                    }
                }
                Ok(V::Str(s))
            }
            ("string", "equals", [a, b]) => equal(a, b).map(V::Bool),
            ("regex", "ismatch", [input, V::Str(p)]) => self.is_match(input, p, None),
            ("regex", "ismatch", [input, V::Str(p), opts]) => self.is_match(input, p, Some(opts)),
            ("math", "abs", [V::Num(n)]) => Ok(V::Num(n.abs())),
            ("math", "floor", [V::Num(n)]) => Ok(V::Num(n.floor())),
            ("math", "ceiling", [V::Num(n)]) => Ok(V::Num(n.ceil())),
            ("math", "round", [V::Num(n)]) => Ok(V::Num(n.round_ties_even())),
            ("math", "min", [V::Num(a), V::Num(b)]) => Ok(V::Num(a.min(*b))),
            ("math", "max", [V::Num(a), V::Num(b)]) => Ok(V::Num(a.max(*b))),
            ("convert", "tostring", [v]) => to_text(v).map(V::Str),
            ("convert", "toint16" | "toint32" | "toint64" | "todouble" | "todecimal" | "tosingle", [v]) => {
                let integer = lower.starts_with("toint");
                match v {
                    V::Num(n) => Ok(V::Num(if integer { n.round_ties_even() } else { *n })),
                    V::Bool(b) => Ok(V::Num(if *b { 1.0 } else { 0.0 })),
                    V::Str(s) => {
                        let parsed = s.trim().parse::<f64>().ok().filter(|n| n.is_finite());
                        match parsed {
                            Some(n) if !integer || n.fract() == 0.0 => Ok(V::Num(n)),
                            _ => Err(format!("Convert.{name}: `{s}` is not a number")),
                        }
                    }
                    // Convert.ToInt64((string)null) is 0.
                    V::Null => Ok(V::Num(0.0)),
                    v => Err(format!("Convert.{name} on {}", v.kind())),
                }
            }
            ("char", f, [V::Str(s)]) => {
                let Some(c) = s.chars().next().filter(|_| s.chars().count() == 1) else {
                    return Some(Err(format!("char.{name} needs one character")));
                };
                match f {
                    "iscontrol" => Ok(V::Bool(c.is_control())),
                    "iswhitespace" => Ok(V::Bool(c.is_whitespace())),
                    "isletter" => Ok(V::Bool(c.is_alphabetic())),
                    "isdigit" => Ok(V::Bool(c.is_numeric())),
                    "isletterordigit" => Ok(V::Bool(c.is_alphanumeric())),
                    "isupper" => Ok(V::Bool(c.is_uppercase())),
                    "islower" => Ok(V::Bool(c.is_lowercase())),
                    "ispunctuation" => Ok(V::Bool(c.is_ascii_punctuation())),
                    "toupper" | "toupperinvariant" => Ok(V::Str(c.to_uppercase().collect())),
                    "tolower" | "tolowerinvariant" => Ok(V::Str(c.to_lowercase().collect())),
                    _ => Err(format!("char.{name} is not supported")),
                }
            }
            _ => Err(format!("{t}.{name} with these arguments is not supported")),
        })
    }

    fn is_match(&mut self, input: &V, pattern: &str, opts: Option<&V>) -> R<V> {
        let V::Str(input) = input else {
            return Err(format!("RegEx.IsMatch on {}", input.kind()));
        };
        let mut ignore_case = false;
        if let Some(o) = opts {
            let (V::Enum(o) | V::Str(o)) = o else {
                return Err("RegEx.IsMatch options must be RegexOptions values".into());
            };
            for part in o.split(',').map(str::trim) {
                match part.to_ascii_lowercase().as_str() {
                    "ignorecase" => ignore_case = true,
                    "none" | "compiled" | "cultureinvariant" | "explicitcapture" => {}
                    other => return Err(format!("RegexOptions.{other} is not supported")),
                }
            }
        }
        let (p, inline) = inline_ignore_case(pattern)?;
        ignore_case |= inline;
        let key = (p.clone(), ignore_case);
        if self.regexes.len() >= MAX_CACHED_REGEXES && !self.regexes.contains_key(&key) {
            self.regexes.clear();
        }
        let re = self
            .regexes
            .entry(key)
            .or_insert_with(|| {
                let r = if ignore_case {
                    Regex::new_ignore_case(&p)
                } else {
                    Regex::new(&p)
                };
                r.map(Rc::new)
            })
            .clone();
        match re {
            Some(re) => Ok(V::Bool(re.is_match(input))),
            None => Err(format!(
                "the regular expression `{pattern}` uses syntax bicraft does not support"
            )),
        }
    }
}

/// Removes the inline `(?i)` options of a .NET pattern. Case-insensitivity then applies to the
/// whole pattern, which is only right when nothing before the first `(?i)` depends on case (a
/// letter outside an escape); such patterns are refused.
fn inline_ignore_case(pattern: &str) -> R<(String, bool)> {
    const FLAG: &str = "(?i)";
    let Some(first) = pattern.find(FLAG) else {
        return Ok((pattern.to_string(), false));
    };
    let before = pattern.get(..first).unwrap_or_default();
    let mut escaped = false;
    for c in before.chars() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c.is_alphabetic() {
            return Err(format!(
                "the regular expression `{pattern}` switches case sensitivity after a letter"
            ));
        }
    }
    Ok((pattern.replace(FLAG, ""), true))
}

fn to_text(v: &V) -> R<String> {
    Ok(match v {
        V::Null => String::new(),
        V::Bool(true) => "True".into(),
        V::Bool(false) => "False".into(),
        V::Num(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{}", *n as i64)
            } else {
                n.to_string()
            }
        }
        V::Str(s) | V::Enum(s) => s.clone(),
        V::Token(list, i) => list.get(*i).map(|t| t.text.clone()).unwrap_or_default(),
        other => return Err(format!("cannot turn {} into text", other.kind())),
    })
}

fn equal(a: &V, b: &V) -> R<bool> {
    Ok(match (a, b) {
        (V::Null, V::Null) => true,
        (V::Null, _) | (_, V::Null) => false,
        (V::Bool(x), V::Bool(y)) => x == y,
        (V::Num(x), V::Num(y)) => x == y,
        (V::Str(x), V::Str(y)) => x == y,
        // Token types: `COLUMN_OR_MEASURE` / `TABLE_OR_VARIABLE` also match their resolved forms.
        (V::Enum(x), V::Enum(y)) => tokens::same_type(x, y),
        (V::Enum(x), V::Str(y)) | (V::Str(x), V::Enum(y)) => x.eq_ignore_ascii_case(y),
        (V::Token(a, i), V::Token(b, j)) => Rc::ptr_eq(a, b) && i == j,
        (V::Obj(x), V::Obj(y)) => x == y,
        _ => return Err(format!("cannot compare {} with {}", a.kind(), b.kind())),
    })
}

/// Whether a `StringComparison` argument asks to ignore case.
fn ignore_case_arg(v: Option<&V>) -> R<bool> {
    match v {
        None => Ok(false),
        Some(V::Enum(e) | V::Str(e)) => Ok(e.to_ascii_lowercase().contains("ignorecase")),
        Some(V::Bool(b)) => Ok(*b),
        Some(other) => Err(format!("unexpected comparison argument: {}", other.kind())),
    }
}

/// Position (in UTF-16 units, as .NET counts) of byte offset `at` in `s`.
fn utf16_pos(s: &str, at: usize) -> f64 {
    s.get(..at).map_or(0, |p| p.encode_utf16().count()) as f64
}

fn string_method(s: &str, lower: &str, name: &str, vals: &[V]) -> R<V> {
    let arg_str = |i: usize| -> R<String> {
        match vals.get(i) {
            Some(V::Str(x) | V::Enum(x)) => Ok(x.clone()),
            Some(V::Null) => Err(format!("{name}: null argument")),
            Some(other) => Err(format!("{name}: argument is {}", other.kind())),
            None => Err(format!("{name}: missing argument")),
        }
    };
    let fold = |x: &str, ic: bool| if ic { x.to_lowercase() } else { x.to_string() };
    match lower {
        "contains" | "startswith" | "endswith" => {
            let needle = arg_str(0)?;
            let ic = ignore_case_arg(vals.get(1))?;
            let (h, n) = (fold(s, ic), fold(&needle, ic));
            Ok(V::Bool(match lower {
                "contains" => h.contains(&n),
                "startswith" => h.starts_with(&n),
                _ => h.ends_with(&n),
            }))
        }
        "toupper" | "toupperinvariant" => Ok(V::Str(s.to_uppercase())),
        "tolower" | "tolowerinvariant" => Ok(V::Str(s.to_lowercase())),
        "trim" | "trimstart" | "trimend" => {
            let chars: Vec<char> = match vals.first() {
                None => Vec::new(),
                Some(_) => arg_str(0)?.chars().collect(),
            };
            let pred = |c: char| {
                if chars.is_empty() {
                    c.is_whitespace()
                } else {
                    chars.contains(&c)
                }
            };
            Ok(V::Str(
                match lower {
                    "trim" => s.trim_matches(pred),
                    "trimstart" => s.trim_start_matches(pred),
                    _ => s.trim_end_matches(pred),
                }
                .to_string(),
            ))
        }
        "replace" => {
            let from = arg_str(0)?;
            if from.is_empty() {
                return Err("Replace with an empty search string".into());
            }
            let to = match vals.get(1) {
                Some(V::Null) => String::new(),
                _ => arg_str(1)?,
            };
            Ok(V::Str(s.replace(&from, &to)))
        }
        "indexof" | "lastindexof" => {
            let needle = arg_str(0)?;
            let ic = ignore_case_arg(vals.get(1))?;
            let (h, n) = if ic {
                // Simple case folding keeps one character per character for the scripts names
                // use; positions are counted on the folded text.
                (fold(s, true), fold(&needle, true))
            } else {
                (s.to_string(), needle)
            };
            let pos = if lower == "indexof" {
                h.find(&n)
            } else {
                h.rfind(&n)
            };
            Ok(V::Num(pos.map_or(-1.0, |p| utf16_pos(&h, p))))
        }
        "substring" => {
            let chars: Vec<char> = s.chars().collect();
            let num = |i: usize| match vals.get(i) {
                Some(V::Num(n)) if *n >= 0.0 && n.fract() == 0.0 => Ok(*n as usize),
                _ => Err(format!("{name}: bad index")),
            };
            let start = num(0)?;
            let len = if vals.len() > 1 {
                num(1)?
            } else {
                chars.len().saturating_sub(start)
            };
            let end = start.checked_add(len).ok_or("Substring: range overflow")?;
            chars
                .get(start..end)
                .map(|c| V::Str(c.iter().collect()))
                .ok_or_else(|| "Substring: the range is outside the string".into())
        }
        "split" => {
            let sep = arg_str(0)?;
            if sep.is_empty() {
                return Err("Split with an empty separator".into());
            }
            Ok(V::List(
                s.split(sep.as_str()).map(|x| V::Str(x.to_string())).collect(),
            ))
        }
        "tochararray" => Ok(V::List(s.chars().map(|c| V::Str(c.to_string())).collect())),
        _ => Err(format!("string method {name} is not supported")),
    }
}

/// Runs one rule and records its outcome and findings.
pub(crate) fn run_rule(interp: &mut Interp<'_, '_>, rule: &Rule, max: usize, out: &mut Analysis) {
    let view = interp.view;
    let mut outcome = RuleOutcome {
        rule_id: rule.id.clone(),
        rule_name: rule.name.clone(),
        category: rule.category.clone(),
        severity: severity_name(rule.severity),
        source: rule.source.clone(),
        status: "evaluated",
        objects: 0,
        findings: 0,
        reason: None,
        fix_expression: rule.fix_expression.clone(),
    };
    if let (Some(need), Some(have)) = (rule.compatibility_level, view.m.compatibility_level)
        && need > have
    {
        outcome.status = "skipped";
        outcome.reason = Some(format!(
            "the rule needs compatibility level {need}; the model is at {have}"
        ));
        out.rules.push(outcome);
        return;
    }
    let mut scopes = Vec::new();
    let mut unsupported = Vec::new();
    for s in rule.scope.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        match Scope::parse(s) {
            Some(sc) => {
                if !scopes.contains(&sc) {
                    scopes.push(sc);
                }
            }
            None => unsupported.push(s.to_string()),
        }
    }
    let unsupported_note =
        (!unsupported.is_empty()).then(|| format!("scope {} is not supported", unsupported.join(", ")));
    if scopes.is_empty() {
        outcome.status = "not_evaluated";
        outcome.reason = Some(unsupported_note.unwrap_or_else(|| "the rule has no scope".into()));
        out.rules.push(outcome);
        return;
    }
    let ex = match linq::parse(&rule.expression) {
        Ok(e) => e,
        Err(e) => {
            outcome.status = "not_evaluated";
            outcome.reason = Some(format!("the expression cannot be read: {e}"));
            out.rules.push(outcome);
            return;
        }
    };
    let mut errors = 0usize;
    let mut first_error: Option<String> = None;
    let mut total_steps = 0u64;
    let objects: Vec<Obj> = scopes.iter().flat_map(|s| view.objects(*s)).collect();
    for o in &objects {
        if total_steps > RULE_BUDGET {
            errors += 1;
            first_error.get_or_insert_with(|| "the rule needs more evaluation steps than allowed".into());
            continue;
        }
        let r = interp.eval_on(&ex, *o);
        total_steps = total_steps.saturating_add(interp.steps);
        match r {
            Ok(V::Bool(true)) => {
                outcome.objects += 1;
                outcome.findings += 1;
                out.push(
                    Finding {
                        rule_id: rule.id.clone(),
                        rule_name: rule.name.clone(),
                        category: rule.category.clone(),
                        severity: severity_name(rule.severity),
                        severity_level: rule.severity,
                        object_type: view.type_name(*o).into(),
                        object: view.display(*o),
                        table: view
                            .table_of(*o)
                            .and_then(|ti| view.table(ti))
                            .map(|t| t.name.clone()),
                    },
                    max,
                );
            }
            Ok(V::Bool(false)) => outcome.objects += 1,
            Ok(other) => {
                errors += 1;
                first_error.get_or_insert_with(|| {
                    format!("the expression returned {}, not true or false", other.kind())
                });
            }
            Err(e) => {
                errors += 1;
                first_error.get_or_insert_with(|| format!("{e} (on {})", view.display(*o)));
            }
        }
    }
    let mut reasons: Vec<String> = Vec::new();
    if errors > 0 {
        outcome.status = if errors == objects.len() {
            "not_evaluated"
        } else {
            "partial"
        };
        reasons.push(format!(
            "{errors} of {} objects not evaluated: {}",
            objects.len(),
            first_error.unwrap_or_default()
        ));
    }
    if let Some(n) = unsupported_note {
        if outcome.status == "evaluated" {
            outcome.status = "partial";
        }
        reasons.push(n);
    }
    if !reasons.is_empty() {
        outcome.reason = Some(reasons.join("; "));
    }
    out.rules.push(outcome);
}
