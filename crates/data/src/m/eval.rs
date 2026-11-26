//! The M interpreter: environments, lazy `let`/record bindings, operators,
//! function invocation and the evaluation caps.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Instant;

use super::SourceOptions;
use super::ast::{BinOp, Bindings, Expr, Handler, ListItem, Param, TypeExpr, UnOp};
use super::convert::CultureInfo;
use super::lib;
use super::value::{
    Flow, Func, Kind, MError, MType, Num, Record, Res, Table, Value, err, expr_err, fatal, unsupported,
};
use crate::Error;

pub(crate) const MS_PER_DAY: i64 = 86_400_000;

enum Slot {
    Pending(Rc<Expr>),
    Busy,
    Done(Value),
}

/// One scope: a `let`, a record initializer, a function call or the shared queries.
pub(crate) struct Frame {
    names: Vec<Rc<str>>,
    slots: Vec<RefCell<Slot>>,
    parent: Env,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame").field("names", &self.names).finish()
    }
}

pub(crate) type Env = Option<Rc<Frame>>;

pub(crate) struct Interp {
    pub culture: CultureInfo,
    pub file_roots: Vec<std::path::PathBuf>,
    pub limits: crate::Limits,
    /// The connectors the caller allowed (`crate::connect`).
    pub connectors: crate::connect::Connectors,
    /// What the UTC clock functions return (ms since 1899-12-30); `None`: unsupported.
    pub clock_utc_ms: Option<i64>,
    /// Local time minus UTC (minutes) for the local clock functions; `None`: unsupported.
    pub clock_local_offset_minutes: Option<i32>,
    /// Folding indicators (`super::folding`): a remote table's rows are never read; the first
    /// read sets `probe_hit` and stops the evaluation.
    pub probe: bool,
    /// The connector whose rows a probe needed.
    pub probe_hit: Option<&'static str>,
    /// The source requests of the evaluation (query diagnostics), when kept.
    pub requests: Option<Vec<super::folding::SourceRequest>>,
    steps: u64,
    deadline: Option<Instant>,
    depth: usize,
    /// Lazily bound frames; their slots are cleared at the end to break `Rc` cycles.
    lazy_frames: Vec<Weak<Frame>>,
    compact_at: usize,
    globals: Env,
    lib: HashMap<&'static str, &'static lib::Builtin>,
}

impl Drop for Interp {
    fn drop(&mut self) {
        for w in self.lazy_frames.drain(..) {
            if let Some(f) = w.upgrade() {
                for s in &f.slots {
                    if let Ok(mut s) = s.try_borrow_mut() {
                        *s = Slot::Busy;
                    }
                }
            }
        }
        if let Some(g) = self.globals.take() {
            for s in &g.slots {
                if let Ok(mut s) = s.try_borrow_mut() {
                    *s = Slot::Busy;
                }
            }
        }
    }
}

impl Interp {
    pub fn new(opts: &SourceOptions, shared: Vec<(Rc<str>, Rc<Expr>)>) -> Self {
        let millis = opts.limits.max_millis;
        let mut me = Self {
            culture: CultureInfo::from_name(opts.culture.as_deref().unwrap_or("en-US")),
            file_roots: opts.file_roots.clone(),
            limits: opts.limits,
            connectors: opts.connectors.clone(),
            clock_utc_ms: opts.clock_utc_ms,
            clock_local_offset_minutes: opts.clock_local_offset_minutes,
            probe: false,
            probe_hit: None,
            requests: None,
            steps: 0,
            deadline: (millis > 0).then(|| Instant::now() + std::time::Duration::from_millis(millis)),
            depth: 0,
            lazy_frames: Vec::new(),
            compact_at: 1024,
            globals: None,
            lib: lib::table(),
        };
        if !shared.is_empty() {
            let (names, exprs): (Vec<_>, Vec<_>) = shared.into_iter().unzip();
            let frame = Rc::new(Frame {
                names,
                slots: exprs
                    .into_iter()
                    .map(|e| RefCell::new(Slot::Pending(e)))
                    .collect(),
                parent: None,
            });
            me.globals = Some(frame);
        }
        me
    }

    /// Counts `n` units of work against the step budget and checks the deadline.
    pub fn tick(&mut self, n: u64) -> Res<()> {
        let before = self.steps;
        self.steps = self.steps.saturating_add(n);
        if self.steps > self.limits.max_steps {
            return fatal(Error::LimitExceeded(format!(
                "M evaluation took more than {} steps",
                self.limits.max_steps
            )));
        }
        if before >> 12 != self.steps >> 12
            && let Some(d) = self.deadline
            && Instant::now() > d
        {
            return fatal(Error::LimitExceeded(format!(
                "M evaluation took longer than {} ms",
                self.limits.max_millis
            )));
        }
        Ok(())
    }

    /// Checks a produced row/item count against `max_rows`.
    pub fn check_rows(&self, n: usize) -> Res<()> {
        if n > self.limits.max_rows {
            return fatal(Error::LimitExceeded(format!(
                "more than {} rows",
                self.limits.max_rows
            )));
        }
        Ok(())
    }

