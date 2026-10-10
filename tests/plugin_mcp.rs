use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdout, Command, Stdio},
    time::Duration,
};

struct McpProcess {
    child: Child,
    output: BufReader<ChildStdout>,
    next_id: u64,
}

impl McpProcess {
    fn start(database: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_cash-ledger-mcp"))
            .env("CASH_REGISTER_DB", database)
            .env(
                "CASH_LEDGER_UPDATE_PREFERENCES",
                database.with_file_name("updates.json"),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start MCP server against isolated database");
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut process = Self {
            child,
            output,
            next_id: 1,
        };
        process.request("initialize", json!({}));
        process.notification("notifications/initialized", json!({}));
        process
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        let input = self.child.stdin.as_mut().unwrap();
        serde_json::to_writer(&mut *input, &request).unwrap();
        input.write_all(b"\n").unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        self.output.read_line(&mut line).expect("read MCP response");
        let response: Value = serde_json::from_str(&line).expect("valid JSON-RPC response");
        assert_eq!(response["id"], id);
        response
    }

    fn notification(&mut self, method: &str, params: Value) {
        let request = json!({"jsonrpc":"2.0","method":method,"params":params});
        let input = self.child.stdin.as_mut().unwrap();
        serde_json::to_writer(&mut *input, &request).unwrap();
        input.write_all(b"\n").unwrap();
        input.flush().unwrap();
    }

    fn tool(&mut self, name: &str, args: Value) -> Value {
        self.request("tools/call", json!({"name":name,"arguments":args}))
    }

    fn result(&mut self, name: &str, args: Value) -> Value {
        let response = self.tool(name, args);
        assert!(response.get("error").is_none(), "{name}: {response}");
        response["result"]["structuredContent"].clone()
    }

    fn error(&mut self, name: &str, args: Value) -> Value {
        let response = self.tool(name, args);
        if let Some(error) = response.get("error") {
            return error.clone();
        }
        assert_eq!(
            response["result"]["isError"], true,
            "expected {name} to fail: {response}"
        );
        json!({"message":response["result"]["content"][0]["text"]})
    }
}

impl Drop for McpProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn stdio_protocol_discovers_tools_and_rejects_bad_requests_without_panicking() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("protocol.sqlite");
    let mut server = McpProcess::start(&database);

