//! Model Context Protocol stdio interface for Cash Ledger.
//!
//! The server uses the same public services as the desktop application. It never
//! writes ledger tables directly, and authorization is held only for this process.

use crate::{
    analytics::{self, DashboardFilter},
    backup_service::{BackupFrequency, BackupMode, BackupService, BackupSettings},
    company::{CompanyProfile, CompanyService, SetupCompany},
    database::{Database, Direction, NewEntry},
    register::{Operation, RegisterService, Selection},
    reporting::{self, ReportFilter},
};
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

const PROTOCOL_VERSION: &str = "2025-03-26";

#[derive(Debug)]
struct RpcError {
    code: i64,
    message: String,
}

impl RpcError {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: message.into(),
        }
    }
    fn internal(message: impl Into<String>) -> Self {
        Self {
            code: -32000,
            message: message.into(),
        }
    }
}

/// Stateful request dispatcher. `database_path` is injectable for embedding and tests.
pub struct Server {
    database_path: PathBuf,
    authenticated: bool,
    prepared_update: Option<crate::updater::PreparedUpdate>,
    exit_after_response: bool,
    close_preview: Option<(String, i64, i64, i64)>,
    restore_preview: Option<(String, crate::restore::RestorePreview)>,
    authenticated_signal: Arc<AtomicBool>,
    background_operation: Arc<Mutex<()>>,
    initialized: bool,
}

impl Server {
    pub fn open(database_path: impl Into<PathBuf>) -> Self {
        Self {
            database_path: database_path.into(),
            authenticated: false,
            prepared_update: None,
            exit_after_response: false,
            close_preview: None,
            restore_preview: None,
            authenticated_signal: Arc::new(AtomicBool::new(false)),
            background_operation: Arc::new(Mutex::new(())),
            initialized: false,
        }
    }