    pub fn check_text(&self, n: usize) -> Res<()> {
        if n > self.limits.max_cell_bytes {
            return fatal(Error::LimitExceeded(format!(
                "a text value is longer than {} bytes",
                self.limits.max_cell_bytes
            )));
        }
        Ok(())
    }

    fn enter(&mut self) -> Res<()> {
        self.depth += 1;
        if self.depth > self.limits.max_eval_depth {
            return fatal(Error::LimitExceeded(format!(
                "M evaluation nested deeper than {} (recursion?)",
                self.limits.max_eval_depth
            )));
        }
        Ok(())
    }

    pub(crate) fn lazy_frame(&mut self, bindings: &Bindings, parent: &Env) -> Rc<Frame> {
        let frame = Rc::new(Frame {
            names: bindings.iter().map(|(n, _)| n.clone()).collect(),
            slots: bindings
                .iter()
                .map(|(_, e)| RefCell::new(Slot::Pending(e.clone())))
                .collect(),
            parent: parent.clone(),
        });
        if self.lazy_frames.len() >= self.compact_at {
            self.lazy_frames.retain(|w| w.strong_count() > 0);
            self.compact_at = self.lazy_frames.len().saturating_mul(2).max(1024);
        }
        self.lazy_frames.push(Rc::downgrade(&frame));
        frame
    }

    fn force(&mut self, frame: &Rc<Frame>, i: usize) -> Res<Value> {
        let Some(cell) = frame.slots.get(i) else {
            return expr_err("bad binding");
        };
        let expr = {
            let Ok(mut slot) = cell.try_borrow_mut() else {
                return expr_err("A cyclic reference was encountered during evaluation.");
            };
            match &*slot {
                Slot::Done(v) => return v.clone().check(),
                Slot::Busy => return expr_err("A cyclic reference was encountered during evaluation."),
                Slot::Pending(e) => {
                    let e = e.clone();
                    *slot = Slot::Busy;
                    e
                }
            }
        };
        let r = self.eval(&expr, &Some(frame.clone()));
        let stored = Value::store(r)?;
        if let Ok(mut slot) = cell.try_borrow_mut() {
            *slot = Slot::Done(stored.clone());
        }
        stored.check()
    }

    fn lookup(&mut self, env: &Env, name: &str) -> Res<Value> {
        let mut cur = env.clone();
        while let Some(f) = cur {
            if let Some(i) = f.names.iter().position(|n| &**n == name) {
                return self.force(&f, i);
            }
            cur = f.parent.clone();
        }
        if let Some(g) = self.globals.clone()
            && let Some(i) = g.names.iter().position(|n| &**n == name)
        {
            return self.force(&g, i);
        }
        if let Some(v) = lib::constant(name, self) {
            return Ok(v);
        }
        if let Some(b) = self.lib.get(name) {
            return Ok(Value::Function(Rc::new(Func::Builtin(b))));
        }
        if self.connectors.find(name).is_some() {
            return Ok(Value::Function(Rc::new(Func::Connector(Rc::from(name)))));
        }
        if name.contains('.')
            && let Some(what) = lib::files::external(name)
        {
            return fatal(lib::files::external_error(name, what));
        }
        if name.contains('.') || name.starts_with('#') {
            unsupported(format!("M function `{name}` is not supported by bicraft"))
        } else {
            unsupported(format!(
                "M name `{name}` is not defined (a reference to another query or parameter that was not supplied?)"
            ))
        }
    }

    pub fn eval(&mut self, e: &Expr, env: &Env) -> Res<Value> {
        self.enter()?;
        let r = self.eval_inner(e, env);
        self.depth = self.depth.saturating_sub(1);
        r
    }

