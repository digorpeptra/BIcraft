//! A small JSON Schema (draft-07) validator for the vendored PBIR schemas and the report theme
//! schema.
//!
//! A schema is compiled once into an arena of nodes: `$ref`s become node ids, patterns become
//! compiled [`Regex`]es, properties a sorted table, messages are prepared. The vendored PBIR
//! schemas are compiled on first use and cached for the life of the process, one entry per root
//! file (with everything it references).
//!
//! Keywords: `type`, `const`, `enum`, `$ref` (draft-07: siblings ignored), `definitions`/`$defs`
//! (through `$ref`), `allOf`, `anyOf`, `oneOf`, `not`, `if`/`then`/`else`, `properties`,
//! `patternProperties`, `additionalProperties`, `propertyNames`, `required`, `dependencies`,
//! `dependentRequired`, `dependentSchemas`, `minProperties`, `maxProperties`, `items` (schema or
//! tuple), `additionalItems`, `minItems`, `maxItems`, `uniqueItems`, `contains`, `minContains`,
//! `maxContains`, `minLength`, `maxLength`, `pattern`, `minimum`, `maximum`, `exclusiveMinimum`,
//! `exclusiveMaximum` (number or draft-04 boolean), `multipleOf`. `format` and the annotations are
//! ignored. A pattern the [`Regex`] subset cannot compile accepts every string.
//!
//! Validation is bounded: nesting depth, a work budget per document and a cap on issues; schema
//! compilation is bounded by a node cap. Coverage table: `docs/formats/pbir.md`.
//! Why not the `jsonschema` crate: see `docs/formats/pbir.md`.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{Map, Value};

use crate::json::{SCHEMA_PREFIX, json_eq, parse_version, type_name};
use crate::regex::Regex;
use crate::schema_files::FILES;

const MAX_DEPTH: u32 = 1500;
const BUDGET: u64 = 2_000_000;
/// Most nodes in one compiled schema (the report theme schema needs about 20,000).
const MAX_NODES: usize = 2_000_000;
pub(crate) const MAX_ISSUES: usize = 100;
/// Stack for validation threads: validation recurses with the schema and the instance.
const STACK: usize = 64 * 1024 * 1024;

/// Runs `f` on a thread with a 64 MiB stack, or inline when no thread can be spawned. `None` if
/// it panicked.
pub(crate) fn on_big_stack<T: Send>(f: impl Fn() -> T + Sync) -> Option<T> {
    std::thread::scope(|scope| {
        match std::thread::Builder::new()
            .name("pbir-validate".into())
            .stack_size(STACK)
            .spawn_scoped(scope, &f)
        {
            Ok(handle) => handle.join().ok(),
            Err(_) => Some(f()),
        }
    })
}

/// Resolves the vendored file for a schema `key` and version: the exact version, else the newest
/// vendored version with the same major. Returns the file key and whether it was substituted.
pub(crate) fn vendored(key: &str, version: (u64, u64, u64)) -> Option<(String, bool)> {
    let mut best: Option<((u64, u64, u64), &str)> = None;
    for (path, _) in FILES {
        let mut parts = path.split('/');
        let (Some(k), Some(v), Some("schema.json")) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        if k != key {
            continue;
        }
        let Some(v) = parse_version(v) else { continue };
        if v == version {
            return Some(((*path).to_string(), false));
        }
        if v.0 == version.0 && best.is_none_or(|(b, _)| v > b) {
            best = Some((v, path));
        }
    }
    best.map(|(_, p)| (p.to_string(), true))
}

/// The compiled vendored schema `key` (e.g. `page/2.1.0/schema.json`), cached per process.
/// `None` when it is not vendored or does not parse.
pub(crate) fn vendored_schema(key: &str) -> Option<Arc<Compiled>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<Compiled>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Mutex::default);
    let mut map = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = map.get(key) {
        return Some(Arc::clone(c));
    }
    let docs = load(key)?;
    let compiled = Arc::new(Compiled::new(&docs, key));
    map.insert(key.to_string(), Arc::clone(&compiled));
    Some(compiled)
}

/// A parsed vendored file, cached per process (several roots share the referenced files).
fn parsed(key: &str) -> Option<Arc<Value>> {
    static PARSED: OnceLock<Mutex<HashMap<String, Arc<Value>>>> = OnceLock::new();
    let cache = PARSED.get_or_init(Mutex::default);
    let mut map = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = map.get(key) {
        return Some(Arc::clone(v));
    }
    let (_, text) = FILES.iter().find(|(p, _)| *p == key)?;
    let value = Arc::new(serde_json::from_str::<Value>(text).ok()?);
    map.insert(key.to_string(), Arc::clone(&value));
    Some(value)
}

/// Parses `key` and, transitively, every vendored file it references. `None` if `key` is not
/// vendored or a file does not parse. Missing referenced files are left out (their references
/// report as unresolvable).
fn load(key: &str) -> Option<HashMap<String, Arc<Value>>> {
    let mut files = HashMap::new();
    let mut queue = vec![key.to_string()];
    while let Some(k) = queue.pop() {
        if files.contains_key(&k) {
            continue;
        }
        if !FILES.iter().any(|(p, _)| *p == k) {
            if k == key {
                return None;
            }
            continue;
        }
        let value = parsed(&k)?;
        let mut refs = Vec::new();
        collect_refs(&value, &mut refs, 0);
        for r in refs {
            if let Some((file, _)) = resolve(&k, &r)
                && !files.contains_key(&file)
            {
                queue.push(file);
            }
        }
        files.insert(k, value);
    }
    Some(files)
}

fn collect_refs(v: &Value, out: &mut Vec<String>, depth: u32) {
    if depth > MAX_DEPTH {
        return;
    }
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                if k == "$ref" {
                    if let Some(s) = x.as_str() {
                        out.push(s.to_string());
                    }
                } else {
                    collect_refs(x, out, depth + 1);
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect_refs(x, out, depth + 1)),
        _ => {}
    }
}

