use serde_json::{Value, json};

use crate::{
    INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND, PARSE_ERROR, PROTOCOL_VERSION, Server, handle, serve,
    serve_with_limit,
};

fn call(server: &mut Server, msg: Value) -> Value {
    let reply = handle(server, &msg.to_string()).expect("a request gets a reply");
    serde_json::from_str(&reply).unwrap()
}

fn tool(server: &mut Server, name: &str, args: Value) -> Value {
    let r = call(
        server,
        json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": name, "arguments": args}}),
    );
    assert_eq!(r["id"], 7);
    let result = r["result"].clone();
    assert!(result["structuredContent"].is_object(), "{result}");
    assert_eq!(result["content"][0]["type"], "text");
    result
}

fn error_code(v: &Value) -> i64 {
    v["error"]["code"].as_i64().unwrap()
}

#[test]
fn initialize_handshake() {
    let mut s = Server::new();
    let r = call(
        &mut s,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
               "params": {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}}),
    );
    assert_eq!(r["jsonrpc"], "2.0");
    assert_eq!(r["id"], 1);
    assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
    assert!(r["result"]["capabilities"]["tools"].is_object());
    assert_eq!(r["result"]["serverInfo"]["name"], "bicraft");
    assert_eq!(r["result"]["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(
        r["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("validate_dax")
    );
    assert!(
        r["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("add_visual")
    );

    // An unknown revision gets our latest one.
    let r = call(
        &mut s,
        json!({"jsonrpc": "2.0", "id": "a", "method": "initialize", "params": {"protocolVersion": "1999-01-01"}}),
    );
    assert_eq!(r["id"], "a");
    assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSION);

    assert!(!s.is_initialized());
    assert_eq!(
        handle(
            &mut s,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
        ),
        None
    );
    assert!(s.is_initialized());
    let r = call(&mut s, json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}));
    assert_eq!(r["result"], json!({}));
}

#[test]
fn notifications_never_get_a_reply() {
    let mut s = Server::new();
    for m in [
        "notifications/initialized",
        "notifications/cancelled",
        "notifications/whatever",
        "tools/call",
    ] {
        assert_eq!(
            handle(&mut s, &json!({"jsonrpc": "2.0", "method": m}).to_string()),
            None
        );
    }
    // Responses from the client and blank lines are ignored too.
    assert_eq!(handle(&mut s, r#"{"jsonrpc":"2.0","id":3,"result":{}}"#), None);
    assert_eq!(handle(&mut s, "   "), None);
}

#[test]
fn tools_list_has_every_tool_with_a_schema() {
    let mut s = Server::new();
    let r = call(&mut s, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}));
    let tools = r["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "list_commands",
            "run_command",
            "batch",
            "open_file",
            "inspect_file",
            "model_summary",
            "validate_dax",
            "validate_model",
            "model_usage",
            "model_unused",
            "remove_unused",
            "new_model",
            "add_table",
            "add_column",
            "add_measure",
            "set_measure",
            "add_relationship",
            "save",
            "save_as",
            "convert",
            "diff_files",
            "import_objects",
            "new_project",
            "report_summary",
            "add_page",
            "add_visual",
            "bind_field",
            "validate_report",
            "validate_dax_query",
            "evaluate_dax_query",
            "refresh_model",
            "render_page",
            "run_tests",
            "list_sources",
            "rebase_sources",
            "apply_env",
            "check_env",
        ]
    );
    for t in tools {
        assert!(!t["description"].as_str().unwrap().is_empty());
        assert_eq!(t["inputSchema"]["type"], "object", "{t}");
        assert!(t["inputSchema"]["properties"].is_object(), "{t}");
    }
    let dax = tools.iter().find(|t| t["name"] == "validate_dax").unwrap();
    assert_eq!(dax["inputSchema"]["required"], json!(["expression"]));
}

