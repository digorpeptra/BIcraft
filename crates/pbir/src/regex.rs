//! A small regular-expression matcher for JSON Schema `pattern` and `patternProperties`.
//!
//! JSON Schema patterns use ECMA-262 syntax and are not anchored: a pattern matches when it
//! matches anywhere in the string. This module supports the common subset:
//!
//! - literals, `.`, `^`, `$`, `\b`, `\B`;
//! - classes `[...]`/`[^...]` with ranges, and `\d \D \w \W \s \S` inside or outside them;
//! - escapes `\n \r \t \v \f \0 \xHH \uHHHH` and escaped punctuation;
//! - groups `(...)`, `(?:...)`, `(?<name>...)`, lookaheads `(?=...)` and `(?!...)`;
//! - alternation `|` and quantifiers `* + ? {n} {n,} {n,m}`, greedy or lazy.
//!
//! Anything else (backreferences, lookbehind, `\p{…}`, unknown letter escapes) fails to compile;
//! callers treat such a pattern as unknown and let every string pass.
//!
//! Matching runs a Pike VM: an NFA simulation over the characters, with no backtracking and no
//! recursion over the input, so time is O(length × program) and the stack stays flat. A
//! lookahead runs a nested VM at the current position (results memoized per position); lookahead
//! nesting is bounded by the parser.

use std::collections::HashMap;

/// Most instructions in one compiled pattern (all programs together).
const MAX_PROGRAM: usize = 20_000;
/// Largest `{n,m}` bound.
const MAX_REPEAT: u32 = 1_000;
/// Deepest group nesting in a pattern.
const MAX_NESTING: u32 = 64;