    pub(crate) fn dispatch(&mut self, request: &Value) -> Value {
        if request.get("id").is_none() {
            if request.get("method").and_then(Value::as_str) == Some("notifications/initialized") {
                self.initialized = true;
            }
            return Value::Null;
        }
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        if !request.is_object() || request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return json!({"jsonrpc":"2.0","id":id,"error":{"code":-32600,"message":"Invalid JSON-RPC request"}});
        }
        let Some(method) = request.get("method").and_then(Value::as_str) else {
            return json!({"jsonrpc":"2.0","id":id,"error":{"code":-32600,"message":"Request method must be a string"}});
        };
        if method == "initialize" {
            self.initialized = true;
        } else if !self.initialized && method != "ping" {
            return json!({"jsonrpc":"2.0","id":id,"error":{"code":-32002,"message":"MCP initialize is required before using server methods"}});
        }
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        let result = match method {
            "initialize" => Ok(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "cash-ledger", "version": env!("CARGO_PKG_VERSION")}
            })),
            "notifications/initialized" | "notifications/cancelled" => return Value::Null,
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": schema::tools()})),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                match schema::validate_arguments(name, &arguments)
                    .and_then(|()| self.call_tool(name, &arguments))
                {
                    Ok(value) => Ok(
                        json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value}),
                    ),
                    Err(error) => {
                        Ok(json!({"isError":true,"content":[{"type":"text","text":error.message}]}))
                    }
                }
            }
            _ => Err(RpcError {
                code: -32601,
                message: format!("Unknown method: {method}"),
            }),
        };
        match result {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err(error) => {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":error.code,"message":error.message}})
            }
        }
    }

    fn call_tool(&mut self, name: &str, args: &Value) -> Result<Value, RpcError> {
        match name {
            "setup_company" => self.setup_company(args),
            "login" => self.login(args),
            "logout" => {
                self.authenticated = false;
                self.authenticated_signal.store(false, Ordering::Release);
                self.clear_pending_operations();
                Ok(json!({"logged_out": true}))
            }
            "status" => self.status(),
            "check_updates" => crate::updater::check()
                .map(|update| match update {
                    Some(update) => json!({"available":true,"version":update.version}),
                    None => json!({"available":false}),
                })
                .map_err(RpcError::internal),
            _ => {
                self.require_auth()?;
                if name == "update_preferences" {
                    return self.update_preferences(None);
                }
                if name == "set_update_preferences" {
                    return self.update_preferences(Some(args.clone()));
                }
                self.call_authenticated(name, args)
            }
        }
    }

    fn call_authenticated(&mut self, name: &str, a: &Value) -> Result<Value, RpcError> {
        match name {
            "company_profile" => {
                let company = CompanyService::open(&self.database_path).map_err(company_error)?;
                let profile = company.profile().map_err(company_error)?;
                Ok(profile.map(profile_json).unwrap_or(Value::Null))
            }
            "save_company_settings" => {
                let company = CompanyService::open(&self.database_path).map_err(company_error)?;
                let current = company
                    .profile()
                    .map_err(company_error)?
                    .ok_or_else(|| RpcError::invalid("Complete company setup first"))?;
                let profile = CompanyProfile {
                    name: optional_string(a, "name")?.unwrap_or(&current.name).into(),
                    address: optional_string(a, "address")?
                        .unwrap_or(&current.address)
                        .into(),
                    rules: optional_string(a, "rules")?
                        .unwrap_or(&current.rules)
                        .into(),
                    closing_variance_threshold_cents: optional_integer(
                        a,
                        "variance_threshold_cents",
                    )?
                    .unwrap_or(current.closing_variance_threshold_cents),
                    printer_name: optional_string(a, "printer_name")?
                        .unwrap_or(&current.printer_name)
                        .into(),
                    paper_size: optional_string(a, "paper_size")?
                        .unwrap_or(&current.paper_size)
                        .into(),
                };
                company.save_settings(&profile).map_err(company_error)?;
                Ok(json!({"saved":true}))
            }
            "change_password" => {
                let company = CompanyService::open(&self.database_path).map_err(company_error)?;
                let username = string(a, "username")?;
                let current = string(a, "current_password")?;
                let new = string(a, "new_password")?;
                if company
                    .change_password(username, current, new)
                    .map_err(company_error)?
                {
                    self.authenticated = false;
                    self.authenticated_signal.store(false, Ordering::Release);
                    self.close_preview = None;
                    self.restore_preview = None;
                    self.prepared_update = None;
                    Ok(json!({"changed":true,"authenticated":false}))
                } else {
                    Err(RpcError::invalid("Current credentials were not accepted"))
                }
            }
            "registers" => self.registers(),
            "register_sessions" => self.sessions(a),
            "session_balance" => self.session_balance(a),
            "create_register" => {
                let name = string(a, "name")?;
                let mut db = self.database()?;
                let id = db.create_register(name, "CAD").map_err(db_error)?;
                Ok(json!({"register_id":id,"name":name,"currency":"CAD"}))
            }
            "open_register" => {
                let register_id = integer(a, "register_id")?;
                let date = string(a, "business_date")?;
                let opening = cents(a, "opening_cents")?;
                let mut service = RegisterService::open(&self.database_path).map_err(db_error)?;
                let reply = service.execute(
                    &Operation::Open {
                        register: register_id,
                        date: date.into(),
                        opening,
                    },
                    &Selection::default(),
                );
                if !reply.applied {
                    return Err(RpcError::invalid(
                        reply
                            .snapshot
                            .err()
                            .unwrap_or_else(|| "Unable to open register".into()),
                    ));
                }
                match reply.snapshot {
                    Ok(snapshot) => Ok(json!({"opened":true,"snapshot":snapshot_json(snapshot)})),
                    Err(error) => Ok(json!({"opened":true,"snapshot":null,"refresh_error":error})),
                }
            }
            "record_movement" => self.record_movement(a),
            "register_history" => self.history(a),
            "close_preview" => {
                let session = integer(a, "session_id")?;
                let counted = cents(a, "counted_cents")?;
                let db = self.database()?;
                let balance = db.balance(session).map_err(db_error)?;
                if balance.counted.is_some() {
                    return Err(RpcError::invalid("Session is already closed"));
                }
                let difference = counted.checked_sub(balance.expected).ok_or_else(|| {
                    RpcError::invalid("Closing difference exceeds supported range")
                })?;
                let profile = CompanyService::open(&self.database_path)
                    .map_err(company_error)?
                    .profile()
                    .map_err(company_error)?
                    .ok_or_else(|| RpcError::invalid("Company setup is required"))?;
                let token = uuid::Uuid::new_v4().to_string();
                self.close_preview = Some((token.clone(), session, balance.expected, counted));
                Ok(
                    json!({"preview_token":token,"session_id":session,"opening_cents":balance.opening,"cash_in_cents":balance.cash_in,"cash_out_cents":balance.cash_out,"expected_cents":balance.expected,"counted_cents":counted,"difference_cents":difference,"variance_threshold_cents":profile.closing_variance_threshold_cents,"exceeds_tolerance":difference.unsigned_abs()>profile.closing_variance_threshold_cents as u64,"closed":false}),
                )
            }
            "close_register" => {
                if a.get("confirm").and_then(Value::as_bool) != Some(true) {
                    return Err(RpcError::invalid(
                        "Set confirm=true to close this register session",
                    ));
                }
                let session = integer(a, "session_id")?;
                let token = string(a, "preview_token")?;
                let Some((expected_token, preview_session, expected, counted)) =
                    self.close_preview.take()
                else {
                    return Err(RpcError::invalid(
                        "Run close_preview and review its result first",
                    ));
                };
                if token != expected_token || session != preview_session {
                    return Err(RpcError::invalid(
                        "Close confirmation does not match the latest preview",
                    ));
                }
                let mut db = self.database()?;
                db.close_session_if_unchanged(session, counted, expected)
                    .map_err(db_error)?;
                Ok(
                    json!({"closed":true,"session_id":session,"counted_cents":counted,"expected_cents":expected,"difference_cents":counted.checked_sub(expected).ok_or_else(||RpcError::invalid("Closing difference exceeds supported range"))?}),
                )
            }
            "report" | "export_report_csv" => self.report(name, a),
            "print_report" => self.print_report(a),
            "analytics_dashboard" => self.analytics(a),
            "backup_status" => self.backup_status(),
            "save_backup_settings" => self.save_backup_settings(a),
            "run_backup" => self.run_backup(),
            "restore_preview" => self.restore_preview(a),
            "restore_backup" => self.restore_backup(a),
            "update_status" => {
                let path = a.get("executable_path").and_then(Value::as_str);
                let valid = path
                    .map(|p| {
                        crate::updater::validate_installed_executable(PathBuf::from(p).as_path())
                            .is_ok()
                    })
                    .unwrap_or(false);
                Ok(
                    json!({"installed_target_valid":valid,"last_failure":crate::updater::read_last_failure()}),
                )
            }
            "update_prepare" => {
                let Some(update) = crate::updater::check().map_err(RpcError::internal)? else {
                    return Ok(json!({"available":false}));
                };
                let version = update.version.clone();
                let prepared = crate::updater::download(&update).map_err(RpcError::internal)?;
                self.prepared_update = Some(prepared);
                Ok(json!({"available":true,"version":version,"prepared":true}))
            }
            "update_install" => {
                if a.get("confirm").and_then(Value::as_bool) != Some(true) {
                    return Err(RpcError::invalid(
                        "Set confirm=true to install the prepared update",
                    ));
                }
                let path = PathBuf::from(string(a, "executable_path")?);
                let validated = crate::updater::validate_installed_executable(&path)
                    .map_err(RpcError::invalid)?;
                let prepared = self
                    .prepared_update
                    .as_ref()
                    .ok_or_else(|| RpcError::invalid("Prepare an update first"))?;
                self.authenticated = false;
                self.authenticated_signal.store(false, Ordering::Release);
                let background_operation = self.background_operation.clone();
                let _guard = background_operation
                    .lock()
                    .map_err(|_| RpcError::internal("Background operations are unavailable"))?;
                crate::updater::launch_update_for_executable(prepared, &validated)
                    .map_err(RpcError::internal)?;
                self.exit_after_response = true;
                Ok(
                    json!({"installing":true,"message":"The updater handoff has started; close this MCP process after receiving this response."}),
                )
            }
            _ => Err(RpcError {
                code: -32602,
                message: format!("Unknown tool: {name}"),
            }),
        }
    }

    fn setup_company(&mut self, a: &Value) -> Result<Value, RpcError> {
        let mut company = CompanyService::open(&self.database_path).map_err(company_error)?;
        if company.profile().map_err(company_error)?.is_some() {
            return Err(RpcError::invalid("Company setup is already complete"));
        }
        let profile = profile_from(a)?;
        company
            .setup(SetupCompany {
                profile,
                username: string(a, "username")?.into(),
                password: string(a, "password")?.into(),
            })
            .map_err(company_error)?;
        self.authenticated = true;
        self.authenticated_signal.store(true, Ordering::Release);
        Ok(json!({"configured":true,"authenticated":true}))
    }
    fn login(&mut self, a: &Value) -> Result<Value, RpcError> {
        self.clear_pending_operations();
        self.authenticated = false;
        self.authenticated_signal.store(false, Ordering::Release);
        let company = CompanyService::open(&self.database_path).map_err(company_error)?;
        if company.profile().map_err(company_error)?.is_none() {
            return Err(RpcError::invalid("Complete company setup first"));
        }
        let ok = company
            .authenticate(string(a, "username")?, string(a, "password")?)
            .map_err(company_error)?;
        self.authenticated = ok;
        self.authenticated_signal.store(ok, Ordering::Release);
        if ok {
            Ok(json!({"authenticated":true}))
        } else {
            Err(RpcError::invalid("Credentials were not accepted"))
        }
    }
    fn status(&self) -> Result<Value, RpcError> {
        let profile = CompanyService::open(&self.database_path)
            .map_err(company_error)?
            .profile()
            .map_err(company_error)?;
        Ok(
            json!({"setup_required":profile.is_none(),"authenticated":self.authenticated,"database_path":self.database_path}),
        )
    }
    fn update_preferences(&self, args: Option<Value>) -> Result<Value, RpcError> {
        let path = preference_path().map_err(RpcError::internal)?;
        match args {
            None => Ok(
                json!({"automatic":crate::updater::preferences::read_preference(&path).map_err(RpcError::internal)?}),
            ),
            Some(args) => {
                let automatic = args
                    .get("automatic")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| RpcError::invalid("automatic must be a boolean"))?;
                crate::updater::preferences::write_preference(&path, automatic)
                    .map_err(RpcError::internal)?;
                Ok(json!({"automatic":automatic,"saved":true}))
            }
        }
    }
    fn require_auth(&self) -> Result<(), RpcError> {
        if self.authenticated {
            Ok(())
        } else {
            Err(RpcError {
                code: -32001,
                message: "Login is required".into(),
            })
        }
    }
    fn clear_pending_operations(&mut self) {
        self.close_preview = None;
        self.restore_preview = None;
        self.prepared_update = None;
    }
    fn database(&self) -> Result<Database, RpcError> {
        Database::open(&self.database_path).map_err(db_error)
    }
}