    fn eval_inner(&mut self, e: &Expr, env: &Env) -> Res<Value> {
        self.tick(1)?;
        match e {
            Expr::Const(v) => Ok(v.clone()),
            Expr::Ident(name) => self.lookup(env, name),
            Expr::Let(bindings, body) => {
                let frame = self.lazy_frame(bindings, env);
                self.eval(body, &Some(frame))
            }
            Expr::If(c, t, f) => match self.eval(c, env)? {
                Value::Logical(true) => self.eval(t, env),
                Value::Logical(false) => self.eval(f, env),
                v => expr_err(format!(
                    "We cannot convert the value {} to type Logical.",
                    self.show(&v)
                )),
            },
            Expr::Func(params, body) => Ok(Value::Function(Rc::new(Func::Closure {
                params: params.clone(),
                body: body.clone(),
                env: env.clone(),
            }))),
            Expr::Call(f, args) => {
                let f = self.eval(f, env)?;
                let mut vals = Vec::with_capacity(args.len());
                for a in args {
                    vals.push(self.eval(a, env)?);
                }
                self.call(&f, vals)
            }
            Expr::Field(target, name, opt) => {
                let v = self.eval(target, env)?;
                self.field(&v, name, *opt)
            }
            Expr::Project(target, names, opt) => {
                let v = self.eval(target, env)?;
                self.project(&v, names, *opt)
            }
            Expr::Item(target, index, opt) => {
                let v = self.eval(target, env)?;
                let i = self.eval(index, env)?;
                self.item(&v, &i, *opt)
            }
            Expr::List(items) => {
                let mut out = Vec::with_capacity(items.len());
                for it in items {
                    match it {
                        ListItem::One(e) => {
                            let v = self.eval(e, env);
                            out.push(Value::store(v)?);
                        }
                        ListItem::Range(a, b) => {
                            let a = self.eval(a, env)?;
                            let b = self.eval(b, env)?;
                            self.range(&a, &b, &mut out)?;
                        }
                    }
                    self.check_rows(out.len())?;
                }
                Ok(Value::list(out))
            }
            Expr::Record(fields) => {
                let frame = self.lazy_frame(fields, env);
                let mut values = Vec::with_capacity(fields.len());
                for i in 0..fields.len() {
                    let v = self.force(&frame, i);
                    values.push(Value::store(v)?);
                }
                let names: Vec<Rc<str>> = fields.iter().map(|(n, _)| n.clone()).collect();
                for (i, n) in names.iter().enumerate() {
                    if names.iter().take(i).any(|m| m == n) {
                        return expr_err(format!("The field '{n}' already exists in the record."));
                    }
                }
                Ok(Value::Record(Rc::new(Record {
                    names: Rc::new(names),
                    values,
                })))
            }
            Expr::Unary(op, x) => {
                let v = self.eval(x, env)?;
                self.unary(*op, v)
            }
            Expr::Binary(BinOp::And, a, b) => {
                let a = self.eval(a, env)?;
                match a {
                    Value::Logical(false) => Ok(Value::Logical(false)),
                    Value::Logical(true) | Value::Null => {
                        let b = self.eval(b, env)?;
                        match (a, b) {
                            (_, Value::Logical(false)) => Ok(Value::Logical(false)),
                            (Value::Logical(true), Value::Logical(true)) => Ok(Value::Logical(true)),
                            (_, Value::Logical(_) | Value::Null) => Ok(Value::Null),
                            (_, b) => self.op_error("and", &Value::Null, &b),
                        }
                    }
                    a => self.op_error("and", &a, &Value::Null),
                }
            }
            Expr::Binary(BinOp::Or, a, b) => {
                let a = self.eval(a, env)?;
                match a {
                    Value::Logical(true) => Ok(Value::Logical(true)),
                    Value::Logical(false) | Value::Null => {
                        let b = self.eval(b, env)?;
                        match (a, b) {
                            (_, Value::Logical(true)) => Ok(Value::Logical(true)),
                            (Value::Logical(false), Value::Logical(false)) => Ok(Value::Logical(false)),
                            (_, Value::Logical(_) | Value::Null) => Ok(Value::Null),
                            (_, b) => self.op_error("or", &Value::Null, &b),
                        }
                    }
                    a => self.op_error("or", &a, &Value::Null),
                }
            }
            Expr::Binary(BinOp::Coalesce, a, b) => match self.eval(a, env)? {
                Value::Null => self.eval(b, env),
                v => Ok(v),
            },
            Expr::Binary(op, a, b) => {
                let a = self.eval(a, env)?;
                let b = self.eval(b, env)?;
                self.binary(*op, a, b)
            }
            Expr::Is(x, t) => {
                let v = self.eval(x, env)?;
                let t = self.eval_type(t, env)?;
                Ok(Value::Logical(conforms(&v, &t)))
            }
            Expr::As(x, t) => {
                let v = self.eval(x, env)?;
                let t = self.eval_type(t, env)?;
                if conforms(&v, &t) {
                    Ok(v)
                } else {
                    expr_err(format!(
                        "We cannot convert the value {} to type {}.",
                        self.show(&v),
                        title(t.name())
                    ))
                }
            }
            Expr::Meta(x, m) => {
                let v = self.eval(x, env)?;
                match self.eval(m, env)? {
                    Value::Record(_) => Ok(v),
                    other => expr_err(format!(
                        "We cannot convert the value {} to type Record.",
                        self.show(&other)
                    )),
                }
            }
            Expr::Type(t) => Ok(Value::ty(self.eval_type(t, env)?)),
            Expr::Error(x) => {
                let v = self.eval(x, env)?;
                Err(Flow::Err(Rc::new(self.error_from_value(v)?)))
            }
            Expr::Try(x, handler) => {
                let r = self.eval(x, env);
                match (r, handler) {
                    (Err(Flow::Fatal(f)), _) => Err(Flow::Fatal(f)),
                    (Ok(v), Handler::None) => Ok(Value::record(vec![
                        ("HasError", Value::Logical(false)),
                        ("Value", v),
                    ])),
                    (Ok(v), _) => Ok(v),
                    (Err(Flow::Err(e)), Handler::None) => Ok(Value::record(vec![
                        ("HasError", Value::Logical(true)),
                        ("Error", error_record(&e)),
                    ])),
                    (Err(Flow::Err(_)), Handler::Otherwise(o)) => self.eval(o, env),
                    (Err(Flow::Err(e)), Handler::Catch(params, body)) => {
                        let f = Value::Function(Rc::new(Func::Closure {
                            params: params.clone(),
                            body: body.clone(),
                            env: env.clone(),
                        }));
                        let args = if params.is_empty() {
                            vec![]
                        } else {
                            vec![error_record(&e)]
                        };
                        self.call(&f, args)
                    }
                }
            }
            Expr::NotImplemented => expr_err("Not Implemented"),
        }
    }