/// A compiled pattern.
#[derive(Debug)]
pub struct Regex {
    /// `progs[0]` is the pattern; the others are lookahead bodies.
    progs: Vec<Vec<Inst>>,
    /// Letters match regardless of case (simple one-to-one case mapping).
    ignore_case: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Builtin {
    Digit,
    Word,
    Space,
}

impl Builtin {
    fn matches(self, c: char) -> bool {
        match self {
            Builtin::Digit => c.is_ascii_digit(),
            Builtin::Word => c.is_ascii_alphanumeric() || c == '_',
            Builtin::Space => matches!(
                c,
                '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
                    ..='\u{200a}'
                        | '\u{2028}'
                        | '\u{2029}'
                        | '\u{202f}'
                        | '\u{205f}'
                        | '\u{3000}'
                        | '\u{feff}'
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum ClassItem {
    Range(char, char),
    Builtin(Builtin, bool),
}

#[derive(Debug, Clone, PartialEq)]
struct Class {
    items: Vec<ClassItem>,
    negated: bool,
}

impl Class {
    fn matches(&self, c: char) -> bool {
        let hit = self.items.iter().any(|i| match *i {
            ClassItem::Range(a, b) => a <= c && c <= b,
            ClassItem::Builtin(b, neg) => b.matches(c) != neg,
        });
        hit != self.negated
    }

    /// [`Class::matches`] ignoring case: an item matches when it matches `c` or one of its
    /// one-character case variants; negation applies after that.
    fn matches_ignore_case(&self, c: char) -> bool {
        let variants = [c, fold_lower(c), fold_upper(c)];
        let hit = self.items.iter().any(|i| {
            variants.iter().any(|&v| match *i {
                ClassItem::Range(a, b) => a <= v && v <= b,
                ClassItem::Builtin(b, neg) => b.matches(v) != neg,
            })
        });
        hit != self.negated
    }
}

/// The lowercase of `c` when it is a single character, else `c`.
fn fold_lower(c: char) -> char {
    let mut it = c.to_lowercase();
    match (it.next(), it.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

/// The uppercase of `c` when it is a single character, else `c`.
fn fold_upper(c: char) -> char {
    let mut it = c.to_uppercase();
    match (it.next(), it.next()) {
        (Some(u), None) => u,
        _ => c,
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Ast {
    Empty,
    Char(char),
    Any,
    Class(Class),
    Start,
    End,
    WordBoundary(bool),
    Look(Box<Ast>, bool),
    Concat(Vec<Ast>),
    Alt(Vec<Ast>),
    Repeat(Box<Ast>, u32, Option<u32>),
}

#[derive(Debug)]
enum Inst {
    Char(char),
    Any,
    Class(Box<Class>),
    Split(usize, usize),
    Jmp(usize),
    Start,
    End,
    WordBoundary(bool),
    Look { prog: usize, negate: bool },
    Match,
}

fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
    depth: u32,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos.checked_add(offset)?).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        Some(c)
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn alternation(&mut self) -> Option<Ast> {
        let mut branches = vec![self.concat()?];
        while self.eat('|') {
            branches.push(self.concat()?);
        }
        Some(if branches.len() == 1 {
            branches.pop()?
        } else {
            Ast::Alt(branches)
        })
    }

    fn concat(&mut self) -> Option<Ast> {
        let mut items = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom()?;
            let atom = self.quantified(atom)?;
            items.push(atom);
        }
        Some(match items.len() {
            0 => Ast::Empty,
            1 => items.pop()?,
            _ => Ast::Concat(items),
        })
    }

    /// `{n}`, `{n,}` or `{n,m}` at the cursor; `None` (cursor unchanged) when it is not one.
    fn braces(&mut self) -> Option<(u32, Option<u32>)> {
        let start = self.pos;
        let number = |p: &mut Self| -> Option<u32> {
            let begin = p.pos;
            let mut n: u32 = 0;
            while let Some(d) = p.peek().and_then(|c| c.to_digit(10)) {
                n = n.checked_mul(10)?.checked_add(d)?;
                p.pos += 1;
            }
            (p.pos > begin).then_some(n)
        };
        let parsed = (|| {
            if !self.eat('{') {
                return None;
            }
            let min = number(self)?;
            let max = if self.eat(',') {
                if self.peek() == Some('}') {
                    None
                } else {
                    Some(number(self)?)
                }
            } else {
                Some(min)
            };
            self.eat('}').then_some((min, max))
        })();
        if parsed.is_none() {
            self.pos = start;
        }
        parsed
    }

    fn quantified(&mut self, atom: Ast) -> Option<Ast> {
        let (min, max) = match self.peek() {
            Some('*') => {
                self.pos += 1;
                (0, None)
            }
            Some('+') => {
                self.pos += 1;
                (1, None)
            }
            Some('?') => {
                self.pos += 1;
                (0, Some(1))
            }
            Some('{') => match self.braces() {
                Some(b) => b,
                None => return Some(atom),
            },
            _ => return Some(atom),
        };
        if max.is_some_and(|m| m < min) || min > MAX_REPEAT || max.is_some_and(|m| m > MAX_REPEAT) {
            return None;
        }
        // Lazy and greedy quantifiers accept the same strings.
        self.eat('?');
        if matches!(atom, Ast::Start | Ast::End | Ast::WordBoundary(_) | Ast::Empty) {
            return None;
        }
        if matches!(self.peek(), Some('*' | '+' | '?')) {
            return None;
        }
        Some(Ast::Repeat(Box::new(atom), min, max))
    }

    fn atom(&mut self) -> Option<Ast> {
        let c = self.bump()?;
        Some(match c {
            '.' => Ast::Any,
            '^' => Ast::Start,
            '$' => Ast::End,
            '(' => {
                self.depth += 1;
                if self.depth > MAX_NESTING {
                    return None;
                }
                let look = if self.peek() == Some('?') {
                    match (self.peek_at(1), self.peek_at(2)) {
                        (Some(':'), _) => {
                            self.pos += 2;
                            None
                        }
                        (Some('='), _) => {
                            self.pos += 2;
                            Some(false)
                        }
                        (Some('!'), _) => {
                            self.pos += 2;
                            Some(true)
                        }
                        (Some('<'), Some(n)) if n != '=' && n != '!' => {
                            // Named group: matching is the same as for a plain group.
                            self.pos += 2;
                            while let Some(ch) = self.bump() {
                                if ch == '>' {
                                    break;
                                }
                                if !(ch.is_alphanumeric() || ch == '_' || ch == '$') {
                                    return None;
                                }
                            }
                            None
                        }
                        _ => return None,
                    }
                } else {
                    None
                };
                let inner = self.alternation()?;
                if !self.eat(')') {
                    return None;
                }
                self.depth -= 1;
                match look {
                    Some(negate) => Ast::Look(Box::new(inner), negate),
                    None => inner,
                }
            }
            ')' => return None,
            '[' => Ast::Class(self.class()?),
            '\\' => self.escape(false)?,
            '*' | '+' | '?' => return None,
            '{' => {
                // A brace that does not start a quantifier is a literal (Annex B).
                self.pos -= 1;
                if self.braces().is_some() {
                    return None;
                }
                self.pos += 1;
                Ast::Char('{')
            }
            other => Ast::Char(other),
        })
    }

    fn hex(&mut self, n: usize) -> Option<char> {
        let mut v: u32 = 0;
        for _ in 0..n {
            v = v * 16 + self.bump()?.to_digit(16)?;
        }
        char::from_u32(v)
    }

    /// After a `\`. In a class, `\b` is a backspace and the assertions are not allowed.
    fn escape(&mut self, in_class: bool) -> Option<Ast> {
        let c = self.bump()?;
        Some(match c {
            'd' => Ast::Class(builtin(Builtin::Digit, false)),
            'D' => Ast::Class(builtin(Builtin::Digit, true)),
            'w' => Ast::Class(builtin(Builtin::Word, false)),
            'W' => Ast::Class(builtin(Builtin::Word, true)),
            's' => Ast::Class(builtin(Builtin::Space, false)),
            'S' => Ast::Class(builtin(Builtin::Space, true)),
            'b' if in_class => Ast::Char('\u{8}'),
            'b' => Ast::WordBoundary(true),
            'B' if !in_class => Ast::WordBoundary(false),
            'n' => Ast::Char('\n'),
            'r' => Ast::Char('\r'),
            't' => Ast::Char('\t'),
            'v' => Ast::Char('\u{b}'),
            'f' => Ast::Char('\u{c}'),
            '0' if !self.peek().is_some_and(|d| d.is_ascii_digit()) => Ast::Char('\0'),
            'x' => Ast::Char(self.hex(2)?),
            'u' => {
                let hi = self.hex(4)?;
                Ast::Char(hi)
            }
            // Backreferences, `\k<name>`, `\p{…}`, `\c`, other letters: unsupported.
            c if c.is_ascii_alphanumeric() => return None,
            c => Ast::Char(c),
        })
    }

    fn class_atom(&mut self) -> Option<ClassItem> {
        let c = self.bump()?;
        if c != '\\' {
            return Some(ClassItem::Range(c, c));
        }
        match self.escape(true)? {
            Ast::Char(ch) => Some(ClassItem::Range(ch, ch)),
            Ast::Class(class) if class.items.len() == 1 => {
                let item = class.items.into_iter().next()?;
                match item {
                    ClassItem::Builtin(b, neg) => Some(ClassItem::Builtin(b, neg != class.negated)),
                    other => Some(other),
                }
            }
            _ => None,
        }
    }

    fn class(&mut self) -> Option<Class> {
        let negated = self.eat('^');
        let mut items = Vec::new();
        loop {
            if self.peek()? == ']' {
                self.pos += 1;
                break;
            }
            let first = self.class_atom()?;
            if self.peek() == Some('-') && self.peek_at(1).is_some_and(|c| c != ']') {
                self.pos += 1;
                let last = self.class_atom()?;
                match (first, last) {
                    (ClassItem::Range(a, _), ClassItem::Range(b, _)) if a <= b => {
                        items.push(ClassItem::Range(a, b));
                    }
                    _ => return None,
                }
            } else {
                items.push(first);
            }
        }
        Some(Class { items, negated })
    }
}

fn builtin(b: Builtin, negated: bool) -> Class {
    Class {
        items: vec![ClassItem::Builtin(b, false)],
        negated,
    }
}

struct Compiler {
    progs: Vec<Vec<Inst>>,
    size: usize,
}

impl Compiler {
    fn push(&mut self, prog: usize, inst: Inst) -> Option<usize> {
        self.size += 1;
        if self.size > MAX_PROGRAM {
            return None;
        }
        let p = self.progs.get_mut(prog)?;
        p.push(inst);
        Some(p.len() - 1)
    }

    fn len(&self, prog: usize) -> usize {
        self.progs.get(prog).map_or(0, Vec::len)
    }

    fn patch(&mut self, prog: usize, at: usize, inst: Inst) {
        if let Some(slot) = self.progs.get_mut(prog).and_then(|p| p.get_mut(at)) {
            *slot = inst;
        }
    }

    fn emit(&mut self, prog: usize, ast: &Ast) -> Option<()> {
        match ast {
            Ast::Empty => {}
            Ast::Char(c) => {
                self.push(prog, Inst::Char(*c))?;
            }
            Ast::Any => {
                self.push(prog, Inst::Any)?;
            }
            Ast::Class(c) => {
                self.push(prog, Inst::Class(Box::new(c.clone())))?;
            }
            Ast::Start => {
                self.push(prog, Inst::Start)?;
            }
            Ast::End => {
                self.push(prog, Inst::End)?;
            }
            Ast::WordBoundary(b) => {
                self.push(prog, Inst::WordBoundary(*b))?;
            }
            Ast::Look(inner, negate) => {
                let sub = self.progs.len();
                self.progs.push(Vec::new());
                self.emit(sub, inner)?;
                self.push(sub, Inst::Match)?;
                self.push(
                    prog,
                    Inst::Look {
                        prog: sub,
                        negate: *negate,
                    },
                )?;
            }
            Ast::Concat(items) => {
                for i in items {
                    self.emit(prog, i)?;
                }
            }
            Ast::Alt(branches) => {
                let mut jumps = Vec::new();
                let count = branches.len();
                for (i, b) in branches.iter().enumerate() {
                    if i + 1 < count {
                        let split = self.push(prog, Inst::Jmp(0))?;
                        self.emit(prog, b)?;
                        jumps.push(self.push(prog, Inst::Jmp(0))?);
                        let next = self.len(prog);
                        self.patch(prog, split, Inst::Split(split + 1, next));
                    } else {
                        self.emit(prog, b)?;
                    }
                }
                let end = self.len(prog);
                for j in jumps {
                    self.patch(prog, j, Inst::Jmp(end));
                }
            }
            Ast::Repeat(inner, min, max) => {
                for _ in 0..*min {
                    self.emit(prog, inner)?;
                }
                match max {
                    None => {
                        let split = self.push(prog, Inst::Jmp(0))?;
                        self.emit(prog, inner)?;
                        self.push(prog, Inst::Jmp(split))?;
                        let end = self.len(prog);
                        self.patch(prog, split, Inst::Split(split + 1, end));
                    }
                    Some(max) => {
                        let mut splits = Vec::new();
                        for _ in *min..*max {
                            splits.push(self.push(prog, Inst::Jmp(0))?);
                            self.emit(prog, inner)?;
                        }
                        let end = self.len(prog);
                        for s in splits {
                            self.patch(prog, s, Inst::Split(s + 1, end));
                        }
                    }
                }
            }
        }
        Some(())
    }
}

/// A set of program counters with O(1) insert, membership and clear.
struct Threads {
    dense: Vec<usize>,
    sparse: Vec<usize>,
}

impl Threads {
    fn new(n: usize) -> Self {
        Self {
            dense: Vec::with_capacity(n),
            sparse: vec![0; n],
        }
    }

    fn contains(&self, pc: usize) -> bool {
        self.sparse
            .get(pc)
            .and_then(|&i| self.dense.get(i))
            .is_some_and(|&d| d == pc)
    }

    fn insert(&mut self, pc: usize) {
        if let Some(slot) = self.sparse.get_mut(pc) {
            *slot = self.dense.len();
            self.dense.push(pc);
        }
    }

    fn clear(&mut self) {
        self.dense.clear();
    }
}

struct Run<'a> {
    regex: &'a Regex,
    chars: &'a [char],
    looks: HashMap<(usize, usize), bool>,
    look_depth: u32,
}

impl Run<'_> {
    fn is_word(&self, i: Option<usize>) -> bool {
        i.and_then(|i| self.chars.get(i))
            .is_some_and(|c| Builtin::Word.matches(*c))
    }

    fn look(&mut self, prog: usize, pos: usize) -> bool {
        if let Some(r) = self.looks.get(&(prog, pos)) {
            return *r;
        }
        // Lookahead nesting is bounded by the parser's group nesting; this is a second guard.
        let r = if self.look_depth > MAX_NESTING {
            true
        } else {
            self.look_depth += 1;
            let r = self.exec(prog, pos, true);
            self.look_depth -= 1;
            r
        };
        self.looks.insert((prog, pos), r);
        r
    }

    /// Adds `pc` and its epsilon closure at `pos`. True when `Match` is reached.
    fn add(&mut self, prog: usize, list: &mut Threads, pc: usize, pos: usize) -> bool {
        let mut stack = vec![pc];
        while let Some(pc) = stack.pop() {
            if list.contains(pc) {
                continue;
            }
            list.insert(pc);
            let Some(inst) = self.regex.progs.get(prog).and_then(|p| p.get(pc)) else {
                continue;
            };
            match inst {
                Inst::Match => return true,
                Inst::Jmp(t) => stack.push(*t),
                Inst::Split(a, b) => {
                    stack.push(*b);
                    stack.push(*a);
                }
                Inst::Start => {
                    if pos == 0 {
                        stack.push(pc + 1);
                    }
                }
                Inst::End => {
                    if pos == self.chars.len() {
                        stack.push(pc + 1);
                    }
                }
                Inst::WordBoundary(want) => {
                    let at = self.is_word(pos.checked_sub(1)) != self.is_word(Some(pos));
                    if at == *want {
                        stack.push(pc + 1);
                    }
                }
                Inst::Look { prog: sub, negate } => {
                    let (sub, negate) = (*sub, *negate);
                    if self.look(sub, pos) != negate {
                        stack.push(pc + 1);
                    }
                }
                Inst::Char(_) | Inst::Any | Inst::Class(_) => {}
            }
        }
        false
    }

    /// Runs program `prog` from `start`; anchored runs only start there (lookaheads).
    fn exec(&mut self, prog: usize, start: usize, anchored: bool) -> bool {
        let n = self.regex.progs.get(prog).map_or(0, Vec::len);
        let mut clist = Threads::new(n);
        let mut nlist = Threads::new(n);
        let mut pos = start;
        loop {
            if (!anchored || pos == start) && self.add(prog, &mut clist, 0, pos) {
                return true;
            }
            if clist.dense.is_empty() && (anchored || pos >= self.chars.len()) {
                return false;
            }
            let Some(&c) = self.chars.get(pos) else {
                return false;
            };
            let pcs = std::mem::take(&mut clist.dense);
            for &pc in &pcs {
                let Some(inst) = self.regex.progs.get(prog).and_then(|p| p.get(pc)) else {
                    continue;
                };
                let step = match inst {
                    Inst::Char(x) => *x == c || (self.regex.ignore_case && fold_lower(*x) == fold_lower(c)),
                    Inst::Any => !is_line_terminator(c),
                    Inst::Class(class) if self.regex.ignore_case => class.matches_ignore_case(c),
                    Inst::Class(class) => class.matches(c),
                    _ => false,
                };
                if step && self.add(prog, &mut nlist, pc + 1, pos + 1) {
                    return true;
                }
            }
            clist.dense = pcs;
            clist.clear();
            std::mem::swap(&mut clist, &mut nlist);
            pos += 1;
        }
    }
}

impl Regex {
    /// Compiles an ECMA-262 pattern; `None` when it uses unsupported syntax or exceeds the caps.
    pub fn new(pattern: &str) -> Option<Regex> {
        Self::compile(pattern, false)
    }

    /// [`Regex::new`], matching letters regardless of case (like the `i` flag).
    pub fn new_ignore_case(pattern: &str) -> Option<Regex> {
        Self::compile(pattern, true)
    }

    fn compile(pattern: &str, ignore_case: bool) -> Option<Regex> {
        let mut p = Parser {
            chars: pattern.chars().collect(),
            pos: 0,
            depth: 0,
        };
        let ast = p.alternation()?;
        if p.pos != p.chars.len() {
            return None;
        }
        let mut c = Compiler {
            progs: vec![Vec::new()],
            size: 0,
        };
        c.emit(0, &ast)?;
        c.push(0, Inst::Match)?;
        Some(Regex {
            progs: c.progs,
            ignore_case,
        })
    }

    /// True when the pattern matches somewhere in `s`.
    pub fn is_match(&self, s: &str) -> bool {
        let chars: Vec<char> = s.chars().collect();
        let mut run = Run {
            regex: self,
            chars: &chars,
            looks: HashMap::new(),
            look_depth: 0,
        };
        run.exec(0, 0, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(p: &str, s: &str) -> bool {
        Regex::new(p)
            .unwrap_or_else(|| panic!("pattern {p} did not compile"))
            .is_match(s)
    }

    #[test]
    fn vendored_patterns() {
        assert!(m("^[0-9]+$", "12"));
        assert!(!m("^[0-9]+$", "1a"));
        assert!(!m("^[0-9]+$", ""));
        assert!(m("^[0-9]+\\.[0-9]+\\.[0-9]+$", "5.43.0"));
        assert!(!m("^[0-9]+\\.[0-9]+\\.[0-9]+$", "5.43"));
        assert!(!m("^[0-9]+\\.[0-9]+\\.[0-9]+$", "5x43.0"));
        assert!(m("^[1-9][0-9]*\\.(0|[1-9][0-9]*)\\.0$", "10.0.0"));
        assert!(m("^[1-9][0-9]*\\.(0|[1-9][0-9]*)\\.0$", "2.10.0"));
        assert!(!m("^[1-9][0-9]*\\.(0|[1-9][0-9]*)\\.0$", "1.01.0"));
        assert!(!m("^[1-9][0-9]*\\.(0|[1-9][0-9]*)\\.0$", "0.1.0"));
        assert!(!m("^[1-9][0-9]*\\.(0|[1-9][0-9]*)\\.0$", "1.0.1"));
        assert!(m(".*", ""));
        assert!(m("^.+$", "x"));
        assert!(!m("^.+$", ""));
        assert!(!m("^.+$", "a\nb"));
        assert!(!m("^(?!\\*$).+$", "*"));
        assert!(m("^(?!\\*$).+$", "*x"));
        assert!(m("^(?!\\*$).+$", "a"));
        assert!(m("^[A-Za-z0-9]+$", "abC9"));
        assert!(!m("^[A-Za-z0-9]+$", "a-b"));
        let color = "^#[0-9a-fA-F]{8}$|^#(?:[0-9a-fA-F]{3}){1,2}$";
        for (s, ok) in [
            ("#abc", true),
            ("#A1B2C3", true),
            ("#A1B2C3D4", true),
            ("#abcd", false),
            ("#abcde", false),
            ("#A1B2C3D", false),
            ("red", false),
        ] {
            assert_eq!(m(color, s), ok, "{s}");
        }
        let state = "^(selection:(selected|unselected|default)|expansion:(expanded|collapsed)|interaction:(hover|press|rest)|series:(selectall))?(-selection:(selected|unselected|default)|-expansion:(expanded|collapsed)|-interaction:(hover|press|rest)|-series:(selectall))*$|^default$";
        for (s, ok) in [
            ("default", true),
            ("", true),
            ("interaction:hover", true),
            ("selection:selected-interaction:press", true),
            ("-series:selectall", true),
            ("hover", false),
            ("-", false),
        ] {
            assert_eq!(m(state, s), ok, "{s}");
        }
    }

    #[test]
    fn ignore_case() {
        let r = |p: &str| Regex::new_ignore_case(p).unwrap_or_else(|| panic!("{p}"));
        assert!(r(r"iferror\s*\(").is_match("x = IfError (1, 2)"));
        assert!(!m("iferror", "IFERROR"));
        assert!(r("^[a-c]+$").is_match("AbC"));
        assert!(
            !r("^[^a]$").is_match("A"),
            "negated classes exclude every case variant"
        );
        assert!(r("^[^a]$").is_match("b"));
        assert!(r(r"\bKEY$").is_match("customer key"));
        // .NET-style patterns of rule files: a lookahead on a bare `]`, escaped quotes.
        assert!(r(r"x\s*\(\s*.+?(?=])\]\s*,\s*'*T'*\[").is_match("X( 'A'[b], 't'[c]"));
        assert!(r(r"\'*F\'*\[").is_match("'f'["));
    }

    #[test]
    fn unanchored_search_and_syntax() {
        assert!(m("b", "abc"));
        assert!(!m("^b", "abc"));
        assert!(m("c$", "abc"));
        assert!(!m("b$", "abc"));
        assert!(m("a|z", "zz"));
        assert!(m("^(ab)*$", "ababab"));
        assert!(!m("^(ab)*$", "aba"));
        assert!(m("^a{2,3}$", "aaa"));
        assert!(!m("^a{2,3}$", "aaaa"));
        assert!(m("^a{2,}$", "aaaaa"));
        assert!(m("^a{2}?$", "aa"));
        assert!(m("^x{a}$", "x{a}"));
        assert!(m("^\\d\\D\\w\\W\\s\\S$", "1a_ \tq"));
        assert!(m("^[\\d.-]+$", "1.2-3"));
        assert!(m("^[^\\s]+$", "abc"));
        assert!(!m("^[^\\s]+$", "a c"));
        assert!(m("\\bfoo\\b", "a foo b"));
        assert!(!m("\\bfoo\\b", "afoob"));
        assert!(m("^\\u0041\\x42$", "AB"));
        assert!(m("^(?<year>\\d{4})-(?:\\d{2})$", "2024-01"));
        assert!(m("^(?=.*\\d)(?=.*[a-z]).{4,}$", "ab1c"));
        assert!(!m("^(?=.*\\d)(?=.*[a-z]).{4,}$", "abcd"));
        assert!(m("^.$", "é"));
        assert!(m("^(a*)*b$", "aaab"));
    }

    #[test]
    fn unsupported_syntax_does_not_compile() {
        for p in [
            "(a)\\1", "(?<=a)b", "(?<!a)b", "\\p{L}", "\\k<x>", "\\q", "a**", "*a", "(a", "a)", "[a",
            "[z-a]", "a{3,2}", "a{5000}",
        ] {
            assert!(Regex::new(p).is_none(), "{p}");
        }
    }

    #[test]
    fn hostile_inputs_stay_bounded() {
        // Classic catastrophic-backtracking shapes run in linear time here.
        let s = "a".repeat(50_000);
        assert!(!m("^(a+)+$", &format!("{s}!")));
        assert!(!m("^(a|aa)*c$", &s));
        // A long string does not grow the stack.
        let long = "x".repeat(1_000_000);
        assert!(m("^.+$", &long));
        // Nesting and program size are capped.
        let deep = format!("{}a{}", "(".repeat(100), ")".repeat(100));
        assert!(Regex::new(&deep).is_none());
        assert!(Regex::new("(a{1000}){1000}").is_none());
        assert!(Regex::new(&"(".repeat(100_000)).is_none());
    }
}