/// Runs the newline-delimited JSON-RPC transport until stdin closes.
pub fn run_stdio() -> io::Result<()> {
    let path = database_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout());
    let mut server = Server::open(path);
    let backup_path = server.database_path.clone();
    let authenticated = server.authenticated_signal.clone();
    let background_operation = server.background_operation.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
            if authenticated.load(Ordering::Acquire) {
                let Ok(_guard) = background_operation.lock() else {
                    continue;
                };
                if !authenticated.load(Ordering::Acquire) {
                    continue;
                }
                if let Err(error) = BackupService::open(&backup_path)
                    .and_then(|mut service| service.run_backup(true, now()).map(|_| ()))
                {
                    eprintln!("Cash Ledger scheduled backup failed: {error}");
                }
            }
        }
    });
    for line in stdin.lock().lines() {
        let line = line?;
        if line.len() > 8 * 1024 * 1024 {
            serde_json::to_writer(
                &mut stdout,
                &json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Message exceeds 8 MiB limit"}}),
            )?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(request) => server.dispatch(&request),
            Err(error) => {
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":format!("Parse error: {error}")}})
            }
        };
        if !response.is_null() {
            serde_json::to_writer(&mut stdout, &response)?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;
        }
        if server.exit_after_response {
            break;
        }
    }
    Ok(())
}