    fn error_from_value(&mut self, v: Value) -> Res<MError> {
        match v {
            Value::Text(t) => Ok(MError {
                reason: "Expression.Error".into(),
                message: t.to_string(),
                detail: Value::Null,
            }),
            Value::Record(r) => {
                let text = |v: Option<&Value>| match v {
                    Some(Value::Text(t)) => Some(t.to_string()),
                    _ => None,
                };
                Ok(MError {
                    reason: text(r.get("Reason")).unwrap_or_else(|| "Expression.Error".into()),
                    message: text(r.get("Message")).unwrap_or_default(),
                    detail: r.get("Detail").cloned().unwrap_or(Value::Null),
                })
            }
            other => expr_err(format!(
                "We cannot convert the value {} to type Record.",
                self.show(&other)
            )),
        }
    }

    pub fn eval_type(&mut self, t: &TypeExpr, env: &Env) -> Res<MType> {
        self.enter()?;
        let r = self.eval_type_inner(t, env);
        self.depth = self.depth.saturating_sub(1);
        r
    }

    fn eval_type_inner(&mut self, t: &TypeExpr, env: &Env) -> Res<MType> {
        Ok(match t {
            TypeExpr::Prim(t) => t.clone(),
            TypeExpr::Nullable(t) => self.eval_type(t, env)?.nullable(),
            TypeExpr::List(t) => {
                // The item type is checked but not kept: list types only matter for `is`.
                self.eval_type(t, env)?;
                MType::prim(Kind::List)
            }
            TypeExpr::Record(fields) => {
                let mut out = Vec::with_capacity(fields.len());
                for (n, t) in fields {
                    out.push((n.clone(), self.eval_type(t, env)?));
                }
                MType {
                    fields: Some(Rc::new(out)),
                    ..MType::prim(Kind::Record)
                }
            }
            TypeExpr::Table(row) => {
                let row = self.eval_type(row, env)?;
                match (row.kind, row.fields) {
                    (Kind::Record, Some(f)) => MType::table(f.to_vec()),
                    (Kind::Table, Some(f)) => MType::table(f.to_vec()),
                    _ => return expr_err("A table type needs a record type for its rows."),
                }
            }
            TypeExpr::Expr(e) => match self.eval(e, env)? {
                Value::Type(t) => (*t).clone(),
                v => {
                    return expr_err(format!(
                        "We cannot convert the value {} to type Type.",
                        self.show(&v)
                    ));
                }
            },
        })
    }

    /// Invokes a function value.
    pub fn call(&mut self, f: &Value, args: Vec<Value>) -> Res<Value> {
        let Value::Function(func) = f else {
            return expr_err(format!(
                "We cannot convert the value {} to type Function.",
                self.show(f)
            ));
        };
        self.enter()?;
        let r = self.call_inner(func, args);
        self.depth = self.depth.saturating_sub(1);
        r
    }

    fn call_inner(&mut self, func: &Func, mut args: Vec<Value>) -> Res<Value> {
        self.tick(1)?;
        match func {
            Func::Closure { params, body, env } => {
                let required = params.iter().filter(|p| !p.optional).count();
                if args.len() < required || args.len() > params.len() {
                    return expr_err(format!(
                        "{} arguments were passed to a function which expects {}.",
                        args.len(),
                        arity_text(required, params.len())
                    ));
                }
                args.resize(params.len(), Value::Null);
                let frame = Rc::new(Frame {
                    names: params.iter().map(|p: &Param| p.name.clone()).collect(),
                    slots: args.into_iter().map(|v| RefCell::new(Slot::Done(v))).collect(),
                    parent: env.clone(),
                });
                self.eval(body, &Some(frame))
            }
            Func::Builtin(b) => {
                if args.len() < b.min || args.len() > b.max {
                    return expr_err(format!(
                        "{} arguments were passed to {}, which expects {}.",
                        args.len(),
                        b.name,
                        arity_text(b.min, b.max)
                    ));
                }
                args.resize(b.max, Value::Null);
                if super::remote::has_remote(&args) {
                    if let Some(v) = super::remote::fold(self, b.name, &args)? {
                        return Ok(v);
                    }
                    super::remote::force_args(self, &mut args)?;
                }
                (b.f)(self, &args)
            }
            Func::Native(n) => {
                super::remote::force_args(self, &mut args)?;
                lib::call_native(self, n, args)
            }
            Func::Connector(name) => super::remote::open(self, name, &args),
        }
    }

    /// Number of parameters a function value takes (for `Table.Sort` comparers and the like).
    pub fn arity(f: &Value) -> usize {
        match f {
            Value::Function(func) => match &**func {
                Func::Closure { params, .. } => params.len(),
                Func::Builtin(b) => b.max,
                Func::Native(_) => 1,
                Func::Connector(_) => 3,
            },
            _ => 0,
        }
    }