/// `base`-relative reference → (file key, JSON pointer fragment).
fn resolve(base: &str, reference: &str) -> Option<(String, String)> {
    let (file, fragment) = match reference.split_once('#') {
        Some((f, frag)) => (f, percent_decode(frag)),
        None => (reference, String::new()),
    };
    if file.is_empty() {
        return Some((base.to_string(), fragment));
    }
    if let Some(abs) = file.strip_prefix(SCHEMA_PREFIX) {
        return Some((abs.to_string(), fragment));
    }
    let mut parts: Vec<&str> = base.split('/').collect();
    parts.pop();
    for seg in file.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some((parts.join("/"), fragment))
}

/// URI fragments may percent-encode pointer characters (`%25` for `%`, `%22` …).
fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let hex = |j: usize| bytes.get(j).and_then(|c| (*c as char).to_digit(16));
        match (b, hex(i + 1), hex(i + 2)) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

fn escape_pointer(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

type NodeId = usize;

const T_OBJECT: u8 = 1;
const T_ARRAY: u8 = 2;
const T_STRING: u8 = 4;
const T_BOOLEAN: u8 = 8;
const T_NULL: u8 = 16;
const T_NUMBER: u8 = 32;
const T_INTEGER: u8 = 64;
const T_ANY: u8 = 127;

enum Node {
    True,
    False,
    /// Invalid for every instance, with this message (an unresolvable `$ref`, a schema too big).
    Error(String),
    Ref(NodeId),
    Schema(Box<Keywords>),
}

enum Items {
    None,
    All(NodeId),
    Tuple(Vec<NodeId>),
}

/// A `pattern`: `None` when the pattern is outside the supported subset (every string passes).
struct Pattern {
    source: String,
    regex: Option<Regex>,
}

impl Pattern {
    fn new(source: &str) -> Self {
        Self {
            source: source.to_string(),
            regex: Regex::new(source),
        }
    }

    /// `None` when unknown.
    fn matches(&self, s: &str) -> Option<bool> {
        self.regex.as_ref().map(|r| r.is_match(s))
    }
}

struct Keywords {
    types: u8,
    /// The `type` keyword as written (`a or b` for a list), for messages.
    type_label: Cow<'static, str>,
    constant: Option<Value>,
    enumeration: Option<Vec<Value>>,
    // strings
    min_length: Option<u64>,
    max_length: Option<u64>,
    pattern: Option<Pattern>,
    // numbers
    minimum: Option<f64>,
    maximum: Option<f64>,
    exclusive_minimum: Option<f64>,
    exclusive_maximum: Option<f64>,
    multiple_of: Option<f64>,
    // objects
    required: Vec<String>,
    /// Sorted by name.
    properties: Vec<(String, NodeId)>,
    pattern_properties: Vec<(Pattern, NodeId)>,
    additional_properties: Option<NodeId>,
    property_names: Option<NodeId>,
    min_properties: Option<u64>,
    max_properties: Option<u64>,
    dependent_required: Vec<(String, Vec<String>)>,
    dependent_schemas: Vec<(String, NodeId)>,
    // arrays
    items: Items,
    additional_items: Option<NodeId>,
    min_items: Option<u64>,
    max_items: Option<u64>,
    unique_items: bool,
    contains: Option<NodeId>,
    min_contains: Option<u64>,
    max_contains: Option<u64>,
    // combinators
    all_of: Vec<NodeId>,
    any_of: Vec<NodeId>,
    one_of: Vec<NodeId>,
    not: Option<NodeId>,
    if_then_else: Option<(NodeId, Option<NodeId>, Option<NodeId>)>,
}

impl Default for Keywords {
    fn default() -> Self {
        Self {
            types: T_ANY,
            type_label: Cow::Borrowed(""),
            constant: None,
            enumeration: None,
            min_length: None,
            max_length: None,
            pattern: None,
            minimum: None,
            maximum: None,
            exclusive_minimum: None,
            exclusive_maximum: None,
            multiple_of: None,
            required: Vec::new(),
            properties: Vec::new(),
            pattern_properties: Vec::new(),
            additional_properties: None,
            property_names: None,
            min_properties: None,
            max_properties: None,
            dependent_required: Vec::new(),
            dependent_schemas: Vec::new(),
            items: Items::None,
            additional_items: None,
            min_items: None,
            max_items: None,
            unique_items: false,
            contains: None,
            min_contains: None,
            max_contains: None,
            all_of: Vec::new(),
            any_of: Vec::new(),
            one_of: Vec::new(),
            not: None,
            if_then_else: None,
        }
    }
}

/// A compiled schema: the node arena and the root.
pub(crate) struct Compiled {
    nodes: Vec<Node>,
    root: NodeId,
}

struct Builder<'a> {
    docs: &'a HashMap<String, Arc<Value>>,
    nodes: Vec<Node>,
    /// `$ref` targets already interned, by `file#pointer`. Inline subschemas are not memoized:
    /// each position is compiled once from its parent.
    refs: HashMap<String, NodeId>,
    queue: Vec<(NodeId, &'a str, &'a Value)>,
    too_big: Option<NodeId>,
}

fn type_mask(name: &str) -> u8 {
    match name {
        "object" => T_OBJECT,
        "array" => T_ARRAY,
        "string" => T_STRING,
        "boolean" => T_BOOLEAN,
        "null" => T_NULL,
        "number" => T_NUMBER,
        "integer" => T_INTEGER,
        _ => T_ANY,
    }
}

impl<'a> Builder<'a> {
    /// A node for the schema `value` of `file`, compiled later from the queue.
    fn sub(&mut self, file: &'a str, value: &'a Value) -> NodeId {
        if self.nodes.len() >= MAX_NODES {
            if let Some(id) = self.too_big {
                return id;
            }
            let id = self.nodes.len();
            self.nodes
                .push(Node::Error("the schema is too large to validate with".into()));
            self.too_big = Some(id);
            return id;
        }
        let id = self.nodes.len();
        self.nodes.push(Node::True);
        self.queue.push((id, file, value));
        id
    }

    fn run(&mut self) {
        while let Some((id, file, value)) = self.queue.pop() {
            let node = self.build(file, value);
            if let Some(slot) = self.nodes.get_mut(id) {
                *slot = node;
            }
        }
    }

