//! No server path in an answer (docs/server-mode.md, "Paths in answers"): over HTTP every path in
//! a tool result, an error message or a viewer snapshot is relative to the workspace, as clients
//! give paths, never the resolved absolute path or the server root.
//!
//! The engine resolves each path parameter under the workspace folder and reports the resolved
//! path (its messages and results are shared with the CLI and stdio, where absolute paths are what
//! a local user wants). The HTTP layer therefore rewrites every string of an answer: a workspace
//! folder prefix (in each spelling it can take: as given, canonical, without the Windows verbatim
//! `\\?\` prefix, with either separator) followed by a separator is removed, and the folder alone
//! becomes `.`; then the same for the server root, which leaves `<workspace>/…`.

use std::path::Path;

use serde_json::Value;

/// The spellings of `dir` an engine message may contain, longest first.
pub(super) fn prefixes(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |s: String| {
        let s = s.trim_end_matches(['/', '\\']).to_string();
        if s.len() > 1 && !out.contains(&s) {
            out.push(s);
        }
    };
    let mut bases = vec![dir.display().to_string()];
    if let Ok(c) = std::fs::canonicalize(dir) {
        bases.push(c.display().to_string());
    }
    for b in bases {
        let plain = b
            .strip_prefix(r"\\?\UNC\")
            .map(|r| format!(r"\\{r}"))
            .or_else(|| b.strip_prefix(r"\\?\").map(str::to_string))
            .unwrap_or_else(|| b.clone());
        for s in [b, plain] {
            add(s.replace('\\', "/"));
            add(s.replace('/', "\\"));
            add(s);
        }
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.len()));
    out
}

/// `text` with every occurrence of a prefix (followed by a separator, or ending a path) made
/// relative.
fn scrub_str(text: &str, prefixes: &[String]) -> Option<String> {
    if !prefixes.iter().any(|p| text.contains(p.as_str())) {
        return None;
    }
    let mut s = text.to_string();
    for p in prefixes {
        for sep in ['/', '\\'] {
            s = s.replace(&format!("{p}{sep}"), "");
        }
        // the folder itself: only where it ends a path (end of text, a quote, a space…)
        let mut out = String::with_capacity(s.len());
        let mut rest = s.as_str();
        while let Some(i) = rest.find(p.as_str()) {
            let (before, after) = rest.split_at(i);
            out.push_str(before);
            let tail = after.get(p.len()..).unwrap_or_default();
            let ends = tail
                .chars()
                .next()
                .is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.')));
            out.push_str(if ends { "." } else { p.as_str() });
            rest = tail;
        }
        out.push_str(rest);
        s = out;
    }
    Some(s)
}

/// Rewrites every string (values and keys) of `v`, depth-bounded.
pub(super) fn scrub(v: &mut Value, prefixes: &[String]) {
    fn walk(v: &mut Value, prefixes: &[String], depth: usize) {
        if depth > 256 {
            return;
        }
        match v {
            Value::String(s) => {
                // a tool result's text is its JSON, serialized (backslashes escaped): rewritten
                // as JSON, then serialized again the same way
                let embedded = (s.starts_with('{') || s.starts_with('['))
                    && prefixes
                        .iter()
                        .any(|p| s.contains(p.as_str()) || s.contains(&p.replace('\\', "\\\\")));
                if embedded && let Ok(mut inner) = serde_json::from_str::<Value>(s) {
                    walk(&mut inner, prefixes, depth + 1);
                    if let Ok(text) = serde_json::to_string_pretty(&inner) {
                        *s = text;
                        return;
                    }
                }
                if let Some(n) = scrub_str(s, prefixes) {
                    *s = n;
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|x| walk(x, prefixes, depth + 1)),
            Value::Object(o) => {
                let renamed: Vec<(String, String)> = o
                    .keys()
                    .filter_map(|k| scrub_str(k, prefixes).map(|n| (k.clone(), n)))
                    .collect();
                for (old, new) in renamed {
                    if let Some(x) = o.remove(&old) {
                        o.insert(new, x);
                    }
                }
                o.values_mut().for_each(|x| walk(x, prefixes, depth + 1));
            }
            _ => {}
        }
    }
    if !prefixes.is_empty() {
        walk(v, prefixes, 0);
    }
}

/// The workspace folder's spellings, then the server root's.
pub(super) fn workspace_prefixes(dir: &Path, root: &Path) -> Vec<String> {
    let mut p = prefixes(dir);
    p.extend(prefixes(root));
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn server_paths_become_relative() {
        let p = vec![
            r"C:\srv\root\demo".to_string(),
            "C:/srv/root/demo".to_string(),
            r"C:\srv\root".to_string(),
        ];
        let mut v = json!({
            "path": r"C:\srv\root\demo\Ventes.pbip",
            "error": r"C:\srv\root\demo/sub/x.bim: not found; root C:\srv\root\demo",
            "list": ["C:/srv/root/demo/a.pbix", r"C:\srv\root\other\b.pbix", r"C:\srv\root\demo2\x"],
            "n": 3,
            r"C:\srv\root\demo\k": 1,
        });
        scrub(&mut v, &p);
        assert_eq!(v["path"], "Ventes.pbip");
        assert_eq!(v["error"], "sub/x.bim: not found; root .");
        assert_eq!(v["list"][0], "a.pbix");
        assert_eq!(v["list"][1], "other\\b.pbix");
        // another folder sharing the prefix is not the workspace: only the root goes
        assert_eq!(v["list"][2], "demo2\\x");
        assert_eq!(v["k"], 1);
        assert_eq!(v["n"], 3);
        // a tool result's text: JSON with escaped backslashes
        let inner = json!({"saved": r"C:\srv\root\demo\out\a.bim"});
        let mut v =
            json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&inner).unwrap()}]});
        scrub(&mut v, &p);
        let text = v["content"][0]["text"].as_str().unwrap();
        assert!(!text.contains("srv"), "{text}");
        assert!(text.contains(r#""saved": "out\\a.bim""#), "{text}");
    }

    #[test]
    fn spellings_cover_verbatim_and_separators() {
        let dir = std::env::temp_dir();
        let p = prefixes(&dir);
        assert!(p.iter().any(|s| s.contains('/')) || !cfg!(windows));
        assert!(p.windows(2).all(|w| w[0].len() >= w[1].len()));
    }
}