#[test]
fn validate_dax_valid_and_invalid() {
    let mut s = Server::new();
    let ok = tool(&mut s, "validate_dax", json!({"expression": "1 + 2"}));
    assert_eq!(ok["isError"], false);
    assert_eq!(ok["structuredContent"]["valid"], true);
    assert_eq!(ok["structuredContent"]["checked_against_model"], false);
    let text: Value = serde_json::from_str(ok["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, ok["structuredContent"]);

    let bad = tool(&mut s, "validate_dax", json!({"expression": "SUM(1 +"}));
    // An invalid expression is a successful validation with `valid: false`, not a tool error.
    assert_eq!(bad["isError"], false);
    assert_eq!(bad["structuredContent"]["valid"], false);
    assert!(
        !bad["structuredContent"]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let missing = tool(&mut s, "validate_dax", json!({}));
    assert_eq!(missing["isError"], true);
}

#[test]
fn validate_dax_against_an_open_model() {
    let tmp = crate::tempdir::TempDir::new("mcp-model");
    let dir = tmp.path();
    let bim = dir.join("model.bim");
    std::fs::write(
        &bim,
        r#"{"name":"M","model":{"tables":[{"name":"Sales",
            "columns":[{"name":"Amount","dataType":"double"}],
            "measures":[{"name":"Total","expression":"SUM(Sales[Amount])"}]}]}}"#,
    )
    .unwrap();
    let mut s = Server::new();
    let opened = tool(&mut s, "open_file", json!({"path": bim.to_str().unwrap()}));
    assert_eq!(opened["isError"], false, "{opened}");
    assert_eq!(tool(&mut s, "inspect_file", json!({}))["isError"], false);
    let summary = tool(&mut s, "model_summary", json!({}));
    assert_eq!(summary["structuredContent"]["tables"][0]["name"], "Sales");
    let ok = tool(
        &mut s,
        "validate_dax",
        json!({"expression": "SUM(Sales[Amount])"}),
    );
    assert_eq!(ok["structuredContent"]["valid"], true);
    assert_eq!(ok["structuredContent"]["checked_against_model"], true);
    let bad = tool(&mut s, "validate_dax", json!({"expression": "SUM(Sales[Nope])"}));
    assert_eq!(bad["structuredContent"]["valid"], false);
    let model = tool(&mut s, "validate_model", json!({}));
    assert_eq!(model["structuredContent"]["valid"], true);
}