    fn reference(&mut self, file: &'a str, r: &str) -> Node {
        let docs = self.docs;
        let Some((f, frag)) = resolve(file, r) else {
            return Node::Error(format!("unresolvable schema reference `{r}`"));
        };
        let key = format!("{f}#{frag}");
        if let Some(id) = self.refs.get(&key) {
            return Node::Ref(*id);
        }
        let target = docs.get_key_value(&f).and_then(|(name, doc)| {
            let doc: &Value = doc;
            let v = if frag.is_empty() {
                Some(doc)
            } else {
                doc.pointer(&frag)
            };
            Some((name.as_str(), v?))
        });
        match target {
            Some((name, v)) => {
                let id = self.sub(name, v);
                self.refs.insert(key, id);
                Node::Ref(id)
            }
            None => Node::Error(format!("unresolvable schema reference `{r}`")),
        }
    }

    fn build(&mut self, file: &'a str, value: &'a Value) -> Node {
        let m = match value {
            Value::Bool(false) => return Node::False,
            Value::Object(m) => m,
            _ => return Node::True,
        };
        if let Some(r) = m.get("$ref").and_then(Value::as_str) {
            return self.reference(file, r);
        }
        let mut k = Keywords::default();
        let mut exclusive = (None, None);
        let mut conditional = (None, None, None);
        let mut additional_items = None;
        let (mut min_contains, mut max_contains) = (None, None);
        for (key, v) in m {
            match key.as_str() {
                "type" => match v {
                    Value::String(t) => {
                        k.types = type_mask(t);
                        k.type_label = Cow::Owned(t.clone());
                    }
                    Value::Array(a) => {
                        let names: Vec<&str> = a.iter().filter_map(Value::as_str).collect();
                        k.types = names.iter().fold(0, |acc, n| acc | type_mask(n));
                        k.type_label = Cow::Owned(names.join(" or "));
                    }
                    _ => {}
                },
                "const" => k.constant = Some(v.clone()),
                "enum" => {
                    if let Value::Array(options) = v {
                        k.enumeration = Some(options.clone());
                    }
                }
                "minLength" => k.min_length = v.as_u64(),
                "maxLength" => k.max_length = v.as_u64(),
                "pattern" => k.pattern = v.as_str().map(Pattern::new),
                "minimum" => k.minimum = v.as_f64(),
                "maximum" => k.maximum = v.as_f64(),
                "exclusiveMinimum" => exclusive.0 = Some(v),
                "exclusiveMaximum" => exclusive.1 = Some(v),
                "multipleOf" => k.multiple_of = v.as_f64().filter(|x| *x > 0.0 && x.is_finite()),
                "required" => {
                    if let Value::Array(req) = v {
                        k.required = req.iter().filter_map(Value::as_str).map(str::to_string).collect();
                    }
                }
                "properties" => {
                    if let Value::Object(props) = v {
                        for (name, s) in props {
                            let id = self.sub(file, s);
                            k.properties.push((name.clone(), id));
                        }
                        k.properties.sort_by(|a, b| a.0.cmp(&b.0));
                        k.properties.dedup_by(|a, b| a.0 == b.0);
                    }
                }
                "patternProperties" => {
                    if let Value::Object(pp) = v {
                        for (pattern, s) in pp {
                            let id = self.sub(file, s);
                            k.pattern_properties.push((Pattern::new(pattern), id));
                        }
                    }
                }
                "additionalProperties" => k.additional_properties = Some(self.sub(file, v)),
                "propertyNames" => k.property_names = Some(self.sub(file, v)),
                "minProperties" => k.min_properties = v.as_u64(),
                "maxProperties" => k.max_properties = v.as_u64(),
                "dependencies" | "dependentRequired" | "dependentSchemas" => {
                    let Value::Object(deps) = v else { continue };
                    for (name, d) in deps {
                        match d {
                            Value::Array(names) if key != "dependentSchemas" => {
                                let names = names.iter().filter_map(Value::as_str).map(str::to_string);
                                k.dependent_required.push((name.clone(), names.collect()));
                            }
                            Value::Object(_) | Value::Bool(_) if key != "dependentRequired" => {
                                let id = self.sub(file, d);
                                k.dependent_schemas.push((name.clone(), id));
                            }
                            _ => {}
                        }
                    }
                }
                "items" => {
                    k.items = match v {
                        Value::Array(tuple) => {
                            Items::Tuple(tuple.iter().map(|s| self.sub(file, s)).collect())
                        }
                        s => Items::All(self.sub(file, s)),
                    }
                }
                "additionalItems" => additional_items = Some(v),
                "minItems" => k.min_items = v.as_u64(),
                "maxItems" => k.max_items = v.as_u64(),
                "uniqueItems" => k.unique_items = v == &Value::Bool(true),
                "contains" => k.contains = Some(self.sub(file, v)),
                "minContains" => min_contains = v.as_u64(),
                "maxContains" => max_contains = v.as_u64(),
                "allOf" | "anyOf" | "oneOf" => {
                    let Value::Array(list) = v else { continue };
                    let ids = list.iter().map(|s| self.sub(file, s)).collect();
                    match key.as_str() {
                        "allOf" => k.all_of = ids,
                        "anyOf" => k.any_of = ids,
                        _ => k.one_of = ids,
                    }
                }
                "not" => k.not = Some(self.sub(file, v)),
                "if" => conditional.0 = Some(v),
                "then" => conditional.1 = Some(v),
                "else" => conditional.2 = Some(v),
                _ => {}
            }
        }
        match exclusive.0 {
            Some(Value::Bool(true)) => k.exclusive_minimum = k.minimum.take(),
            Some(v) => k.exclusive_minimum = v.as_f64(),
            None => {}
        }
        match exclusive.1 {
            Some(Value::Bool(true)) => k.exclusive_maximum = k.maximum.take(),
            Some(v) => k.exclusive_maximum = v.as_f64(),
            None => {}
        }
        if matches!(k.items, Items::Tuple(_))
            && let Some(s) = additional_items
        {
            k.additional_items = Some(self.sub(file, s));
        }
        if k.contains.is_some() {
            k.min_contains = min_contains;
            k.max_contains = max_contains;
        }
        if let (Some(cond), then, other) = conditional {
            let cond = self.sub(file, cond);
            let then = then.map(|s| self.sub(file, s));
            let other = other.map(|s| self.sub(file, s));
            k.if_then_else = Some((cond, then, other));
        }
        Node::Schema(Box::new(k))
    }
}

