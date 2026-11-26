//! L0: compiles a custom visual's TypeScript to JavaScript and bundles its modules into one
//! script, with the swc crates from crates.io (Apache-2.0) and no Node (ROADMAP M11.30,
//! docs/formats/pbiviz.md "Building a visual").
//!
//! The sources come in memory, as a map of project-relative paths (`src/visual.ts`) to text, so
//! the bundler never touches the file system: a relative import resolves only to another entry
//! of the map, never outside the project. Each module is parsed (TypeScript for `.ts`, ES for
//! `.js`), its types are stripped by swc's TypeScript pass (type-only imports such as
//! `powerbi-visuals-api` go with them), and swc's bundler joins the modules from `entry` into
//! one script wrapped in a function (IIFE). A bare package import that survives the strip (an npm
//! package) is refused: there is no `node_modules` to bundle from.
//!
//! The input is hostile (a project folder anyone may hand over): sizes are capped, every error
//! is a `String` naming the file and line, and a panic inside swc becomes an error.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use std::collections::{BTreeMap, HashMap};
use std::panic::{AssertUnwindSafe, catch_unwind};

use swc_bundler::{Bundler, Config, Hook, Load, ModuleData, ModuleRecord, ModuleType, Resolve};
use swc_common::sync::Lrc;
use swc_common::{FileName, GLOBALS, Globals, Mark, SourceMap, Span, Spanned};
use swc_ecma_ast::{EsVersion, KeyValueProp, Module, ModuleDecl, ModuleItem, Program};
use swc_ecma_codegen::Emitter;
use swc_ecma_codegen::text_writer::JsWriter;
use swc_ecma_parser::{EsSyntax, Parser, StringInput, Syntax, TsSyntax};
use swc_ecma_transforms_base::fixer::fixer;
use swc_ecma_transforms_base::helpers::Helpers;
use swc_ecma_transforms_base::resolver;
use swc_ecma_transforms_typescript::strip;

/// Most source files a build reads.
pub const MAX_FILES: usize = 512;
/// Most bytes of one source file.
pub const MAX_FILE_BYTES: usize = 4 << 20;
/// Most bytes of all the sources together.
pub const MAX_TOTAL_BYTES: usize = 16 << 20;

/// The extensions a module may have, in the order an extensionless import tries them.
pub const EXTENSIONS: &[&str] = &[".ts", ".js"];

/// What [`bundle`] made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// The script: every module reached from the entry, in one function.
    pub js: String,
    /// The modules it holds (project paths), sorted.
    pub modules: Vec<String>,
}

/// Whether `path` names a source file the bundler compiles (`.ts` or `.js`, not a `.d.ts`).
pub fn is_source(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    !lower.ends_with(".d.ts") && EXTENSIONS.iter().any(|e| lower.ends_with(e))
}

/// Compiles `entry` and every module it imports from `files` into one script.
///
/// `files` maps project paths (`/`-separated, relative, no `.` or `..` parts) to their text;
/// `entry` is one of them. A relative import (`./util`, `../shared/colors.ts`) resolves against
/// the importing file to an entry of `files` (tried as written, then with `.ts`, `.js`, then as a
/// folder's `index.ts` / `index.js`).
pub fn bundle(files: &BTreeMap<String, String>, entry: &str) -> Result<Output, String> {
    check_sizes(files)?;
    if !files.contains_key(entry) {
        return Err(format!("the entry `{entry}` is not among the sources"));
    }
    let files = files.clone();
    let entry = entry.to_string();
    // swc recurses over the syntax tree and asserts its own invariants: it runs on a thread of
    // its own with a large stack, and a panic there becomes an error.
    let worker = std::thread::Builder::new()
        .name("bicraft-tsbuild".into())
        .stack_size(STACK_BYTES)
        .spawn(move || catch_unwind(AssertUnwindSafe(|| bundle_inner(&files, &entry))));
    let failed = || "the TypeScript compiler (swc) failed on these sources".to_string();
    match worker {
        Ok(handle) => match handle.join() {
            Ok(Ok(r)) => r,
            _ => Err(failed()),
        },
        Err(e) => Err(format!("cannot start the compiler's thread: {e}")),
    }
}