#[test]
fn run_command_and_list_commands() {
    let mut s = Server::new();
    let unknown = tool(&mut s, "run_command", json!({"command": "no.such.command"}));
    assert_eq!(unknown["isError"], true);
    assert!(
        unknown["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("no.such.command")
    );

    let r = tool(
        &mut s,
        "run_command",
        json!({"command": "dax.validate", "params": {"expression": "1"}}),
    );
    assert_eq!(r["isError"], false);
    assert_eq!(r["structuredContent"]["valid"], true);

    // No file open: a command error, reported as a tool error.
    assert_eq!(tool(&mut s, "model_summary", json!({}))["isError"], true);
    // Wrong argument types are tool errors, not crashes.
    assert_eq!(
        tool(&mut s, "run_command", json!({"command": 5}))["isError"],
        true
    );
    assert_eq!(
        tool(
            &mut s,
            "run_command",
            json!({"command": "dax.validate", "params": "x"})
        )["isError"],
        true
    );
    assert_eq!(
        tool(&mut s, "list_commands", json!({"filter": 3}))["isError"],
        true
    );

    let all = tool(&mut s, "list_commands", json!({}));
    let list = all["structuredContent"]["commands"].as_array().unwrap();
    assert!(list.iter().any(|c| c["id"] == "dax.validate"));
    // stdio is a local session: every command is listed, server-only filtering does not apply
    assert_eq!(list.len(), bicraft_engine::COMMANDS.len());
    assert!(list.iter().any(|c| c["id"] == "file.open_in_desktop"));
    assert_eq!(
        crate::tool_definitions().len(),
        crate::tools::server_tool_definitions().len()
    );
    let dax = tool(&mut s, "list_commands", json!({"filter": "DAX"}));
    let dax = dax["structuredContent"]["commands"].as_array().unwrap();
    assert!(dax.iter().any(|c| c["id"] == "dax.validate_query"));
    assert!(dax.iter().all(|c| {
        let text = format!("{} {}", c["id"], c["label"]).to_ascii_lowercase();
        text.contains("dax")
    }));
}

#[test]
fn batch_stops_at_the_first_error() {
    let mut s = Server::new();
    let r = tool(
        &mut s,
        "batch",
        json!({"commands": [
            {"command": "dax.validate", "params": {"expression": "1 + 1"}},
            {"command": "dax.validate", "params": {"expression": "2"}},
            {"command": "nope.nope"},
            {"command": "dax.validate", "params": {"expression": "3"}},
        ]}),
    );
    assert_eq!(r["isError"], true);
    let sc = &r["structuredContent"];
    assert_eq!(sc["failed_index"], 2);
    assert_eq!(sc["results"].as_array().unwrap().len(), 2);
    assert!(sc["error"].as_str().unwrap().contains("nope.nope"));

    let ok = tool(
        &mut s,
        "batch",
        json!({"commands": [{"command": "dax.validate", "params": {"expression": "1"}}]}),
    );
    assert_eq!(ok["isError"], false);
    assert_eq!(ok["structuredContent"]["results"].as_array().unwrap().len(), 1);

    let malformed = tool(
        &mut s,
        "batch",
        json!({"commands": [{"command": "dax.validate"}, 7]}),
    );
    assert_eq!(malformed["isError"], true);
    assert_eq!(malformed["structuredContent"]["failed_index"], 1);
    assert_eq!(tool(&mut s, "batch", json!({}))["isError"], true);
}

#[test]
fn protocol_errors() {
    let mut s = Server::new();
    let r: Value = serde_json::from_str(&handle(&mut s, "{not json").unwrap()).unwrap();
    assert_eq!(error_code(&r), PARSE_ERROR);
    assert_eq!(r["id"], Value::Null);

    let r = call(
        &mut s,
        json!({"jsonrpc": "2.0", "id": 1, "method": "resources/list"}),
    );
    assert_eq!(error_code(&r), METHOD_NOT_FOUND);

    let r = call(
        &mut s,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {}}),
    );
    assert_eq!(error_code(&r), INVALID_PARAMS);
    let r = call(
        &mut s,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "nope"}}),
    );
    assert_eq!(error_code(&r), INVALID_PARAMS);
    let r = call(
        &mut s,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "validate_dax", "arguments": [1]}}),
    );
    assert_eq!(error_code(&r), INVALID_PARAMS);
    let r = call(
        &mut s,
        json!({"jsonrpc": "2.0", "id": 1, "method": "ping", "params": 3}),
    );
    assert_eq!(error_code(&r), INVALID_PARAMS);

    for bad in [
        json!(42),
        json!("x"),
        json!({"id": 1}),
        json!({"id": {}, "method": "ping"}),
        json!([]),
    ] {
        let r = call(&mut s, bad);
        assert_eq!(error_code(&r), INVALID_REQUEST);
    }
    // Old-style JSON-RPC batches still work; notifications inside get no reply.
    let r = call(
        &mut s,
        json!([{"jsonrpc": "2.0", "id": 1, "method": "ping"}, {"jsonrpc": "2.0", "method": "notifications/initialized"}]),
    );
    assert_eq!(r.as_array().unwrap().len(), 1);
}

#[test]
fn serve_over_streams() {
    let input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        "\r\n\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"validate_dax","arguments":{"expression":"1+1"}}}"#,
        "\n",
        "garbage",
    );
    let mut out = Vec::new();
    serve(input.as_bytes(), &mut out).unwrap();
    let lines: Vec<Value> = std::str::from_utf8(&out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(lines[1]["result"]["structuredContent"]["valid"], true);
    assert_eq!(error_code(&lines[2]), PARSE_ERROR);
}