impl Compiled {
    /// Compiles the document `root` of `docs`; references into the other documents resolve.
    fn new(docs: &HashMap<String, Arc<Value>>, root: &str) -> Self {
        let mut b = Builder {
            docs,
            nodes: Vec::new(),
            refs: HashMap::new(),
            queue: Vec::new(),
            too_big: None,
        };
        let root = match docs.get_key_value(root) {
            Some((key, doc)) => b.sub(key, doc),
            None => {
                b.nodes
                    .push(Node::Error(format!("schema `{root}` is not available")));
                0
            }
        };
        b.run();
        Self { nodes: b.nodes, root }
    }

    /// Compiles one standalone schema document; its `#/…` references resolve inside it.
    pub(crate) fn standalone(schema: Arc<Value>) -> Self {
        let mut docs = HashMap::new();
        docs.insert("standalone".to_string(), schema);
        Self::new(&docs, "standalone")
    }

    /// Validates `instance`. Issues are (JSON pointer, message), at most [`MAX_ISSUES`] plus a
    /// budget note. Recursion is bounded by `MAX_DEPTH`: run it on a large stack
    /// ([`on_big_stack`]).
    pub(crate) fn validate(&self, instance: &Value) -> Vec<(String, String)> {
        let mut v = Validator {
            nodes: &self.nodes,
            steps: 0,
            issues: Vec::new(),
            stopped: false,
        };
        v.check(instance, self.root, &Ptr::Root, 0, true);
        v.issues
    }
}

struct Validator<'a> {
    nodes: &'a [Node],
    steps: u64,
    issues: Vec<(String, String)>,
    stopped: bool,
}

/// A JSON pointer as a chain of borrowed segments: built for free while descending, rendered
/// only when an issue is reported.
#[derive(Clone, Copy)]
enum Ptr<'p> {
    Root,
    Key(&'p Ptr<'p>, &'p str),
    Index(&'p Ptr<'p>, usize),
}

impl Ptr<'_> {
    fn render(&self) -> String {
        let mut segments = Vec::new();
        let mut at = self;
        loop {
            match at {
                Ptr::Root => break,
                Ptr::Key(parent, key) => {
                    segments.push(escape_pointer(key));
                    at = parent;
                }
                Ptr::Index(parent, i) => {
                    segments.push(i.to_string());
                    at = parent;
                }
            }
        }
        let mut out = String::new();
        for seg in segments.iter().rev() {
            out.push('/');
            out.push_str(seg);
        }
        out
    }
}