/// The compiler thread's stack.
const STACK_BYTES: usize = 64 << 20;
/// Deepest bracket nesting a source may have (the parser recurses once per level).
pub const MAX_NESTING: usize = 500;

/// The deepest nesting of `(`, `[`, `{` in `text` (strings and comments counted too: an upper
/// bound, cheap and safe).
fn nesting(text: &str) -> usize {
    let (mut depth, mut max) = (0usize, 0usize);
    for b in text.bytes() {
        match b {
            b'(' | b'[' | b'{' => {
                depth += 1;
                max = max.max(depth);
            }
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    max
}

fn check_sizes(files: &BTreeMap<String, String>) -> Result<(), String> {
    if files.len() > MAX_FILES {
        return Err(format!(
            "{} source files: at most {MAX_FILES} are compiled",
            files.len()
        ));
    }
    let mut total = 0usize;
    for (path, text) in files {
        if !valid_path(path) {
            return Err(format!("`{path}` is not a plain relative path"));
        }
        if text.len() > MAX_FILE_BYTES {
            return Err(format!(
                "{path}: {} bytes, at most {MAX_FILE_BYTES} per file",
                text.len()
            ));
        }
        total = total.saturating_add(text.len());
        if nesting(text) > MAX_NESTING {
            return Err(format!(
                "{path}: brackets nested deeper than {MAX_NESTING} levels"
            ));
        }
    }
    if total > MAX_TOTAL_BYTES {
        return Err(format!(
            "{total} bytes of sources: at most {MAX_TOTAL_BYTES} in all"
        ));
    }
    Ok(())
}

/// A `/`-separated relative path without empty, `.` or `..` parts.
fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && path.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
}

/// One module compiled to JavaScript, with the project files it imports.
struct Compiled {
    js: String,
    deps: Vec<String>,
}

fn bundle_inner(files: &BTreeMap<String, String>, entry: &str) -> Result<Output, String> {
    let globals = Globals::new();
    let cm: Lrc<SourceMap> = Lrc::default();
    GLOBALS.set(&globals, || {
        let compiled = compile_all(&cm, files, entry)?;
        check_cycles(&compiled, entry)?;
        let js: BTreeMap<String, String> = compiled.iter().map(|(k, c)| (k.clone(), c.js.clone())).collect();
        let loader = Loader {
            cm: cm.clone(),
            js: &js,
        };
        let resolver = Resolver { files: &js };
        let mut bundler = Bundler::new(
            &globals,
            cm.clone(),
            &loader,
            &resolver,
            Config {
                module: ModuleType::Iife,
                ..Default::default()
            },
            Box::new(NoHook),
        );
        let mut entries = HashMap::new();
        entries.insert("main".to_string(), FileName::Custom(entry.to_string()));
        let bundles = bundler.bundle(entries).map_err(|e| {
            e.chain()
                .last()
                .map_or_else(|| "the bundler failed".into(), |c| c.to_string())
        })?;
        let module = match bundles.into_iter().next() {
            Some(b) => b.module,
            None => return Err("the bundler wrote nothing".into()),
        };
        Ok(Output {
            js: emit(&cm, &module)?,
            modules: compiled.into_keys().collect(),
        })
    })
}

fn ts_syntax() -> Syntax {
    Syntax::Typescript(TsSyntax {
        decorators: true,
        ..Default::default()
    })
}

/// Parses `text` as a module named `name`; the first error names its file and line.
fn parse(cm: &Lrc<SourceMap>, name: &str, text: &str, syntax: Syntax) -> Result<Module, String> {
    let fm = cm.new_source_file(Lrc::new(FileName::Custom(name.to_string())), text.to_string());
    let mut parser = Parser::new(syntax, StringInput::from(&*fm), None);
    let module = parser
        .parse_module()
        .map_err(|e| at(cm, e.span(), &e.kind().msg()))?;
    if let Some(e) = parser.take_errors().into_iter().next() {
        return Err(at(cm, e.span(), &e.kind().msg()));
    }
    Ok(module)
}

/// Compiles every module reached from `entry` to JavaScript (TypeScript stripped of its types)
/// and resolves its imports.
fn compile_all(
    cm: &Lrc<SourceMap>,
    files: &BTreeMap<String, String>,
    entry: &str,
) -> Result<BTreeMap<String, Compiled>, String> {
    let mut out: BTreeMap<String, Compiled> = BTreeMap::new();
    let mut todo = vec![entry.to_string()];
    while let Some(name) = todo.pop() {
        if out.contains_key(&name) {
            continue;
        }
        let text = files
            .get(&name)
            .ok_or_else(|| format!("{name}: no such source file"))?;
        let ts = name.to_ascii_lowercase().ends_with(".ts");
        let syntax = if ts {
            ts_syntax()
        } else {
            Syntax::Es(EsSyntax::default())
        };
        let mut module = parse(cm, &name, text, syntax)?;
        if ts {
            // types, interfaces and type-only imports go
            let unresolved = Mark::new();
            let top_level = Mark::new();
            let program = Program::Module(module)
                .apply(resolver(unresolved, top_level, true))
                .apply(strip(unresolved, top_level))
                .apply(fixer(None));
            module = match program {
                Program::Module(m) => m,
                Program::Script(_) => return Err(format!("{name}: not a module")),
            };
        }
        let mut deps = Vec::new();
        for spec in import_sources(&module) {
            let to = resolve_spec(files, &name, &spec)?;
            if !deps.contains(&to) {
                deps.push(to.clone());
            }
            todo.push(to);
        }
        // written out and read again by the bundler as plain JavaScript: fresh marks
        let js = if ts { emit(cm, &module)? } else { text.clone() };
        out.insert(name, Compiled { js, deps });
    }
    Ok(out)
}

/// The sources of a module's `import … from`, `export … from` and `export * from`.
fn import_sources(module: &Module) -> Vec<String> {
    module
        .body
        .iter()
        .filter_map(|item| match item {
            ModuleItem::ModuleDecl(ModuleDecl::Import(i)) => Some(i.src.value.to_string_lossy().to_string()),
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(e)) => {
                Some(e.src.value.to_string_lossy().to_string())
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(e)) => {
                e.src.as_ref().map(|s| s.value.to_string_lossy().to_string())
            }
            _ => None,
        })
        .collect()
}