#[test]
fn over_long_and_non_utf8_lines_are_rejected() {
    let mut input = vec![b'x'; 5000];
    input.push(b'\n');
    input.extend_from_slice(&[0xff, 0xfe, b'\n']);
    input.extend_from_slice(br#"{"jsonrpc":"2.0","id":9,"method":"ping"}"#);
    input.push(b'\n');
    let mut out = Vec::new();
    // A small buffer forces the line to span several reads.
    let reader = std::io::BufReader::with_capacity(16, input.as_slice());
    serve_with_limit(&mut Server::new(), reader, &mut out, 1024).unwrap();
    let lines: Vec<Value> = std::str::from_utf8(&out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(error_code(&lines[0]), PARSE_ERROR);
    assert_eq!(error_code(&lines[1]), PARSE_ERROR);
    assert_eq!(lines[2]["id"], 9);
}

#[test]
fn deeply_nested_input_does_not_crash() {
    let mut s = Server::new();
    let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    let r: Value = serde_json::from_str(&handle(&mut s, &deep).unwrap()).unwrap();
    assert_eq!(error_code(&r), PARSE_ERROR);
    let expr = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
    let r = tool(&mut s, "validate_dax", json!({"expression": expr}));
    assert!(r["structuredContent"].is_object());
}

#[test]
fn editing_tools_build_validate_and_save_a_model() {
    let tmp = crate::tempdir::TempDir::new("mcp-edit");
    let dir = tmp.path();
    let mut s = Server::new();
    // Nothing open yet.
    assert_eq!(tool(&mut s, "add_table", json!({"name": "T"}))["isError"], true);

    let r = tool(&mut s, "new_model", json!({"name": "Shop"}));
    assert_eq!(r["isError"], false, "{r}");
    let r = tool(
        &mut s,
        "add_table",
        json!({"name": "Sales", "partition": {"source_type": "m", "expression": "#table({\"Amount\"}, {{1}})"}}),
    );
    assert_eq!(r["isError"], false, "{r}");
    tool(&mut s, "add_table", json!({"name": "Product"}));
    for (table, name, data_type) in [
        ("Sales", "Amount", "double"),
        ("Sales", "Product", "string"),
        ("Product", "Product", "string"),
    ] {
        let r = tool(
            &mut s,
            "add_column",
            json!({"table": table, "name": name, "data_type": data_type}),
        );
        assert_eq!(r["isError"], false, "{r}");
    }
    let r = tool(
        &mut s,
        "add_measure",
        json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])", "format_string": "0.00"}),
    );
    assert_eq!(r["isError"], false, "{r}");

    // Invalid DAX is a tool error that names the problem; the model is unchanged.
    let bad = tool(
        &mut s,
        "add_measure",
        json!({"table": "Sales", "name": "Bad", "expression": "SUM(Sales[Nope])"}),
    );
    assert_eq!(bad["isError"], true);
    let text = bad["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("invalid DAX") && text.contains("force"), "{text}");
    let forced = tool(
        &mut s,
        "add_measure",
        json!({"table": "Sales", "name": "Bad", "expression": "SUM(Sales[Nope])", "force": true}),
    );
    assert_eq!(forced["isError"], false);
    assert!(
        !forced["structuredContent"]["warnings"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // Unknown arguments are rejected rather than ignored.
    assert_eq!(
        tool(
            &mut s,
            "add_measure",
            json!({"table": "Sales", "name": "X", "expression": "1", "fmt": "0"})
        )["isError"],
        true
    );

    let r = tool(
        &mut s,
        "set_measure",
        json!({"name": "Bad", "expression": "[Total] * 2"}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let r = tool(
        &mut s,
        "set_measure",
        json!({"name": "Total", "new_name": "Revenue"}),
    );
    // The rename rewrites [Bad]'s DAX, so nothing breaks.
    assert_eq!(r["structuredContent"]["dax_rewritten"][0]["name"], "Bad", "{r}");
    assert_eq!(r["structuredContent"]["broken"], json!([]));

    let r = tool(
        &mut s,
        "add_relationship",
        json!({"from": "Sales[Product]", "to": "Product[Product]", "cross_filter": "both"}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["structuredContent"]["name"].as_str().unwrap().len(), 36);

    // The other commands stay reachable through run_command.
    let r = tool(
        &mut s,
        "run_command",
        json!({"command": "role.add", "params": {"name": "Readers"}}),
    );
    assert_eq!(r["isError"], false, "{r}");

    let summary = tool(&mut s, "model_summary", json!({}));
    assert_eq!(
        summary["structuredContent"]["tables"].as_array().unwrap().len(),
        2
    );
    assert_eq!(
        tool(&mut s, "inspect_file", json!({}))["structuredContent"]["dirty"],
        true
    );

    // Never saved: `save` explains what to do instead.
    let r = tool(&mut s, "save", json!({}));
    assert_eq!(r["isError"], true);
    assert!(r["content"][0]["text"].as_str().unwrap().contains("save_as"));

    let bim = dir.join("Shop.bim");
    let r = tool(&mut s, "save_as", json!({"path": bim.to_str().unwrap()}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["structuredContent"]["format"], "bim");
    assert!(bim.is_file());
    tool(
        &mut s,
        "set_measure",
        json!({"name": "Revenue", "description": "All sales"}),
    );
    assert_eq!(tool(&mut s, "save", json!({}))["isError"], false);
    assert_eq!(
        tool(&mut s, "inspect_file", json!({}))["structuredContent"]["dirty"],
        false
    );

    let r = tool(
        &mut s,
        "new_project",
        json!({"dir": dir.to_str().unwrap(), "name": "ShopProject", "format": "tmsl"}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert!(dir.join("ShopProject.pbip").is_file());
    assert!(dir.join("ShopProject.SemanticModel").join("model.bim").is_file());

    // The saved project reopens with the measure renamed and described.
    let mut fresh = Server::new();
    let r = tool(
        &mut fresh,
        "open_file",
        json!({"path": dir.join("ShopProject.pbip").to_str().unwrap()}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let summary = tool(&mut fresh, "model_summary", json!({}));
    assert_eq!(
        summary["structuredContent"]["tables"][0]["measures"],
        json!(["Revenue", "Bad"])
    );
}

#[test]
fn editing_tool_schemas_match_the_engine() {
    let tools = crate::tool_definitions();
    let find = |name: &str| tools.iter().find(|t| t["name"] == name).unwrap().clone();
    for (name, required) in [
        ("new_model", json!(["name"])),
        ("add_table", json!(["name"])),
        ("add_column", json!(["table", "name", "data_type"])),
        ("add_measure", json!(["table", "name", "expression"])),
        ("set_measure", json!(["name"])),
        ("add_relationship", json!(["from", "to"])),
        ("save", json!([])),
        ("save_as", json!(["path"])),
        ("convert", json!(["from", "to"])),
        ("diff_files", json!(["a", "b"])),
        ("import_objects", json!(["source"])),
        ("new_project", json!(["dir", "name"])),
        ("report_summary", json!([])),
        ("add_page", json!(["display_name"])),
        (
            "add_visual",
            json!(["page", "visual_type", "x", "y", "width", "height"]),
        ),
        ("bind_field", json!(["page", "visual", "role", "field"])),
        ("validate_report", json!([])),
    ] {
        let t = find(name);
        assert_eq!(t["inputSchema"]["required"], required, "{name}");
        assert_eq!(t["inputSchema"]["additionalProperties"], false, "{name}");
        // Every property the schema advertises is accepted by the engine command: a call with
        // only that property must never fail with "unknown parameter".
        let mut s = Server::new();
        tool(&mut s, "new_model", json!({"name": "M"}));
        for key in t["inputSchema"]["properties"].as_object().unwrap().keys() {
            let r = tool(&mut s, name, json!({key: null}));
            let text = r["content"][0]["text"].as_str().unwrap();
            assert!(!text.contains("unknown parameter"), "{name}.{key}: {text}");
        }
    }
    let add_column = find("add_column");
    assert_eq!(
        add_column["inputSchema"]["properties"]["data_type"]["enum"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
}

#[test]
fn report_tools_build_bind_validate_and_follow_renames() {
    let mut s = Server::new();
    // No report yet: a tool error that says how to get one.
    tool(&mut s, "new_model", json!({"name": "Shop"}));
    let r = tool(&mut s, "report_summary", json!({}));
    assert_eq!(r["isError"], true);
    assert!(r["content"][0]["text"].as_str().unwrap().contains("report.new"));

    for (name, args) in [
        ("add_table", json!({"name": "Sales"})),
        (
            "add_column",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "add_column",
            json!({"table": "Sales", "name": "Region", "data_type": "string"}),
        ),
        (
            "add_measure",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        ("run_command", json!({"command": "report.new"})),
        ("add_page", json!({"display_name": "Overview"})),
    ] {
        let r = tool(&mut s, name, args);
        assert_eq!(r["isError"], false, "{name}: {r}");
    }

    let r = tool(
        &mut s,
        "add_visual",
        json!({"page": "Overview", "visual_type": "clusteredColumnChart", "x": 0, "y": 0,
               "width": 640, "height": 360,
               "fields": [{"role": "Category", "field": "Sales[Region]"}, {"role": "Y", "field": "[Total]"}]}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let visual = r["structuredContent"]["visual"].as_str().unwrap().to_string();
    let r = tool(
        &mut s,
        "bind_field",
        json!({"page": "Overview", "visual": visual, "role": "Y", "field": "Avg(Sales[Amount])"}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let bad = tool(
        &mut s,
        "bind_field",
        json!({"page": "Overview", "visual": visual, "role": "Y", "field": "[Margin]"}),
    );
    assert_eq!(bad["isError"], true);
    assert!(bad["content"][0]["text"].as_str().unwrap().contains("no measure"));

    let summary = tool(&mut s, "report_summary", json!({"page": "Overview"}));
    let sc = &summary["structuredContent"];
    assert_eq!(sc["page_count"], 2);
    assert_eq!(
        sc["pages"][0]["visuals"][0]["fields"].as_array().unwrap().len(),
        3
    );
    let valid = tool(&mut s, "validate_report", json!({}));
    assert_eq!(valid["structuredContent"]["valid"], true, "{valid}");

    // A measure rename reports the binding; update_report rewrites it.
    let r = tool(
        &mut s,
        "set_measure",
        json!({"name": "Total", "new_name": "Revenue"}),
    );
    assert_eq!(r["structuredContent"]["report"]["affected_binding_count"], 1);
    let invalid = tool(&mut s, "validate_report", json!({}));
    assert_eq!(invalid["isError"], false);
    assert_eq!(invalid["structuredContent"]["valid"], false);
    tool(
        &mut s,
        "set_measure",
        json!({"name": "Revenue", "new_name": "Total"}),
    );
    let r = tool(
        &mut s,
        "set_measure",
        json!({"name": "Total", "new_name": "Revenue", "update_report": true}),
    );
    assert_eq!(r["structuredContent"]["report"]["updated"], true, "{r}");
    let valid = tool(&mut s, "validate_report", json!({}));
    assert_eq!(valid["structuredContent"]["valid"], true, "{valid}");

    // Page and visual commands are reachable through run_command and batch.
    let r = tool(
        &mut s,
        "batch",
        json!({"commands": [
            {"command": "page.rename", "params": {"page": "Overview", "display_name": "Sales"}},
            {"command": "visual.set_position", "params": {"page": "Sales", "visual": visual, "x": 20}},
            {"command": "visual.unbind", "params": {"page": "Sales", "visual": visual, "role": "Y"}},
            {"command": "page.move", "params": {"page": "Sales", "index": 0}},
            {"command": "page.set_active", "params": {"page": "Sales"}},
            {"command": "visual.remove", "params": {"page": "Sales", "visual": visual}},
            {"command": "page.remove", "params": {"page": "Page 1"}}
        ]}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let info = tool(&mut s, "inspect_file", json!({}));
    assert_eq!(
        info["structuredContent"]["report"],
        json!({"pages": 1, "visuals": 0})
    );
    assert_eq!(info["structuredContent"]["report_dirty"], true);
}

#[test]
fn validate_dax_query_tool() {
    let mut s = Server::new();
    let tools = crate::tool_definitions();
    let def = tools.iter().find(|t| t["name"] == "validate_dax_query").unwrap();
    assert_eq!(def["inputSchema"]["required"], json!(["query"]));

    let r = tool(&mut s, "validate_dax_query", json!({"query": "EVALUATE { 1 }"}));
    assert_eq!(r["isError"], false);
    assert_eq!(r["structuredContent"]["valid"], true);
    assert_eq!(r["structuredContent"]["evaluate_count"], 1);

    // An invalid query is a result, not a tool error.
    let r = tool(&mut s, "validate_dax_query", json!({"query": "EVALUATE 1"}));
    assert_eq!(r["isError"], false);
    assert_eq!(r["structuredContent"]["valid"], false);
    assert_eq!(
        r["structuredContent"]["diagnostics"][0]["code"],
        "evaluate-needs-table"
    );
    assert_eq!(tool(&mut s, "validate_dax_query", json!({}))["isError"], true);
}

#[test]
fn batch_is_atomic_by_default_and_undoable() {
    let mut s = Server::new();
    let tables = |s: &mut Server| {
        tool(s, "model_summary", json!({}))["structuredContent"]["tables"]
            .as_array()
            .map_or(0, Vec::len)
    };
    assert_eq!(tool(&mut s, "new_model", json!({"name": "M"}))["isError"], false);
    let twice = json!([
        {"command": "table.add", "params": {"name": "A"}},
        {"command": "table.add", "params": {"name": "A"}},
    ]);
    let r = tool(&mut s, "batch", json!({"commands": twice}));
    assert_eq!(r["isError"], true);
    let sc = &r["structuredContent"];
    assert_eq!(sc["failed_index"], 1);
    assert_eq!(sc["failed_command"], "table.add");
    assert_eq!(sc["rolled_back"], true);
    assert_eq!(sc["results"].as_array().unwrap().len(), 1);
    assert_eq!(tables(&mut s), 0);

    let r = tool(&mut s, "batch", json!({"commands": twice, "atomic": false}));
    assert_eq!(r["structuredContent"]["rolled_back"], false);
    assert_eq!(tables(&mut s), 1);
    let r = tool(
        &mut s,
        "run_command",
        json!({"command": "edit.undo", "params": {}}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(tables(&mut s), 0);
    assert_eq!(
        tool(&mut s, "batch", json!({"commands": [], "atomic": "yes"}))["isError"],
        true
    );
}

#[test]
fn evaluate_dax_query_runs_a_query() {
    let mut s = Server::new();
    assert_eq!(tool(&mut s, "new_model", json!({"name": "M"}))["isError"], false);
    let r = tool(
        &mut s,
        "evaluate_dax_query",
        json!({"query": "EVALUATE {(1, \"a\"), (2, \"b\")} ORDER BY [Value1] DESC", "max_rows": 1}),
    );
    assert_eq!(r["isError"], false, "{r}");
    let res = &r["structuredContent"]["results"][0];
    assert_eq!(res["rows"], json!([{"[Value1]": 2, "[Value2]": "b"}]));
    assert_eq!(res["truncated"], true);
    let bad = tool(
        &mut s,
        "evaluate_dax_query",
        json!({"query": "EVALUATE ROW(\"x\", [Nope])"}),
    );
    assert_eq!(bad["isError"], true);
}

#[test]
fn render_page_returns_an_image_block() {
    let mut s = Server::new();
    tool(&mut s, "new_model", json!({"name": "Shop"}));
    tool(&mut s, "run_command", json!({"command": "report.new"}));
    let r = tool(
        &mut s,
        "render_page",
        json!({"page": 0, "system_fonts": false, "scale": 0.25}),
    );
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(r["content"][1]["type"], "image");
    assert_eq!(r["content"][1]["mimeType"], "image/png");
    assert!(r["content"][1]["data"].as_str().unwrap().starts_with("iVBOR"));
    assert_eq!(r["structuredContent"]["pages"][0]["image"]["content_block"], 1);
    assert!(!r["content"][0]["text"].as_str().unwrap().contains("iVBOR"));
}

/// The reply line written straight from a tool's result is, byte for byte, the line the value
/// path writes (`{"jsonrpc", "id", "result": to_value()}` serialized), for every kind of result:
/// strings needing every escape, numbers, nesting, plain and structured errors, ids.
#[test]
fn response_lines_are_the_bytes_of_the_value_path() {
    use crate::ToolResult;
    let mut nasty = String::new();
    for b in 0u8..0x20 {
        nasty.push(char::from(b));
    }
    nasty.push_str("\"\\/\u{7f}\u{80}é日😀\u{2028}\u{2029}\u{feff}");
    let results = vec![
        ToolResult::ok(json!({})),
        ToolResult::ok(json!([])),
        ToolResult::ok(json!("text")),
        ToolResult::ok(json!({
            "results": [{
                "columns": [{"name": "T[c]", "type": "string"}, {"name": "[n]", "type": "double"}],
                "rows": [
                    {"T[c]": nasty.clone(), "[n]": 1.5},
                    {"T[c]": null, "[n]": -0.0},
                    {"T[c]": "", "[n]": 1e300},
                    {"T[c]": "a\"b", "[n]": 0.1 + 0.2},
                    {"T[c]": "2024-01-02T03:04:05.006", "[n]": i64::MIN},
                    {"T[c]": true, "[n]": u64::MAX},
                ],
                "row_count": 6,
                "truncated": false,
            }],
            "nested": {"a": [[], {}, [{"b": [1, 2, {"c": nasty.clone()}]}]]},
            "data_source": "DataModel",
            "elapsed_ms": 3,
        })),
        ToolResult::error("plain message with \"quotes\"\nand a newline"),
        ToolResult::error(nasty.clone()),
        ToolResult::error_with(json!({"error": "batch failed", "failed_index": 1, "results": [{"x": 1}]})),
        ToolResult::error_with(json!({"error": 3})),
        ToolResult::error_with(json!({"other": "no error key"})),
    ];
    let ids = [
        json!(1),
        json!(0),
        json!(-7),
        json!(1.5),
        json!("id"),
        json!("q\"uote\n"),
    ];
    for r in &results {
        for id in &ids {
            let reference =
                serde_json::to_string(&json!({"jsonrpc": "2.0", "id": id, "result": r.to_value()})).unwrap();
            assert_eq!(
                r.response_line(id).as_deref(),
                Some(reference.as_str()),
                "{r:?} {id}"
            );
            let moved = crate::to_line(&crate::response(id.clone(), r.clone().into_value()));
            assert_eq!(moved, reference, "{r:?} {id}");
        }
    }
    // results with image blocks keep the value path
    let pages = ToolResult::ok(json!({"pages": [{"image": {"mime": "image/png", "base64": "AAAA"}}]}));
    assert_eq!(pages.response_line(&json!(1)), None);
    assert_eq!(pages.clone().into_value(), pages.to_value());
}

/// End to end: `dax.evaluate` replies over `handle_line` carry the structured result once in
/// compact form and once as its pretty text, exactly as the value path serializes them (the
/// line is the re-serialization of its own parse, and the text the pretty form of the result).
#[test]
fn evaluate_replies_keep_their_format() {
    let mut s = Server::new();
    assert_eq!(tool(&mut s, "new_model", json!({"name": "M"}))["isError"], false);
    let queries = [
        "EVALUATE {(1, \"a\"), (2, \"b\"), (3, \"B\"), (4, \"\u{e9}\"), (5, \"E\")} ORDER BY [Value2], [Value1] DESC",
        "EVALUATE ROW(\"t\", \"q\"\"uote\" & UNICHAR(9) & UNICHAR(10) & UNICHAR(13) & UNICHAR(1) & UNICHAR(127) & \"\u{e9}\u{65e5}\" & UNICHAR(128512), \
         \"b\", BLANK(), \"d\", DATE(2024, 1, 2) + TIME(3, 4, 5), \"c\", CURRENCY(1.2345), \"x\", TRUE(), \"f\", 0.1 + 0.2, \"big\", 1E300, \"i\", 42)",
        "EVALUATE ROW(\"x\", [Nope])",
    ];
    for (k, q) in queries.iter().enumerate() {
        let msg = json!({"jsonrpc": "2.0", "id": k, "method": "tools/call",
            "params": {"name": "evaluate_dax_query", "arguments": {"query": q}}});
        let line = handle(&mut s, &msg.to_string()).unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(crate::to_line(&v), line, "{q}");
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["jsonrpc", "id", "result"]);
        let result = v["result"].as_object().unwrap();
        let keys: Vec<&String> = result.keys().collect();
        assert_eq!(keys, ["content", "structuredContent", "isError"]);
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        let structured = &v["result"]["structuredContent"];
        if v["result"]["isError"] == true {
            assert_eq!(text, structured["error"].as_str().unwrap());
        } else {
            assert_eq!(text, serde_json::to_string_pretty(structured).unwrap(), "{q}");
        }
    }
}

#[test]
fn source_and_environment_tools() {
    let mut s = Server::new();
    tool(&mut s, "new_model", json!({"name": "M"}));
    tool(
        &mut s,
        "run_command",
        json!({"command": "table.add", "params": {"name": "Sales", "partition": {"source_type": "m",
            "expression": "let Source = Csv.Document(File.Contents(\"C:\\Data\\s.csv\")) in Source"}}}),
    );
    let l = tool(&mut s, "list_sources", json!({}));
    assert_eq!(l["isError"], false, "{l}");
    assert_eq!(
        l["structuredContent"]["sources"][1]["arguments"][0]["value"],
        "C:\\Data\\s.csv"
    );
    let r = tool(
        &mut s,
        "rebase_sources",
        json!({"from": "C:\\Data", "to": "D:\\x", "param": "Root", "dry_run": true}),
    );
    assert_eq!(
        r["structuredContent"]["changes"][0]["m_after"], "Root & \"s.csv\"",
        "{r}"
    );
    let r = tool(
        &mut s,
        "rebase_sources",
        json!({"from": "C:\\Data", "param": "Root"}),
    );
    assert_eq!(r["structuredContent"]["added_parameter"]["name"], "Root", "{r}");
    let env = json!({"version": 1, "profiles": {"prod": {"parameters": {"Root": "E:\\prod\\"}}}});
    let c = tool(&mut s, "check_env", json!({"profile": "prod", "env_file": env}));
    assert_eq!(c["structuredContent"]["ok"], true, "{c}");
    let a = tool(&mut s, "apply_env", json!({"profile": "prod", "env_file": env}));
    assert_eq!(
        a["structuredContent"]["parameters"][0]["after"], "E:\\prod\\",
        "{a}"
    );
    let bad = tool(&mut s, "apply_env", json!({"profile": "nope", "env_file": env}));
    assert_eq!(bad["isError"], true);
}