impl Validator<'_> {
    /// For a failed `anyOf`: the allowed values when every branch is a `const`.
    fn any_of_message(&self, any: &[NodeId]) -> String {
        let consts: Vec<&Value> = any
            .iter()
            .filter_map(|id| match self.nodes.get(*id) {
                Some(Node::Schema(k)) => k.constant.as_ref(),
                _ => None,
            })
            .collect();
        if consts.len() == any.len() && !consts.is_empty() {
            one_of_message(consts.into_iter())
        } else {
            "does not match any of the allowed shapes".to_string()
        }
    }

    fn report(&mut self, collect: bool, ptr: &Ptr<'_>, message: String) {
        if collect && self.issues.len() < MAX_ISSUES {
            self.issues.push((ptr.render(), message));
        }
    }

    /// True when `inst` is valid. With `collect`, issues are recorded; without, it stops early.
    fn check(&mut self, inst: &Value, id: NodeId, ptr: &Ptr<'_>, depth: u32, collect: bool) -> bool {
        self.steps += 1;
        if self.steps > BUDGET {
            if !self.stopped {
                self.stopped = true;
                self.issues.push((
                    String::new(),
                    "validation stopped: the document is too large or complex".into(),
                ));
            }
            return true;
        }
        if depth > MAX_DEPTH {
            self.report(collect, ptr, "nesting too deep to validate".into());
            return false;
        }
        let nodes = self.nodes;
        let k = match nodes.get(id) {
            None | Some(Node::True) => return true,
            Some(Node::False) => {
                self.report(collect, ptr, "no value is allowed here".into());
                return false;
            }
            Some(Node::Error(message)) => {
                self.report(collect, ptr, message.clone());
                return false;
            }
            Some(Node::Ref(target)) => return self.check(inst, *target, ptr, depth + 1, collect),
            Some(Node::Schema(k)) => k,
        };

        if k.types != T_ANY && !type_ok(inst, k.types) {
            let expected = if k.type_label.is_empty() {
                "?"
            } else {
                k.type_label.as_ref()
            };
            self.report(
                collect,
                ptr,
                format!("expected {expected}, found {}", type_name(inst)),
            );
            return false;
        }
        if let Some(c) = &k.constant
            && !json_eq(inst, c)
        {
            self.report(collect, ptr, format!("must be {}", short(c)));
            return false;
        }
        if let Some(options) = &k.enumeration
            && !options.iter().any(|o| json_eq(inst, o))
        {
            self.report(collect, ptr, one_of_message(options.iter()));
            return false;
        }

        let mut ok = match inst {
            Value::String(s) => self.check_string(s, k, ptr, collect),
            Value::Number(n) => self.check_number(n.as_f64().unwrap_or(0.0), k, ptr, collect),
            Value::Object(obj) => self.check_object(inst, obj, k, ptr, depth, collect),
            Value::Array(items) => self.check_array(items, k, ptr, depth, collect),
            _ => true,
        };
        if !ok && !collect {
            return false;
        }

        for s in &k.all_of {
            ok &= self.check(inst, *s, ptr, depth + 1, collect);
            if !ok && !collect {
                return false;
            }
        }
        if !k.any_of.is_empty() {
            let matched = k
                .any_of
                .iter()
                .any(|s| self.check(inst, *s, ptr, depth + 1, false));
            if !matched {
                if collect {
                    let message = self.any_of_message(&k.any_of);
                    self.report(collect, ptr, message);
                }
                ok = false;
            }
        }
        if !k.one_of.is_empty() {
            let mut count = 0;
            for s in &k.one_of {
                if self.check(inst, *s, ptr, depth + 1, false) {
                    count += 1;
                    if count > 1 {
                        break;
                    }
                }
            }
            if count != 1 {
                let message = if count == 0 {
                    "does not match any of the allowed shapes"
                } else {
                    "matches more than one of the exclusive shapes"
                };
                self.report(collect, ptr, message.into());
                ok = false;
            }
        }
        if let Some(not) = k.not
            && self.check(inst, not, ptr, depth + 1, false)
        {
            self.report(collect, ptr, "matches a shape that is not allowed".into());
            ok = false;
        }
        if let Some((cond, then, other)) = k.if_then_else {
            let branch = if self.check(inst, cond, ptr, depth + 1, false) {
                then
            } else {
                other
            };
            if let Some(b) = branch {
                ok &= self.check(inst, b, ptr, depth + 1, collect);
            }
        }
        ok
    }

    fn check_string(&mut self, s: &str, k: &Keywords, ptr: &Ptr<'_>, collect: bool) -> bool {
        let mut ok = true;
        if k.max_length.is_some() || k.min_length.is_some() {
            let len = s.chars().count() as u64;
            if let Some(max) = k.max_length
                && len > max
            {
                self.report(collect, ptr, format!("longer than {max} characters"));
                ok = false;
            }
            if let Some(min) = k.min_length
                && len < min
            {
                self.report(collect, ptr, format!("shorter than {min} characters"));
                ok = false;
            }
        }
        if let Some(p) = &k.pattern
            && p.matches(s) == Some(false)
        {
            self.report(collect, ptr, format!("does not match the pattern `{}`", p.source));
            ok = false;
        }
        ok
    }

    fn check_number(&mut self, x: f64, k: &Keywords, ptr: &Ptr<'_>, collect: bool) -> bool {
        let mut ok = true;
        if let Some(min) = k.minimum
            && x < min
        {
            self.report(collect, ptr, format!("less than {min}"));
            ok = false;
        }
        if let Some(max) = k.maximum
            && x > max
        {
            self.report(collect, ptr, format!("greater than {max}"));
            ok = false;
        }
        if let Some(min) = k.exclusive_minimum
            && x <= min
        {
            self.report(collect, ptr, format!("not greater than {min}"));
            ok = false;
        }
        if let Some(max) = k.exclusive_maximum
            && x >= max
        {
            self.report(collect, ptr, format!("not less than {max}"));
            ok = false;
        }
        if let Some(m) = k.multiple_of {
            let q = x / m;
            // Lenient with binary fractions: 0.3 is a multiple of 0.1.
            if q.is_finite() && (q - q.round()).abs() > 1e-9 * q.abs().max(1.0) {
                self.report(collect, ptr, format!("not a multiple of {m}"));
                ok = false;
            }
        }
        ok
    }

    fn check_object(
        &mut self,
        inst: &Value,
        obj: &Map<String, Value>,
        k: &Keywords,
        ptr: &Ptr<'_>,
        depth: u32,
        collect: bool,
    ) -> bool {
        let mut ok = true;
        for r in &k.required {
            if !obj.contains_key(r) {
                self.report(collect, ptr, format!("missing required property `{r}`"));
                ok = false;
                if !collect {
                    return false;
                }
            }
        }
        let count = obj.len() as u64;
        if let Some(min) = k.min_properties
            && count < min
        {
            self.report(collect, ptr, format!("fewer than {min} properties"));
            ok = false;
        }
        if let Some(max) = k.max_properties
            && count > max
        {
            self.report(collect, ptr, format!("more than {max} properties"));
            ok = false;
        }
        for (name, needs) in &k.dependent_required {
            if obj.contains_key(name) {
                for n in needs {
                    if !obj.contains_key(n) {
                        self.report(collect, ptr, format!("property `{name}` requires property `{n}`"));
                        ok = false;
                    }
                }
            }
        }
        if !ok && !collect {
            return false;
        }
        for (name, s) in &k.dependent_schemas {
            if obj.contains_key(name) {
                ok &= self.check(inst, *s, ptr, depth + 1, collect);
                if !ok && !collect {
                    return false;
                }
            }
        }
        let checks_keys = !k.properties.is_empty()
            || !k.pattern_properties.is_empty()
            || k.additional_properties.is_some()
            || k.property_names.is_some();
        if !checks_keys {
            return ok;
        }
        for (key, value) in obj {
            if let Some(names) = k.property_names
                && !self.check(&Value::String(key.clone()), names, &Ptr::Root, depth + 1, false)
            {
                let p = Ptr::Key(ptr, key);
                self.report(collect, &p, format!("property name `{key}` is not allowed"));
                ok = false;
            }
            let mut matched = false;
            if let Ok(i) = k.properties.binary_search_by(|(name, _)| name.as_str().cmp(key))
                && let Some((_, s)) = k.properties.get(i)
            {
                matched = true;
                let p = Ptr::Key(ptr, key);
                ok &= self.check(value, *s, &p, depth + 1, collect);
            }
            for (pattern, s) in &k.pattern_properties {
                match pattern.matches(key) {
                    Some(true) => {
                        matched = true;
                        let p = Ptr::Key(ptr, key);
                        ok &= self.check(value, *s, &p, depth + 1, collect);
                    }
                    // An unknown pattern: the key counts as covered, its schema is not applied.
                    None => matched = true,
                    Some(false) => {}
                }
            }
            if !matched && let Some(s) = k.additional_properties {
                let p = Ptr::Key(ptr, key);
                if matches!(self.nodes.get(s), Some(Node::False)) {
                    self.report(collect, &p, format!("unexpected property `{key}`"));
                    ok = false;
                } else {
                    ok &= self.check(value, s, &p, depth + 1, collect);
                }
            }
            if !ok && !collect {
                return false;
            }
        }
        ok
    }

    fn check_array(
        &mut self,
        items: &[Value],
        k: &Keywords,
        ptr: &Ptr<'_>,
        depth: u32,
        collect: bool,
    ) -> bool {
        let mut ok = true;
        let len = items.len() as u64;
        if let Some(min) = k.min_items
            && len < min
        {
            self.report(collect, ptr, format!("fewer than {min} items"));
            ok = false;
        }
        if let Some(max) = k.max_items
            && len > max
        {
            self.report(collect, ptr, format!("more than {max} items"));
            ok = false;
        }
        if k.unique_items && !all_unique(items) {
            self.report(collect, ptr, "contains duplicate items".into());
            ok = false;
        }
        if !ok && !collect {
            return false;
        }
        match &k.items {
            Items::None => {}
            Items::All(s) => {
                for (i, item) in items.iter().enumerate() {
                    let p = Ptr::Index(ptr, i);
                    ok &= self.check(item, *s, &p, depth + 1, collect);
                    if !ok && !collect {
                        return false;
                    }
                }
            }
            Items::Tuple(tuple) => {
                for (i, item) in items.iter().enumerate() {
                    let schema = match tuple.get(i) {
                        Some(s) => *s,
                        None => match k.additional_items {
                            Some(s) => s,
                            None => break,
                        },
                    };
                    let p = Ptr::Index(ptr, i);
                    if i >= tuple.len() && matches!(self.nodes.get(schema), Some(Node::False)) {
                        self.report(collect, &p, "unexpected item".into());
                        ok = false;
                    } else {
                        ok &= self.check(item, schema, &p, depth + 1, collect);
                    }
                    if !ok && !collect {
                        return false;
                    }
                }
            }
        }
        if let Some(c) = k.contains {
            let mut hits: u64 = 0;
            for item in items {
                if self.check(item, c, &Ptr::Root, depth + 1, false) {
                    hits += 1;
                }
            }
            let min = k.min_contains.unwrap_or(1);
            if hits < min {
                let message = if min == 1 {
                    "no item matches the required shape".to_string()
                } else {
                    format!("fewer than {min} items match the required shape")
                };
                self.report(collect, ptr, message);
                ok = false;
            }
            if let Some(max) = k.max_contains
                && hits > max
            {
                self.report(
                    collect,
                    ptr,
                    format!("more than {max} items match the required shape"),
                );
                ok = false;
            }
        }
        ok
    }
}