/// Resolves import `spec` of `from` to a project file, or says why it cannot be bundled.
fn resolve_spec(files: &BTreeMap<String, String>, from: &str, spec: &str) -> Result<String, String> {
    if !(spec.starts_with("./") || spec.starts_with("../")) {
        return Err(format!(
            "{from}: imports `{spec}`, a package: only the project's own files are bundled \
             (no node_modules); `powerbi-visuals-api` may be imported for its types only"
        ));
    }
    resolve_in(files, from, spec)
        .ok_or_else(|| format!("{from}: imports `{spec}`, which is not a .ts or .js file of the project"))
}

/// Refuses an import cycle (the bundler does not order one), naming it.
fn check_cycles(compiled: &BTreeMap<String, Compiled>, entry: &str) -> Result<(), String> {
    // iterative depth-first search: (module, next dependency index); `path` is the open chain
    let mut done: Vec<&str> = Vec::new();
    let mut path: Vec<(&str, usize)> = vec![(entry, 0)];
    while let Some((name, i)) = path.last().copied() {
        let deps = compiled.get(name).map(|c| c.deps.as_slice()).unwrap_or_default();
        match deps.get(i) {
            None => {
                done.push(name);
                path.pop();
            }
            Some(dep) => {
                if let Some(top) = path.last_mut() {
                    top.1 += 1;
                }
                if let Some(start) = path.iter().position(|(n, _)| n == dep) {
                    let mut chain: Vec<&str> = path.iter().skip(start).map(|(n, _)| *n).collect();
                    chain.push(dep);
                    return Err(format!(
                        "import cycle {}: modules that import each other are not bundled",
                        chain.join(" → ")
                    ));
                }
                if !done.contains(&dep.as_str()) {
                    path.push((dep, 0));
                }
            }
        }
    }
    Ok(())
}