    pub fn field(&mut self, v: &Value, name: &str, opt: bool) -> Res<Value> {
        if let Value::Remote(r) = v {
            let t = Value::Table(super::remote::force(self, r)?);
            return self.field(&t, name, opt);
        }
        match v {
            Value::Record(r) => match r.get(name) {
                Some(v) => v.clone().check(),
                None if opt => Ok(Value::Null),
                None => expr_err(format!("The field '{name}' of the record wasn't found.")),
            },
            Value::Table(t) => match t.index(name) {
                Some(i) => {
                    self.tick(t.rows.len() as u64)?;
                    Ok(Value::list(
                        t.rows
                            .iter()
                            .map(|r| r.get(i).cloned().unwrap_or(Value::Null))
                            .collect(),
                    ))
                }
                None if opt => Ok(Value::Null),
                None => expr_err(format!("The column '{name}' of the table wasn't found.")),
            },
            other => expr_err(format!(
                "We cannot apply field access to the type {}.",
                title(other.type_name())
            )),
        }
    }

    fn project(&mut self, v: &Value, names: &[Rc<str>], opt: bool) -> Res<Value> {
        if let Value::Remote(r) = v {
            let t = Value::Table(super::remote::force(self, r)?);
            return self.project(&t, names, opt);
        }
        match v {
            Value::Record(r) => {
                let mut out = Vec::with_capacity(names.len());
                for n in names {
                    match r.get(n) {
                        Some(v) => out.push((n.clone(), v.clone())),
                        None if opt => out.push((n.clone(), Value::Null)),
                        None => return expr_err(format!("The field '{n}' of the record wasn't found.")),
                    }
                }
                Ok(Value::Record(Rc::new(Record::new(out))))
            }
            Value::Table(t) => {
                let names: Vec<Value> = names.iter().map(|n| Value::Text(n.clone())).collect();
                let missing = if opt { 2.0 } else { 0.0 };
                lib::table::select_columns(self, t, &Value::list(names), &Value::num(missing))
            }
            other => expr_err(format!(
                "We cannot apply field access to the type {}.",
                title(other.type_name())
            )),
        }
    }

    pub fn item(&mut self, v: &Value, i: &Value, opt: bool) -> Res<Value> {
        if let Value::Remote(r) = v {
            if let Value::Record(key) = i
                && let Some(row) = super::remote::lookup(self, r, key)?
            {
                return Ok(row);
            }
            let t = Value::Table(super::remote::force(self, r)?);
            return self.item(&t, i, opt);
        }
        match (v, i) {
            (Value::List(l), Value::Number(n)) => {
                let idx = index_of(n.f())?;
                match idx.and_then(|k| l.get(k)) {
                    Some(v) => v.clone().check(),
                    None if opt => Ok(Value::Null),
                    None => expr_err(
                        "There weren't enough elements in the enumeration to complete the operation.",
                    ),
                }
            }
            (Value::Table(t), Value::Number(n)) => {
                let idx = index_of(n.f())?;
                match idx.and_then(|k| t.rows.get(k)) {
                    Some(r) => Ok(row_record(t, r)),
                    None if opt => Ok(Value::Null),
                    None => expr_err(
                        "There weren't enough elements in the enumeration to complete the operation.",
                    ),
                }
            }
            (Value::Table(t), Value::Record(key)) => {
                let mut cols = Vec::with_capacity(key.names.len());
                for n in key.names.iter() {
                    match t.index(n) {
                        Some(c) => cols.push(c),
                        None => return expr_err(format!("The column '{n}' of the table wasn't found.")),
                    }
                }
                let mut hit: Option<&Vec<Value>> = None;
                for r in &t.rows {
                    self.tick(1)?;
                    let matches = cols
                        .iter()
                        .zip(key.values.iter())
                        .all(|(c, kv)| r.get(*c).is_some_and(|cell| values_equal(cell, kv)));
                    if matches {
                        if hit.is_some() {
                            return expr_err("The key matched more than one row in the table.");
                        }
                        hit = Some(r);
                    }
                }
                match hit {
                    Some(r) => Ok(row_record(t, r)),
                    None if opt => Ok(Value::Null),
                    None => expr_err("The key didn't match any rows in the table."),
                }
            }
            (a, b) => expr_err(format!(
                "We cannot apply operator {{}} to types {} and {}.",
                title(a.type_name()),
                title(b.type_name())
            )),
        }
    }

    fn range(&mut self, a: &Value, b: &Value, out: &mut Vec<Value>) -> Res<()> {
        // A range of single characters (`{"a".."z"}`, as in Learn's `Text.Select` example).
        if let (Value::Text(x), Value::Text(y)) = (a, b) {
            let one = |t: &str| {
                let mut it = t.chars();
                it.next().filter(|_| it.next().is_none())
            };
            let (Some(x), Some(y)) = (one(x), one(y)) else {
                return expr_err("A list range of texts needs single characters.");
            };
            let n = (u32::from(y).saturating_sub(u32::from(x)) as usize).saturating_add(1);
            if y < x {
                return Ok(());
            }
            self.check_rows(out.len().saturating_add(n))?;
            self.tick(n as u64)?;
            out.extend((x..=y).map(|c| Value::text(c.to_string())));
            return Ok(());
        }
        let (Value::Number(a), Value::Number(b)) = (a, b) else {
            return expr_err("A list range needs whole numbers.");
        };
        let (Some(a), Some(b)) = (whole(a.f()), whole(b.f())) else {
            return expr_err("A list range needs whole numbers.");
        };
        if b < a {
            return Ok(());
        }
        let n = usize::try_from(b.saturating_sub(a).saturating_add(1)).unwrap_or(usize::MAX);
        self.check_rows(out.len().saturating_add(n))?;
        self.tick(n as u64)?;
        let mut k = a;
        while k <= b {
            out.push(Value::Number(Num::int(i128::from(k))));
            k += 1;
        }
        Ok(())
    }

