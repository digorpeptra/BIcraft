//! JSON client configurations: `mcpServers` (Claude Code, Claude Desktop, Cursor) or `servers`
//! (VS Code). Key order is kept (`serde_json` `preserve_order`), as are the indentation, line
//! endings, byte-order mark and final newline of the file.

use serde_json::{Map, Value, json};

use super::{Analysis, Change, EntryState, Mode, SERVER_ARGS, SERVER_NAME, Want};

/// The layout details of a JSON file that are restored on writing.
struct Style {
    /// `None`: the file was on one line.
    indent: Option<String>,
    crlf: bool,
    bom: bool,
    final_newline: bool,
}

impl Style {
    fn fresh() -> Style {
        Style {
            indent: Some("  ".into()),
            crlf: false,
            bom: false,
            final_newline: true,
        }
    }

    fn detect(text: &str, bom: bool) -> Style {
        if !text.contains('\n') {
            return Style {
                indent: None,
                crlf: false,
                bom,
                final_newline: false,
            };
        }
        let indent = text
            .lines()
            .skip(1)
            .map(|l| &l[..l.len() - l.trim_start_matches([' ', '\t']).len()])
            .find(|ws| !ws.is_empty())
            .unwrap_or("  ");
        Style {
            indent: Some(indent.to_string()),
            crlf: text.contains("\r\n"),
            bom,
            final_newline: text.ends_with('\n'),
        }
    }

    fn write(&self, v: &Value) -> Result<Vec<u8>, String> {
        let mut out = match &self.indent {
            None => serde_json::to_string(v).map_err(|e| e.to_string())?,
            Some(indent) => {
                // serde_json indents with two spaces; strings never hold a raw newline, so every
                // line's leading spaces are indentation.
                let pretty = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
                if indent == "  " {
                    pretty
                } else {
                    let mut s = String::with_capacity(pretty.len());
                    for (i, line) in pretty.split('\n').enumerate() {
                        if i > 0 {
                            s.push('\n');
                        }
                        let body = line.trim_start_matches(' ');
                        let depth = (line.len() - body.len()) / 2;
                        for _ in 0..depth {
                            s.push_str(indent);
                        }
                        s.push_str(body);
                    }
                    s
                }
            }
        };
        if self.final_newline {
            out.push('\n');
        }
        if self.crlf {
            out = out.replace('\n', "\r\n");
        }
        if self.bom {
            out.insert(0, '\u{feff}');
        }
        Ok(out.into_bytes())
    }
}

fn aligned(entry: &Map<String, Value>, want: &Want) -> bool {
    let command_ok = entry
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|c| want.same_command(c));
    let args_ok = entry.get("args").and_then(Value::as_array).is_some_and(|a| {
        a.len() == SERVER_ARGS.len() && a.iter().zip(SERVER_ARGS).all(|(x, y)| x.as_str() == Some(y))
    });
    let type_ok = entry.get("type").is_none_or(|t| t.as_str() == Some("stdio"));
    command_ok && args_ok && type_ok
}

fn args_value() -> Value {
    Value::Array(SERVER_ARGS.iter().map(|a| json!(a)).collect())
}

/// Reads `bytes` (`None`: no file) and plans the change for `mode`.
pub fn analyze(
    bytes: Option<&[u8]>,
    servers_key: &str,
    typed: bool,
    want: &Want,
    mode: Mode,
) -> Result<Analysis, String> {
    let (mut root, style) = match bytes {
        None => (Map::new(), Style::fresh()),
        Some(b) => {
            let text = std::str::from_utf8(b).map_err(|_| "the file is not UTF-8".to_string())?;
            let (text, bom) = match text.strip_prefix('\u{feff}') {
                Some(t) => (t, true),
                None => (text, false),
            };
            if text.trim().is_empty() {
                let mut style = Style::fresh();
                style.bom = bom;
                (Map::new(), style)
            } else {
                let v: Value = serde_json::from_str(text).map_err(|e| {
                    let hint = if e.is_syntax() && (text.contains("//") || text.contains("/*")) {
                        " (comments and trailing commas are not supported; remove them or edit the \
                         file by hand)"
                    } else {
                        ""
                    };
                    format!("invalid JSON: {e}{hint}")
                })?;
                let Value::Object(map) = v else {
                    return Err("the top level is not a JSON object".into());
                };
                (map, Style::detect(text, bom))
            }
        }
    };

    let state = match root.get(servers_key) {
        None => EntryState::Absent,
        Some(Value::Object(servers)) => match servers.get(SERVER_NAME) {
            None => EntryState::Absent,
            Some(Value::Object(entry)) => {
                if aligned(entry, want) {
                    EntryState::Aligned
                } else {
                    EntryState::Stale {
                        command: entry.get("command").and_then(Value::as_str).map(str::to_string),
                    }
                }
            }
            Some(_) => return Err(format!("`{servers_key}.{SERVER_NAME}` is not a JSON object")),
        },
        Some(_) => return Err(format!("`{servers_key}` is not a JSON object")),
    };

    let changed = match (mode, &state) {
        (Mode::Install, EntryState::Aligned) | (Mode::Uninstall, EntryState::Absent) => false,
        (Mode::Install, _) => {
            let servers = root
                .entry(servers_key)
                .or_insert_with(|| Value::Object(Map::new()));
            if let Value::Object(servers) = servers {
                match servers.get_mut(SERVER_NAME) {
                    Some(Value::Object(entry)) => {
                        entry.insert("command".into(), json!(want.command));
                        entry.insert("args".into(), args_value());
                        if entry.get("type").is_some_and(|t| t.as_str() != Some("stdio")) {
                            entry.insert("type".into(), json!("stdio"));
                        }
                    }
                    _ => {
                        let mut entry = Map::new();
                        if typed {
                            entry.insert("type".into(), json!("stdio"));
                        }
                        entry.insert("command".into(), json!(want.command));
                        entry.insert("args".into(), args_value());
                        servers.insert(SERVER_NAME.into(), Value::Object(entry));
                    }
                }
            }
            true
        }
        (Mode::Uninstall, _) => {
            if let Some(Value::Object(servers)) = root.get_mut(servers_key) {
                servers.shift_remove(SERVER_NAME);
            }
            true
        }
    };
    let change = if changed {
        Change::Write(style.write(&Value::Object(root))?)
    } else {
        Change::None
    };
    Ok(Analysis { state, change })
}