/// Resolves `spec` imported by `from` to a key of `files`.
fn resolve_in<T>(files: &BTreeMap<String, T>, from: &str, spec: &str) -> Option<String> {
    let mut parts: Vec<&str> = from.split('/').collect();
    parts.pop();
    for seg in spec.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    let base = parts.join("/");
    if base.is_empty() {
        return None;
    }
    let mut candidates = vec![base.clone()];
    candidates.extend(EXTENSIONS.iter().map(|e| format!("{base}{e}")));
    candidates.extend(EXTENSIONS.iter().map(|e| format!("{base}/index{e}")));
    candidates
        .into_iter()
        .find(|c| files.contains_key(c) && is_source(c))
}

fn emit(cm: &Lrc<SourceMap>, module: &Module) -> Result<String, String> {
    let mut buf = Vec::new();
    {
        let mut emitter = Emitter {
            cfg: swc_ecma_codegen::Config::default().with_target(EsVersion::Es2020),
            cm: cm.clone(),
            comments: None,
            wr: JsWriter::new(cm.clone(), "\n", &mut buf, None),
        };
        emitter
            .emit_module(module)
            .map_err(|e| format!("can't write the JavaScript: {e}"))?;
    }
    String::from_utf8(buf).map_err(|_| "the JavaScript written is not UTF-8".into())
}

/// `file:line:column: message` for a span of `cm`.
fn at(cm: &SourceMap, span: Span, msg: &str) -> String {
    let loc = cm.lookup_char_pos(span.lo);
    format!(
        "{}:{}:{}: {msg}",
        loc.file.name,
        loc.line,
        loc.col_display.saturating_add(1)
    )
}

/// Hands the bundler the compiled JavaScript of a module.
struct Loader<'a> {
    cm: Lrc<SourceMap>,
    js: &'a BTreeMap<String, String>,
}

impl Load for Loader<'_> {
    fn load(&self, file: &FileName) -> Result<ModuleData, anyhow::Error> {
        let name = match file {
            FileName::Custom(n) => n,
            other => return Err(anyhow::Error::msg(format!("{other}: not a project file"))),
        };
        let text = self
            .js
            .get(name)
            .ok_or_else(|| anyhow::Error::msg(format!("{name}: no such source file")))?;
        let js_name = format!("{name} (compiled)");
        let module =
            parse(&self.cm, &js_name, text, Syntax::Es(EsSyntax::default())).map_err(anyhow::Error::msg)?;
        let fm = self
            .cm
            .new_source_file(Lrc::new(FileName::Custom(js_name)), text.clone());
        Ok(ModuleData {
            fm,
            module,
            helpers: Helpers::new(false),
        })
    }
}

/// The same resolution as [`compile_all`]'s, over the compiled modules.
struct Resolver<'a> {
    files: &'a BTreeMap<String, String>,
}

impl Resolve for Resolver<'_> {
    fn resolve(
        &self,
        base: &FileName,
        spec: &str,
    ) -> Result<swc_ecma_loader::resolve::Resolution, anyhow::Error> {
        let from = match base {
            FileName::Custom(n) => n,
            other => return Err(anyhow::Error::msg(format!("{other}: not a project file"))),
        };
        let to = resolve_spec(self.files, from, spec).map_err(anyhow::Error::msg)?;
        Ok(swc_ecma_loader::resolve::Resolution {
            filename: FileName::Custom(to),
            slug: None,
        })
    }
}

struct NoHook;

impl Hook for NoHook {
    fn get_import_meta_props(
        &self,
        _span: Span,
        _module_record: &ModuleRecord,
    ) -> Result<Vec<KeyValueProp>, anyhow::Error> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests;