    fn unary(&mut self, op: UnOp, v: Value) -> Res<Value> {
        match (op, v) {
            (_, Value::Null) => Ok(Value::Null),
            (UnOp::Plus, v @ (Value::Number(_) | Value::Duration(_))) => Ok(v),
            // Negation keeps a decimal a decimal.
            (UnOp::Neg, Value::Number(n)) => Ok(Value::Number(n.neg())),
            (UnOp::Neg, Value::Duration(d)) => Ok(Value::Duration(d.saturating_neg())),
            (UnOp::Not, Value::Logical(b)) => Ok(Value::Logical(!b)),
            (op, v) => expr_err(format!(
                "We cannot apply operator {} to type {}.",
                match op {
                    UnOp::Plus => "+",
                    UnOp::Neg => "-",
                    UnOp::Not => "not",
                },
                title(v.type_name())
            )),
        }
    }

    fn op_error<T>(&self, op: &str, a: &Value, b: &Value) -> Res<T> {
        expr_err(format!(
            "We cannot apply operator {op} to types {} and {}.",
            title(a.type_name()),
            title(b.type_name())
        ))
    }

    pub fn binary(&mut self, op: BinOp, a: Value, b: Value) -> Res<Value> {
        use Value as V;
        let a = super::remote::force_value(self, a)?;
        let b = super::remote::force_value(self, b)?;
        let sym = match op {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Concat => "&",
            BinOp::Eq => "=",
            BinOp::Ne => "<>",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "and",
            BinOp::Or => "or",
            BinOp::Coalesce => "??",
        };
        match op {
            BinOp::Eq => return Ok(V::Logical(values_equal(&a, &b))),
            BinOp::Ne => return Ok(V::Logical(!values_equal(&a, &b))),
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                if a.is_null() || b.is_null() {
                    return Ok(V::Null);
                }
                if let (V::Number(x), V::Number(y)) = (&a, &b)
                    && (x.f().is_nan() || y.f().is_nan())
                {
                    return Ok(V::Logical(false));
                }
                let Some(o) = compare_same(&a, &b) else {
                    return self.op_error(sym, &a, &b);
                };
                return Ok(V::Logical(match op {
                    BinOp::Lt => o == Ordering::Less,
                    BinOp::Le => o != Ordering::Greater,
                    BinOp::Gt => o == Ordering::Greater,
                    _ => o != Ordering::Less,
                }));
            }
            _ => {}
        }
        if a.is_null() || b.is_null() {
            // Operators propagate null; `&` on lists/records with null is an error in M,
            // but null & text is null.
            return Ok(V::Null);
        }
        let r = match (op, &a, &b) {
            // The arithmetic operators compute in double precision (M specification).
            (BinOp::Add, V::Number(x), V::Number(y)) => V::num(x.f() + y.f()),
            (BinOp::Sub, V::Number(x), V::Number(y)) => V::num(x.f() - y.f()),
            (BinOp::Mul, V::Number(x), V::Number(y)) => V::num(x.f() * y.f()),
            (BinOp::Div, V::Number(x), V::Number(y)) => V::num(x.f() / y.f()),
            (BinOp::Concat, V::Text(x), V::Text(y)) => {
                self.check_text(x.len().saturating_add(y.len()))?;
                let mut s = String::with_capacity(x.len() + y.len());
                s.push_str(x);
                s.push_str(y);
                V::text(s)
            }
            (BinOp::Concat, V::List(x), V::List(y)) => {
                self.check_rows(x.len().saturating_add(y.len()))?;
                self.tick((x.len() + y.len()) as u64)?;
                let mut v = Vec::with_capacity(x.len() + y.len());
                v.extend(x.iter().cloned());
                v.extend(y.iter().cloned());
                V::list(v)
            }
            (BinOp::Concat, V::Record(x), V::Record(y)) => {
                let mut fields: Vec<(Rc<str>, Value)> =
                    x.names.iter().cloned().zip(x.values.iter().cloned()).collect();
                for (n, v) in y.names.iter().zip(y.values.iter()) {
                    match fields.iter_mut().find(|(m, _)| m == n) {
                        Some(slot) => slot.1 = v.clone(),
                        None => fields.push((n.clone(), v.clone())),
                    }
                }
                V::Record(Rc::new(Record::new(fields)))
            }
            (BinOp::Concat, V::Table(x), V::Table(y)) => lib::table::combine(self, &[x.clone(), y.clone()])?,
            (BinOp::Concat, V::Date(d), V::Time(t)) => V::DateTime(date_ms(*d)?.saturating_add(*t)),
            (BinOp::Add, V::Date(d), V::Duration(x)) | (BinOp::Add, V::Duration(x), V::Date(d)) => {
                V::Date(d.saturating_add(x.div_euclid(MS_PER_DAY)))
            }
            (BinOp::Sub, V::Date(d), V::Duration(x)) => V::Date(d.saturating_sub(x.div_euclid(MS_PER_DAY))),
            (BinOp::Add, V::DateTime(d), V::Duration(x)) | (BinOp::Add, V::Duration(x), V::DateTime(d)) => {
                V::DateTime(d.saturating_add(*x))
            }
            (BinOp::Sub, V::DateTime(d), V::Duration(x)) => V::DateTime(d.saturating_sub(*x)),
            (BinOp::Add, V::DateTimeZone(d, o), V::Duration(x))
            | (BinOp::Add, V::Duration(x), V::DateTimeZone(d, o)) => {
                V::DateTimeZone(d.saturating_add(*x), *o)
            }
            (BinOp::Sub, V::DateTimeZone(d, o), V::Duration(x)) => V::DateTimeZone(d.saturating_sub(*x), *o),
            (BinOp::Add, V::Time(t), V::Duration(x)) | (BinOp::Add, V::Duration(x), V::Time(t)) => {
                V::Time(t.saturating_add(*x).rem_euclid(MS_PER_DAY))
            }
            (BinOp::Sub, V::Time(t), V::Duration(x)) => V::Time(t.saturating_sub(*x).rem_euclid(MS_PER_DAY)),
            (BinOp::Add, V::Duration(x), V::Duration(y)) => V::Duration(x.saturating_add(*y)),
            (BinOp::Sub, V::Duration(x), V::Duration(y)) => V::Duration(x.saturating_sub(*y)),
            (BinOp::Sub, V::Date(x), V::Date(y)) => {
                V::Duration(x.saturating_sub(*y).saturating_mul(MS_PER_DAY))
            }
            (BinOp::Sub, V::DateTime(x), V::DateTime(y)) => V::Duration(x.saturating_sub(*y)),
            (BinOp::Sub, V::Time(x), V::Time(y)) => V::Duration(x.saturating_sub(*y)),
            (BinOp::Sub, V::DateTimeZone(x, ox), V::DateTimeZone(y, oy)) => {
                let ux = x.saturating_sub(i64::from(*ox) * 60_000);
                let uy = y.saturating_sub(i64::from(*oy) * 60_000);
                V::Duration(ux.saturating_sub(uy))
            }
            (BinOp::Mul, V::Duration(x), V::Number(n)) | (BinOp::Mul, V::Number(n), V::Duration(x)) => {
                V::Duration(ms_from_f64(*x as f64 * n.f())?)
            }
            (BinOp::Div, V::Duration(x), V::Number(n)) => V::Duration(ms_from_f64(*x as f64 / n.f())?),
            (BinOp::Div, V::Duration(x), V::Duration(y)) => V::num(*x as f64 / *y as f64),
            _ => return self.op_error(sym, &a, &b),
        };
        in_range(r)
    }

    /// A short rendering of a value for error messages.
    pub fn show(&self, v: &Value) -> String {
        match v {
            Value::Text(t) => format!("\"{}\"", crate::value::truncate(t)),
            Value::Null => "null".into(),
            Value::List(_)
            | Value::Record(_)
            | Value::Table(_)
            | Value::Remote(_)
            | Value::Function(_)
            | Value::Type(_) => {
                format!("of type {}", title(v.type_name()))
            }
            other => super::convert::to_text(other, &self.culture).unwrap_or_else(|| v.type_name().into()),
        }
    }
}