fn database_path() -> io::Result<PathBuf> {
    crate::paths::database_path().map_err(io::Error::other)
}
fn preference_path() -> Result<PathBuf, String> {
    crate::updater::preferences::update_preference_path()
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
}
fn db_error(e: impl std::fmt::Display) -> RpcError {
    RpcError::internal(e.to_string())
}
fn company_error(e: impl std::fmt::Display) -> RpcError {
    RpcError::internal(e.to_string())
}
fn string<'a>(a: &'a Value, key: &str) -> Result<&'a str, RpcError> {
    a.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| RpcError::invalid(format!("{key} must be a non-empty string")))
}
fn integer(a: &Value, key: &str) -> Result<i64, RpcError> {
    a.get(key)
        .and_then(Value::as_i64)
        .filter(|n| *n > 0)
        .ok_or_else(|| RpcError::invalid(format!("{key} must be a positive integer")))
}
fn optional_integer(a: &Value, key: &str) -> Result<Option<i64>, RpcError> {
    match a.get(key) {
        None => Ok(None),
        Some(Value::Number(n)) => n
            .as_i64()
            .map(Some)
            .ok_or_else(|| RpcError::invalid(format!("{key} must be an integer"))),
        Some(_) => Err(RpcError::invalid(format!("{key} must be an integer"))),
    }
}
fn optional_string<'a>(a: &'a Value, key: &str) -> Result<Option<&'a str>, RpcError> {
    match a.get(key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(RpcError::invalid(format!("{key} must be a string"))),
    }
}
fn cents(a: &Value, key: &str) -> Result<i64, RpcError> {
    a.get(key)
        .and_then(Value::as_i64)
        .filter(|n| *n >= 0)
        .ok_or_else(|| {
            RpcError::invalid(format!(
                "{key} must be a non-negative integer number of cents"
            ))
        })
}
fn profile_from(a: &Value) -> Result<CompanyProfile, RpcError> {
    Ok(CompanyProfile {
        name: string(a, "name")?.into(),
        address: a
            .get("address")
            .and_then(Value::as_str)
            .unwrap_or("")
            .into(),
        rules: a.get("rules").and_then(Value::as_str).unwrap_or("").into(),
        closing_variance_threshold_cents: a
            .get("variance_threshold_cents")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        printer_name: a
            .get("printer_name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .into(),
        paper_size: a
            .get("paper_size")
            .and_then(Value::as_str)
            .unwrap_or("A4")
            .into(),
    })
}
fn profile_json(p: CompanyProfile) -> Value {
    json!({"name":p.name,"address":p.address,"rules":p.rules,"variance_threshold_cents":p.closing_variance_threshold_cents,"printer_name":p.printer_name,"paper_size":p.paper_size})
}
fn snapshot_json(s: crate::register::Snapshot) -> Value {
    json!({"registers":s.registers.iter().map(|r|json!({"id":r.id,"name":r.name,"currency":r.currency})).collect::<Vec<_>>(),"sessions":s.sessions.iter().map(|v|json!({"id":v.id,"register_id":v.register_id,"business_date":v.business_date,"closed":v.closed})).collect::<Vec<_>>(),"active_session_id":s.active_session.map(|v|v.id),"selected_session_id":s.session.as_ref().map(|v|v.id),"balance":s.balance.map(|b|json!({"opening_cents":b.opening,"cash_in_cents":b.cash_in,"cash_out_cents":b.cash_out,"expected_cents":b.expected,"counted_cents":b.counted,"difference_cents":b.difference})),"history":s.entries.iter().map(|e|json!({"id":e.id,"amount_cents":e.amount,"reason":e.reason,"request_key":e.request_key,"created_at":e.created_at})).collect::<Vec<_>>(),"more_history":s.more_entries,"more_sessions":s.more_sessions})
}
fn backup_json(s: &crate::backup_service::BackupStatus) -> Value {
    json!({"settings":{"directory":s.settings.directory,"mode":if s.settings.mode==BackupMode::Manual{"manual"}else{"automatic"},"frequency":match s.settings.frequency{BackupFrequency::Hourly=>"hourly",BackupFrequency::EverySixHours=>"six_hours",BackupFrequency::Daily=>"daily",BackupFrequency::Weekly=>"weekly"}},"last_success_at":s.last_success_at,"last_backup_path":s.last_backup_path,"next_due_at":s.next_due_at,"last_error":s.last_error})
}

mod backup;
mod ledger;
mod restore_ops;
mod schema;