fn type_ok(inst: &Value, mask: u8) -> bool {
    let bit = match inst {
        Value::Object(_) => T_OBJECT,
        Value::Array(_) => T_ARRAY,
        Value::String(_) => T_STRING,
        Value::Bool(_) => T_BOOLEAN,
        Value::Null => T_NULL,
        Value::Number(n) => {
            if mask & T_NUMBER != 0 {
                return true;
            }
            let integral = n.is_i64() || n.is_u64() || n.as_f64().is_some_and(|f| f.fract() == 0.0);
            return integral && mask & T_INTEGER != 0;
        }
    };
    mask & bit != 0
}

/// `uniqueItems`: equality as in `json_eq` (numbers by value), through a canonical rendering.
/// Values nested deeper than `MAX_DEPTH` count as unique.
fn all_unique(items: &[Value]) -> bool {
    let mut seen = HashSet::with_capacity(items.len());
    for item in items {
        let mut s = String::new();
        if !canonical(item, 0, &mut s) {
            return true;
        }
        if !seen.insert(s) {
            return false;
        }
    }
    true
}

fn canonical(v: &Value, depth: u32, out: &mut String) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    match v {
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.push_str(&i.to_string());
            } else if let Some(u) = n.as_u64() {
                out.push_str(&u.to_string());
            } else {
                let f = n.as_f64().unwrap_or(0.0);
                if f.fract() == 0.0 && f.abs() < 9.0e15 {
                    out.push_str(&(f as i64).to_string());
                } else {
                    out.push_str(&format!("{f:?}"));
                }
            }
        }
        Value::Array(a) => {
            out.push('[');
            for x in a {
                if !canonical(x, depth + 1, out) {
                    return false;
                }
                out.push(',');
            }
            out.push(']');
        }
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for key in keys {
                out.push_str(&format!("{key:?}:"));
                if !m.get(key).is_some_and(|x| canonical(x, depth + 1, out)) {
                    return false;
                }
                out.push(',');
            }
            out.push('}');
        }
        other => out.push_str(&other.to_string()),
    }
    true
}

/// `must be one of …` over the first 12 values.
fn one_of_message<'v>(values: impl Iterator<Item = &'v Value>) -> String {
    let list: Vec<String> = values.take(12).map(short).collect();
    format!("must be one of {}", list.join(", "))
}