/// OLE day numbers of 0001-01-01 and 9999-12-31.
pub(crate) const MIN_DAY: i64 = -657_434;
pub(crate) const MAX_DAY: i64 = 2_958_465;

/// Rejects dates and datetimes outside years 1–9999 (so later arithmetic on
/// them cannot overflow).
pub(crate) fn in_range(v: Value) -> Res<Value> {
    let ok = match &v {
        Value::Date(d) => (MIN_DAY..=MAX_DAY).contains(d),
        Value::DateTime(ms) | Value::DateTimeZone(ms, _) => {
            (MIN_DAY * MS_PER_DAY..(MAX_DAY + 1) * MS_PER_DAY).contains(ms)
        }
        _ => true,
    };
    if ok {
        Ok(v)
    } else {
        err("Expression.Error", "The resulting date or time is out of range.")
    }
}

pub(crate) fn arity_text(min: usize, max: usize) -> String {
    if min == max {
        format!("{min}")
    } else {
        format!("between {min} and {max}")
    }
}

/// `Logical` from `logical`, … (for error messages).
pub(crate) fn title(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

pub(crate) fn error_record(e: &MError) -> Value {
    Value::record(vec![
        ("Reason", Value::text(e.reason.as_str())),
        ("Message", Value::text(e.message.as_str())),
        ("Detail", e.detail.clone()),
    ])
}

pub(crate) fn row_record(t: &Table, row: &[Value]) -> Value {
    Value::Record(Rc::new(Record {
        names: t.names(),
        values: row.to_vec(),
    }))
}

fn index_of(n: f64) -> Res<Option<usize>> {
    if n.fract() != 0.0 || n.is_nan() {
        return expr_err("An index must be a whole number.");
    }
    if n < 0.0 {
        return expr_err("An index cannot be negative.");
    }
    Ok(if n < 1e15 { Some(n as usize) } else { None })
}

/// A whole number that fits comfortably in i64.
pub(crate) fn whole(n: f64) -> Option<i64> {
    (n.is_finite() && n.fract() == 0.0 && n.abs() < 9.0e15).then_some(n as i64)
}

pub(crate) fn ms_from_f64(x: f64) -> Res<i64> {
    let r = x.round();
    if r.is_finite() && r.abs() < 9.0e18 {
        Ok(r as i64)
    } else {
        err("Expression.Error", "The duration is out of range.")
    }
}

pub(crate) fn date_ms(d: i64) -> Res<i64> {
    d.checked_mul(MS_PER_DAY)
        .map_or_else(|| err("Expression.Error", "The date is out of range."), Ok)
}

/// M equality (`=`): structural for lists and records, exact for text.
pub(crate) fn values_equal(a: &Value, b: &Value) -> bool {
    use Value as V;
    match (a, b) {
        (V::Null, V::Null) => true,
        (V::Logical(x), V::Logical(y)) => x == y,
        // Numbers compare as doubles: 123456789012345678 = 123456789012345679.
        (V::Number(x), V::Number(y)) => x.f() == y.f(),
        (V::Text(x), V::Text(y)) => x == y,
        (V::Date(x), V::Date(y)) | (V::DateTime(x), V::DateTime(y)) => x == y,
        (V::Time(x), V::Time(y)) | (V::Duration(x), V::Duration(y)) => x == y,
        (V::DateTimeZone(x, ox), V::DateTimeZone(y, oy)) => {
            x.saturating_sub(i64::from(*ox) * 60_000) == y.saturating_sub(i64::from(*oy) * 60_000)
        }
        (V::Binary(x), V::Binary(y)) => match (x.get(), y.get()) {
            (Ok(p), Ok(q)) => p == q,
            _ => false,
        },
        (V::List(x), V::List(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| values_equal(p, q))
        }
        (V::Record(x), V::Record(y)) => {
            x.names.len() == y.names.len()
                && x.names
                    .iter()
                    .zip(x.values.iter())
                    .all(|(n, v)| y.get(n).is_some_and(|w| values_equal(v, w)))
        }
        (V::Table(x), V::Table(y)) => {
            x.columns.len() == y.columns.len()
                && x.rows.len() == y.rows.len()
                && x.columns
                    .iter()
                    .zip(y.columns.iter())
                    .all(|(p, q)| p.name == q.name)
                && x.rows
                    .iter()
                    .zip(y.rows.iter())
                    .all(|(p, q)| p.iter().zip(q.iter()).all(|(c, d)| values_equal(c, d)))
        }
        (V::Function(x), V::Function(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

/// Ordinal comparison by UTF-16 code units (the .NET ordinal order).
pub(crate) fn cmp_text(a: &str, b: &str, ignore_case: bool) -> Ordering {
    if ignore_case {
        let a = a.to_uppercase();
        let b = b.to_uppercase();
        a.encode_utf16().cmp(b.encode_utf16())
    } else {
        a.encode_utf16().cmp(b.encode_utf16())
    }
}

/// Comparison of two values of the same kind (relational operators).
pub(crate) fn compare_same(a: &Value, b: &Value) -> Option<Ordering> {
    use Value as V;
    Some(match (a, b) {
        (V::Number(x), V::Number(y)) => x.f().partial_cmp(&y.f())?,
        (V::Text(x), V::Text(y)) => cmp_text(x, y, false),
        (V::Logical(x), V::Logical(y)) => x.cmp(y),
        (V::Date(x), V::Date(y)) | (V::DateTime(x), V::DateTime(y)) => x.cmp(y),
        (V::Time(x), V::Time(y)) | (V::Duration(x), V::Duration(y)) => x.cmp(y),
        (V::DateTimeZone(x, ox), V::DateTimeZone(y, oy)) => x
            .saturating_sub(i64::from(*ox) * 60_000)
            .cmp(&y.saturating_sub(i64::from(*oy) * 60_000)),
        _ => return None,
    })
}

/// Total order used by sorting, `List.Min`/`Max` and grouping: null first,
/// then values of the same kind; different kinds order by kind.
pub(crate) fn compare_values(a: &Value, b: &Value, ignore_case: bool) -> Ordering {
    match (a, b) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) => Ordering::Less,
        (_, Value::Null) => Ordering::Greater,
        (Value::Text(x), Value::Text(y)) if ignore_case => cmp_text(x, y, true),
        (Value::Number(x), Value::Number(y)) => x.f().total_cmp(&y.f()),
        _ => compare_same(a, b).unwrap_or_else(|| (a.kind() as u8).cmp(&(b.kind() as u8))),
    }
}

/// Whether a value is compatible with a type (`is`, `as`).
pub(crate) fn conforms(v: &Value, t: &MType) -> bool {
    if v.is_null() {
        return t.nullable || matches!(t.kind, Kind::Any | Kind::Null);
    }
    match t.kind {
        Kind::Any => true,
        Kind::AnyNonNull => true,
        Kind::None | Kind::Null => false,
        k => v.kind() == k,
    }
}