    let initialized = server.request("initialize", json!({}));
    assert_eq!(initialized["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(initialized["result"]["serverInfo"]["name"], "cash-ledger");

    let listed = server.request("tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap();
    for name in [
        "setup_company",
        "login",
        "logout",
        "company_profile",
        "save_company_settings",
        "change_password",
        "registers",
        "register_sessions",
        "session_balance",
        "create_register",
        "open_register",
        "record_movement",
        "register_history",
        "close_preview",
        "close_register",
        "report",
        "print_report",
        "export_report_csv",
        "analytics_dashboard",
        "backup_status",
        "save_backup_settings",
        "run_backup",
        "restore_backup",
        "restore_preview",
        "status",
        "update_preferences",
        "set_update_preferences",
        "update_status",
        "check_updates",
        "update_prepare",
        "update_install",
    ] {
        let tool = tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("missing tool {name}"));
        assert_eq!(tool["inputSchema"]["type"], "object", "{name} schema");
        assert_eq!(
            tool["inputSchema"]["additionalProperties"], false,
            "{name} schema"
        );
    }

    let unauthenticated = server.error("registers", json!({}));
    assert!(
        unauthenticated["message"]
            .as_str()
            .unwrap()
            .contains("Login is required")
    );
    assert_eq!(server.result("status", json!({}))["setup_required"], true);
    assert_eq!(
        server.result("status", json!({}))["database_path"],
        json!(database.to_string_lossy().to_string())
    );
    assert!(
        server.error("update_preferences", json!({}))["message"]
            .as_str()
            .unwrap()
            .contains("Login is required")
    );
    assert!(
        server.error("set_update_preferences", json!({"automatic":false}))["message"]
            .as_str()
            .unwrap()
            .contains("Login is required")
    );
    let missing = server.error("setup_company", json!({"username":"admin"}));
    assert!(missing["message"].as_str().unwrap().contains("name"));
    assert_eq!(server.result("logout", json!({}))["logged_out"], true);
    let setup =
        json!({"name":"Protocol Company","username":"admin","password":"correct horse battery"});
    server.notification(
        "tools/call",
        json!({"name":"setup_company","arguments":setup}),
    );
    // A valid follow-up proves malformed requests did not unwind the server and
    // that a JSON-RPC notification did not execute a state-changing tool.
    assert_eq!(
        server.result(
            "setup_company",
            json!({"name":"Protocol Company","username":"admin","password":"correct horse battery"})
        )["configured"],
        true
    );
    assert_eq!(
        server.result("set_update_preferences", json!({"automatic":false}))["automatic"],
        false
    );
    assert_eq!(
        server.result("update_preferences", json!({}))["automatic"],
        false
    );
    assert_eq!(
        server.result("set_update_preferences", json!({"automatic":true}))["automatic"],
        true
    );
    server.result("logout", json!({}));
    assert_eq!(
        server.error(
            "login",
            json!({"username":"admin","password":"bad-password"})
        )["message"],
        "Credentials were not accepted"
    );
    assert_eq!(
        server.result(
            "login",
            json!({"username":"admin","password":"correct horse battery"})
        )["authenticated"],
        true
    );
    assert_eq!(
        server.result("registers", json!({}))["registers"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(server.request("ping", json!({}))["result"], json!({}));
}

#[test]
fn isolated_ledger_lifecycle_covers_tools_without_printing_or_installing() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("ledger.sqlite");
    let backup_dir = directory.path().join("backups");
    std::fs::create_dir(&backup_dir).unwrap();
    let mut server = McpProcess::start(&database);

    let profile = json!({
        "name":"Example Shop", "address":"1 Main Street", "rules":"Count twice",
        "variance_threshold_cents":500, "printer_name":"Test printer", "paper_size":"Letter"
    });
    assert_eq!(server.result("setup_company", json!({
        "name":"Example Shop", "address":"1 Main Street", "rules":"Count twice",
        "variance_threshold_cents":500, "printer_name":"Test printer", "paper_size":"Letter",
        "username":"admin", "password":"old-password-123"
    }))["configured"], true);
    assert!(
        server.error(
            "setup_company",
            json!({"name":"Second","username":"admin","password":"password"})
        )["message"]
            .as_str()
            .unwrap()
            .contains("already complete")
    );
    assert_eq!(server.result("company_profile", json!({})), profile);
    assert_eq!(
        server.result(
            "save_company_settings",
            json!({
                "name":"Example Shop", "address":"2 Main Street", "rules":"Count twice",
                "variance_threshold_cents":250, "printer_name":"", "paper_size":"A4"
            })
        )["saved"],
        true
    );
    assert_eq!(
        server.result("save_company_settings", json!({"name":"Renamed Shop"}))["saved"],
        true
    );
    let saved_profile = server.result("company_profile", json!({}));
    assert_eq!(saved_profile["name"], "Renamed Shop");
    assert_eq!(saved_profile["address"], "2 Main Street");
    assert_eq!(saved_profile["rules"], "Count twice");
    assert_eq!(saved_profile["variance_threshold_cents"], 250);
    assert_eq!(saved_profile["paper_size"], "A4");

    let register = server.result("create_register", json!({"name":"Front desk"}))["register_id"]
        .as_i64()
        .unwrap();
    assert_eq!(
        server.result("registers", json!({}))["registers"][0]["name"],
        "Front desk"
    );
    assert_eq!(
        server.result("register_sessions", json!({"register_id":register}))["sessions"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let opened = server.result(
        "open_register",
        json!({
            "register_id":register,"business_date":"2026-10-09","opening_cents":10000
        }),
    );
    let session = opened["snapshot"]["active_session_id"].as_i64().unwrap();
    assert_eq!(
        server.result("session_balance", json!({"session_id":session}))["expected_cents"],
        10000
    );
    assert_eq!(
        server.result("register_sessions", json!({"register_id":register}))["sessions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let bad = server.error(
        "open_register",
        json!({"register_id":register,"business_date":"2026-02-30","opening_cents":0}),
    );
    assert!(!bad["message"].as_str().unwrap().is_empty());

    let movement = json!({"session_id":session,"direction":"in","amount_cents":2500,"reason":"Cash sale","request_key":"sale-1"});
    let first = server.result("record_movement", movement.clone());
    let retry = server.result("record_movement", movement);
    assert_eq!(first["entry_id"], retry["entry_id"]);
    server.error("record_movement", json!({"session_id":session,"direction":"sideways","amount_cents":1,"reason":"bad","request_key":"bad"}));
    server.result("record_movement", json!({"session_id":session,"direction":"out","amount_cents":400,"reason":"Supplies","request_key":"expense-1"}));
    let history = server.result(
        "register_history",
        json!({"session_id":session,"direction":"in","after_id":0,"limit":10}),
    );
    assert_eq!(history["entries"].as_array().unwrap().len(), 1);
    let preview = server.result(
        "close_preview",
        json!({"session_id":session,"counted_cents":12000}),
    );
    assert_eq!(preview["expected_cents"], 12100);
    server.result("record_movement", json!({"session_id":session,"direction":"in","amount_cents":25,"reason":"Late sale","request_key":"sale-2"}));
    server.error(
        "close_register",
        json!({"session_id":session,"preview_token":preview["preview_token"],"confirm":true}),
    );
    let refreshed_preview = server.result(
        "close_preview",
        json!({"session_id":session,"counted_cents":12000}),
    );
    assert_eq!(refreshed_preview["expected_cents"], 12125);
    let closed = server.result("close_register", json!({"session_id":session,"preview_token":refreshed_preview["preview_token"],"confirm":true}));
    assert_eq!(closed["difference_cents"], -125);
    assert_eq!(
        server.error(
            "print_report",
            json!({"start_date":"2026-10-09","end_date":"2026-10-09","confirm":false})
        )["message"],
        "Set confirm=true to send this report to the selected printer"
    );

    let report_args =
        json!({"start_date":"2026-10-09","end_date":"2026-10-09","register_id":register});
    assert_eq!(
        server.result("report", report_args.clone())["sessions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        !server.error(
            "report",
            json!({"start_date":"2026-10-09","end_date":"2026-10-09","register_id":"wrong type"})
        )["message"]
            .as_str()
            .unwrap()
            .is_empty()
    );
    let csv = directory.path().join("report.csv");
    assert_eq!(server.result("export_report_csv", json!({"start_date":"2026-10-09","end_date":"2026-10-09","register_id":register,"destination":csv}))["exported"], true);
    assert!(directory.path().join("report.csv").is_file());
    let analytics = server.result("analytics_dashboard", json!({"start_date":"2026-10-09","end_date":"2026-10-09","currency":"CAD","register_id":register}));
    assert_eq!(analytics["summary"]["cash_in_cents"], 2525);
    assert_eq!(analytics["summary"]["cash_out_cents"], 400);

    server.result(
        "save_backup_settings",
        json!({"directory":backup_dir,"mode":"manual","frequency":"daily"}),
    );
    assert_eq!(
        server.result("backup_status", json!({}))["settings"]["mode"],
        "manual"
    );
    let backup = server.result("run_backup", json!({}));
    assert_eq!(backup["created"], true);
    let backup_path = backup["path"].as_str().unwrap().to_owned();
    assert!(Path::new(&backup_path).is_file());

    let restore_preview = server.result("restore_preview", json!({"path":backup_path}));
    assert_eq!(restore_preview["confirm_required"], true);
    server.error("restore_preview", json!({"path":database}));
    server.error(
        "restore_backup",
        json!({"path":backup_path,"confirm":true,"preview_token":"wrong"}),
    );
    server.result("create_register", json!({"name":"Temporary register"}));
    assert_eq!(
        server.result("registers", json!({}))["registers"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let restore_preview = server.result("restore_preview", json!({"path":backup_path}));
    assert_eq!(server.result("restore_backup", json!({"path":backup_path,"confirm":true,"preview_token":restore_preview["preview_token"]}))["restored"], true);
    assert!(
        server.error("registers", json!({}))["message"]
            .as_str()
            .unwrap()
            .contains("Login is required")
    );
    assert_eq!(
        server.result(
            "login",
            json!({"username":"admin","password":"old-password-123"})
        )["authenticated"],
        true
    );
    assert_eq!(
        server.result("registers", json!({}))["registers"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    server.error(
        "change_password",
        json!({"username":"admin","current_password":"wrong","new_password":"new-password-456"}),
    );
    assert_eq!(server.result("change_password", json!({"username":"admin","current_password":"old-password-123","new_password":"new-password-456"}))["changed"], true);
    assert_eq!(server.result("logout", json!({}))["logged_out"], true);
    assert!(
        server.error("registers", json!({}))["message"]
            .as_str()
            .unwrap()
            .contains("Login is required")
    );
    assert_eq!(
        server.result(
            "login",
            json!({"username":"admin","password":"new-password-456"})
        )["authenticated"],
        true
    );
    // This update-only desktop status call has no download, print, or install side effect.
    assert!(
        server
            .result("update_status", json!({}))
            .get("installed_target_valid")
            .is_some()
    );
    assert!(
        !server.error(
            "update_install",
            json!({"executable_path":"cash-ledger.exe"})
        )["message"]
            .as_str()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn automatic_backups_run_while_signed_in_and_pause_after_logout() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("automatic.sqlite");
    let signed_in_backups = directory.path().join("signed-in");
    let signed_out_backups = directory.path().join("signed-out");
    std::fs::create_dir(&signed_in_backups).unwrap();
    std::fs::create_dir(&signed_out_backups).unwrap();
    let mut server = McpProcess::start(&database);
    server.result(
        "setup_company",
        json!({"name":"Automatic Backup Test","username":"admin","password":"test-password-123"}),
    );
    server.result(
        "save_backup_settings",
        json!({"directory":signed_in_backups,"mode":"automatic","frequency":"hourly"}),
    );

    std::thread::sleep(Duration::from_secs(65));
    let status = server.result("backup_status", json!({}));
    assert!(status["last_success_at"].as_i64().is_some());
    assert!(
        std::fs::read_dir(&signed_in_backups)
            .unwrap()
            .next()
            .is_some()
    );

    server.result(
        "save_backup_settings",
        json!({"directory":signed_out_backups,"mode":"automatic","frequency":"hourly"}),
    );
    server.result("logout", json!({}));
    std::thread::sleep(Duration::from_secs(65));
    assert!(
        std::fs::read_dir(&signed_out_backups)
            .unwrap()
            .next()
            .is_none()
    );
}