/// A short rendering of a schema value (a `const`) for messages.
fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.chars().count() > 60 {
        let cut: String = s.chars().take(57).collect();
        format!("{cut}...")
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn issues(schema: Value, inst: Value) -> Vec<(String, String)> {
        Compiled::standalone(Arc::new(schema)).validate(&inst)
    }

    fn valid(schema: &Value, inst: Value) -> bool {
        Compiled::standalone(Arc::new(schema.clone()))
            .validate(&inst)
            .is_empty()
    }

    #[test]
    fn every_written_pbir_schema_version_is_vendored() {
        // The writer's `$schema` versions (bicraft-versions) must validate exactly, not through a
        // newer-minor substitution.
        for schema in bicraft_versions::WRITTEN {
            let Some(name) = schema.key.strip_prefix("item/report/definition/") else {
                continue;
            };
            let version = parse_version(schema.version);
            assert!(version.is_some(), "{schema:?}");
            let found = version.and_then(|v| vendored(name, v));
            assert_eq!(
                found.map(|(_, substituted)| substituted),
                Some(false),
                "{} {} is not vendored",
                schema.key,
                schema.version
            );
        }
    }

    #[test]
    fn every_vendored_file_parses_resolves_and_compiles() {
        for (path, _) in FILES {
            let docs = load(path).unwrap_or_else(|| panic!("{path}"));
            let c = Compiled::new(&docs, path);
            assert!(
                !c.nodes.iter().any(|n| matches!(n, Node::Error(_))),
                "{path}: unresolvable reference"
            );
            for n in &c.nodes {
                if let Node::Schema(k) = n {
                    let unknown = k
                        .pattern
                        .iter()
                        .chain(k.pattern_properties.iter().map(|(p, _)| p))
                        .find(|p| p.regex.is_none());
                    assert!(unknown.is_none(), "{path}: pattern not compiled");
                }
            }
            assert!(vendored_schema(path).is_some());
        }
        // Cached: the same compiled schema comes back.
        let a = vendored_schema("page/2.1.0/schema.json").unwrap();
        let b = vendored_schema("page/2.1.0/schema.json").unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert!(vendored_schema("nope/1.0.0/schema.json").is_none());
    }

    #[test]
    fn version_substitution() {
        assert_eq!(
            vendored("page", (2, 1, 0)),
            Some(("page/2.1.0/schema.json".into(), false))
        );
        assert_eq!(vendored("visualContainer", (2, 99, 0)).map(|v| v.1), Some(true));
        assert_eq!(vendored("page", (9, 0, 0)), None);
        assert_eq!(vendored("nope", (1, 0, 0)), None);
    }

    #[test]
    fn reports_errors_with_pointers() {
        let schema = vendored_schema("page/2.1.0/schema.json").unwrap();
        let doc = json!({
            "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/page/2.1.0/schema.json",
            "name": "p", "displayName": 3, "displayOption": "Nope", "bogus": true
        });
        let issues = schema.validate(&doc);
        let pointers: Vec<&str> = issues.iter().map(|(p, _)| p.as_str()).collect();
        assert!(pointers.contains(&"/displayName"), "{issues:?}");
        assert!(pointers.contains(&"/displayOption"), "{issues:?}");
        assert!(pointers.contains(&"/bogus"), "{issues:?}");
    }

    #[test]
    fn version_pattern_is_checked() {
        let schema = vendored_schema("versionMetadata/1.0.0/schema.json").unwrap();
        let url = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/versionMetadata/1.0.0/schema.json";
        assert!(
            schema
                .validate(&json!({"$schema": url, "version": "2.0.0"}))
                .is_empty()
        );
        let bad = schema.validate(&json!({"$schema": url, "version": "2.01.0"}));
        assert_eq!(bad.len(), 1, "{bad:?}");
        assert_eq!(bad[0].0, "/version");
        assert!(bad[0].1.starts_with("does not match the pattern"));
    }

    #[test]
    fn core_keywords() {
        let s = json!({
            "type": "object",
            "required": ["a"],
            "properties": {"a": {"type": ["string", "null"], "maxLength": 3}, "n": {"type": "integer", "minimum": 1}},
            "additionalProperties": false
        });
        assert!(valid(&s, json!({"a": "abc", "n": 2})));
        assert!(valid(&s, json!({"a": null, "n": 2.0})));
        let i = issues(s.clone(), json!({"a": "abcd", "n": 1.5, "x": 1}));
        let got: Vec<(&str, &str)> = i.iter().map(|(p, m)| (p.as_str(), m.as_str())).collect();
        assert!(got.contains(&("/a", "longer than 3 characters")), "{got:?}");
        assert!(got.contains(&("/n", "expected integer, found number")), "{got:?}");
        assert!(got.contains(&("/x", "unexpected property `x`")), "{got:?}");
        let i = issues(s, json!({}));
        assert_eq!(i, vec![(String::new(), "missing required property `a`".into())]);
    }

    #[test]
    fn if_then_else() {
        let s = json!({
            "if": {"properties": {"kind": {"const": "a"}}},
            "then": {"required": ["x"]},
            "else": {"required": ["y"]}
        });
        assert!(valid(&s, json!({"kind": "a", "x": 1})));
        assert!(!valid(&s, json!({"kind": "a", "y": 1})));
        assert!(valid(&s, json!({"kind": "b", "y": 1})));
        assert!(!valid(&s, json!({"kind": "b", "x": 1})));
        // Only `then`: no constraint when `if` fails.
        assert!(valid(
            &json!({"if": {"type": "string"}, "then": {"minLength": 2}}),
            json!(5)
        ));
        assert!(!valid(
            &json!({"if": {"type": "string"}, "then": {"minLength": 2}}),
            json!("a")
        ));
    }

    #[test]
    fn property_names_and_counts() {
        let s = json!({"propertyNames": {"pattern": "^[a-z]+$"}, "minProperties": 1, "maxProperties": 2});
        assert!(valid(&s, json!({"ab": 1})));
        let i = issues(s.clone(), json!({"aB": 1}));
        assert_eq!(
            i,
            vec![("/aB".into(), "property name `aB` is not allowed".into())]
        );
        assert!(!valid(&s, json!({})));
        assert!(!valid(&s, json!({"a": 1, "b": 2, "c": 3})));
    }

    #[test]
    fn dependencies() {
        let s = json!({
            "dependencies": {"a": ["b"], "c": {"required": ["d"]}},
            "dependentRequired": {"e": ["f"]},
            "dependentSchemas": {"g": {"properties": {"h": {"type": "string"}}}}
        });
        assert!(valid(
            &s,
            json!({"a": 1, "b": 1, "c": 1, "d": 1, "e": 1, "f": 1, "g": 1, "h": "x"})
        ));
        assert_eq!(
            issues(s.clone(), json!({"a": 1})),
            vec![(String::new(), "property `a` requires property `b`".into())]
        );
        assert!(!valid(&s, json!({"c": 1})));
        assert!(!valid(&s, json!({"e": 1})));
        assert!(!valid(&s, json!({"g": 1, "h": 2})));
        assert!(valid(&s, json!({"h": 2})));
    }

    #[test]
    fn arrays() {
        let s = json!({"type": "array", "uniqueItems": true, "contains": {"const": 1}});
        assert!(valid(&s, json!([1, 2])));
        assert!(!valid(&s, json!([1, 1.0])));
        assert!(!valid(&s, json!([{"a": 1, "b": [2]}, {"b": [2.0], "a": 1}, 1])));
        assert!(valid(&s, json!([{"a": 1}, {"a": 2}, 1])));
        assert_eq!(
            issues(s, json!([2, 3])),
            vec![(String::new(), "no item matches the required shape".into())]
        );
        let s = json!({"contains": {"type": "string"}, "minContains": 2, "maxContains": 3});
        assert!(valid(&s, json!(["a", "b", 1])));
        assert!(!valid(&s, json!(["a", 1])));
        assert!(!valid(&s, json!(["a", "b", "c", "d"])));
        let s = json!({"items": [{"type": "string"}, {"type": "number"}], "additionalItems": false});
        assert!(valid(&s, json!(["a", 1])));
        assert!(valid(&s, json!(["a"])));
        assert_eq!(
            issues(s.clone(), json!(["a", 1, true])),
            vec![("/2".into(), "unexpected item".into())]
        );
        assert!(!valid(&s, json!([1, 1])));
        let s = json!({"items": [{"type": "string"}], "additionalItems": {"type": "number"}});
        assert!(valid(&s, json!(["a", 1, 2])));
        assert!(!valid(&s, json!(["a", "b"])));
    }

    #[test]
    fn numbers() {
        let s = json!({"exclusiveMinimum": 0, "exclusiveMaximum": 10, "multipleOf": 0.1});
        assert!(valid(&s, json!(0.3)));
        assert!(valid(&s, json!(9.9)));
        assert!(!valid(&s, json!(0)));
        assert!(!valid(&s, json!(10)));
        assert!(!valid(&s, json!(0.25)));
        // Draft-04 boolean form.
        let s = json!({"minimum": 1, "exclusiveMinimum": true, "maximum": 5, "exclusiveMaximum": false});
        assert!(!valid(&s, json!(1)));
        assert!(valid(&s, json!(5)));
        assert!(valid(&json!({"multipleOf": 3}), json!(9)));
        assert!(!valid(&json!({"multipleOf": 3}), json!(10)));
    }

    #[test]
    fn refs_defs_and_cycles() {
        let s = json!({
            "$defs": {"node": {"type": "object", "properties": {"next": {"$ref": "#/$defs/node"}, "v": {"type": "number"}}}},
            "definitions": {"a~b": {"type": "string"}, "x y": {"type": "boolean"}},
            "properties": {
                "list": {"$ref": "#/$defs/node"},
                "esc": {"$ref": "#/definitions/a~0b"},
                "pct": {"$ref": "#/definitions/x%20y"},
                "bad": {"$ref": "#/definitions/missing"}
            }
        });
        assert!(valid(
            &s,
            json!({"list": {"v": 1, "next": {"v": 2, "next": {}}}, "esc": "s", "pct": true})
        ));
        assert!(!valid(&s, json!({"list": {"next": {"next": {"v": "x"}}}})));
        assert!(!valid(&s, json!({"esc": 1})));
        assert!(!valid(&s, json!({"pct": 1})));
        assert_eq!(
            issues(s, json!({"bad": 1})),
            vec![(
                "/bad".into(),
                "unresolvable schema reference `#/definitions/missing`".into()
            )]
        );
        // A reference cycle with no progress stops at the depth bound instead of overflowing.
        let cyc = json!({"definitions": {"a": {"$ref": "#/definitions/b"}, "b": {"allOf": [{"$ref": "#/definitions/a"}]}}, "$ref": "#/definitions/a"});
        let out = on_big_stack(|| issues(cyc.clone(), json!(1))).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1, "nesting too deep to validate");
    }

    #[test]
    fn unknown_patterns_and_format_pass() {
        let s = json!({
            "properties": {"a": {"pattern": "(?<=x)y"}, "d": {"format": "date-time"}},
            "patternProperties": {"\\p{L}+": {"type": "number"}},
            "additionalProperties": false
        });
        // The unknown pattern property covers every key but its schema is not applied.
        assert!(valid(
            &s,
            json!({"a": "anything", "d": "not a date", "zz": "string"})
        ));
    }

    #[test]
    fn boolean_schemas_and_combinators() {
        assert!(valid(&json!(true), json!(1)));
        assert_eq!(
            issues(json!(false), json!(1)),
            vec![(String::new(), "no value is allowed here".into())]
        );
        let s = json!({"anyOf": [{"const": "a"}, {"const": "b"}]});
        assert_eq!(
            issues(s, json!("c")),
            vec![(String::new(), "must be one of \"a\", \"b\"".into())]
        );
        let s = json!({"oneOf": [{"type": "number"}, {"type": "integer"}]});
        assert!(valid(&s, json!(1.5)));
        assert!(!valid(&s, json!(1)));
        assert!(!valid(&json!({"not": {"type": "string"}}), json!("s")));
        assert!(!valid(
            &json!({"allOf": [{"minLength": 1}, {"maxLength": 1}]}),
            json!("ab")
        ));
    }

    #[test]
    fn hostile_depth_does_not_overflow() {
        let issues = on_big_stack(|| {
            let schema = vendored_schema("visualContainer/2.12.0/schema.json").unwrap();
            // serde_json caps parsing depth at 128; build something deeper by hand instead.
            let mut v = json!({"Literal": {"Value": "1"}});
            for _ in 0..3000 {
                // Built by hand: `json!` would re-serialize `v` each time (quadratic).
                let mut inner = Map::new();
                inner.insert("Expression".into(), v);
                let mut outer = Map::new();
                outer.insert("Not".into(), Value::Object(inner));
                v = Value::Object(outer);
            }
            let doc = json!({"$schema": "x", "name": "v", "position": {"x":0,"y":0,"height":1,"width":1},
                "filterConfig": {"filters": [{"name": "f", "filter": {"Version": 2, "From": [], "Where": [{"Condition": v}]}}]}});
            schema.validate(&doc)
        })
        .unwrap();
        assert!(!issues.is_empty());
    }

    #[test]
    fn theme_schema_compiles() {
        let text = include_str!("../../visuals/schemas/reportThemeSchema-2.159.json");
        let v: Value = serde_json::from_str(text).unwrap();
        let c = Compiled::standalone(Arc::new(v));
        assert!(!c.nodes.iter().any(|n| matches!(n, Node::Error(_))));
        assert!(c.validate(&json!({"name": "t"})).is_empty());
        assert!(!c.validate(&json!({"name": 3})).is_empty());
    }
}
