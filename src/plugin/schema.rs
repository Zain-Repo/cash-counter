use super::RpcError;
use serde_json::{Value, json};

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
pub(super) fn validate_arguments(name: &str, args: &Value) -> Result<(), RpcError> {
    let Some(args) = args.as_object() else {
        return Err(RpcError::invalid("Tool arguments must be a JSON object"));
    };
    let definition = tools()
        .into_iter()
        .find(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
        .ok_or_else(|| RpcError::invalid(format!("Unknown tool: {name}")))?;
    let schema = definition
        .get("inputSchema")
        .ok_or_else(|| RpcError::invalid("Tool schema is unavailable"))?;
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| RpcError::invalid("Tool schema is invalid"))?;
    if let Some(key) = args
        .keys()
        .find(|key| !properties.contains_key(key.as_str()))
    {
        return Err(RpcError::invalid(format!("Unknown argument: {key}")));
    }
    for key in schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if !args.contains_key(key) {
            return Err(RpcError::invalid(format!(
                "Missing required argument: {key}"
            )));
        }
    }
    for (key, value) in args {
        let spec = &properties[key];
        let kind = spec.get("type").and_then(Value::as_str).unwrap_or("");
        let valid = match kind {
            "string" => value.is_string(),
            "integer" => value.as_i64().is_some(),
            "boolean" => value.is_boolean(),
            _ => true,
        };
        if !valid {
            return Err(RpcError::invalid(format!("{key} must be {kind}")));
        }
        if let Some(options) = spec.get("enum").and_then(Value::as_array)
            && !options.contains(value)
        {
            return Err(RpcError::invalid(format!("{key} has an unsupported value")));
        }
        if let Some(number) = value.as_i64()
            && (spec
                .get("minimum")
                .and_then(Value::as_i64)
                .is_some_and(|minimum| number < minimum)
                || spec
                    .get("maximum")
                    .and_then(Value::as_i64)
                    .is_some_and(|maximum| number > maximum))
        {
            return Err(RpcError::invalid(format!(
                "{key} is outside the supported range"
            )));
        }
    }
    Ok(())
}
fn tool(name: &str, description: &str, input: Value) -> Value {
    json!({"name":name,"description":description,"inputSchema":input})
}
fn strprop(description: &str) -> Value {
    json!({"type":"string","description":description})
}
fn intprop(description: &str) -> Value {
    json!({"type":"integer","description":description})
}
pub(super) fn tools() -> Vec<Value> {
    vec![
        tool(
            "setup_company",
            "Create the company profile and administrator. Setup authenticates this MCP process.",
            schema(
                json!({"name":strprop("Company name"),"address":strprop("Company address"),"rules":strprop("Cash handling rules"),"variance_threshold_cents":intprop("Closing variance threshold in integer cents"),"printer_name":strprop("Printer name, empty for system default"),"paper_size":{"type":"string","enum":["A4","Letter"]},"username":strprop("Administrator username"),"password":strprop("Administrator password")}),
                &["name", "username", "password"],
            ),
        ),
        tool(
            "login",
            "Authenticate this MCP process.",
            schema(
                json!({"username":strprop("Username"),"password":strprop("Password")}),
                &["username", "password"],
            ),
        ),
        tool(
            "logout",
            "End this process's authenticated session.",
            schema(json!({}), &[]),
        ),
        tool(
            "check_updates",
            "Check the official stable release metadata for an update.",
            schema(json!({}), &[]),
        ),
        tool(
            "status",
            "Read setup and authentication status without exposing ledger data.",
            schema(json!({}), &[]),
        ),
        tool(
            "company_profile",
            "Read company and printing settings.",
            schema(json!({}), &[]),
        ),
        tool(
            "save_company_settings",
            "Update supplied company and printing settings; omitted fields keep their saved values.",
            schema(
                json!({"name":strprop("Company name"),"address":strprop("Address"),"rules":strprop("Cash rules"),"variance_threshold_cents":intprop("Integer cents"),"printer_name":strprop("Printer name"),"paper_size":{"type":"string","enum":["A4","Letter"]}}),
                &[],
            ),
        ),
        tool(
            "change_password",
            "Change the administrator password after verifying the current password.",
            schema(
                json!({"username":strprop("Username"),"current_password":strprop("Current password"),"new_password":strprop("New password")}),
                &["username", "current_password", "new_password"],
            ),
        ),
        tool(
            "registers",
            "List register metadata.",
            schema(json!({}), &[]),
        ),
        tool(
            "register_sessions",
            "List a register's sessions using a descending ID cursor. Follow next_before_id when has_more is true.",
            schema(
                json!({"register_id":intprop("Register ID"),"before_id":intprop("Exclusive session ID cursor"),"limit":{"type":"integer","minimum":1,"maximum":999}}),
                &["register_id"],
            ),
        ),
        tool(
            "session_balance",
            "Read the exact session balance in integer cents.",
            schema(json!({"session_id":intprop("Session ID")}), &["session_id"]),
        ),
        tool(
            "create_register",
            "Create a CAD register.",
            schema(json!({"name":strprop("Register name")}), &["name"]),
        ),
        tool(
            "open_register",
            "Open a register for a business date with an integer-cent opening count.",
            schema(
                json!({"register_id":intprop("Register ID"),"business_date":strprop("YYYY-MM-DD local business date"),"opening_cents":intprop("Non-negative integer cents")}),
                &["register_id", "business_date", "opening_cents"],
            ),
        ),
        tool(
            "record_movement",
            "Record an inbound or outbound movement. Reuse request_key for retries.",
            schema(
                json!({"session_id":intprop("Open session ID"),"direction":{"type":"string","enum":["in","out"]},"amount_cents":intprop("Positive integer cents"),"reason":strprop("Movement reason"),"request_key":strprop("Unique idempotency key")}),
                &[
                    "session_id",
                    "direction",
                    "amount_cents",
                    "reason",
                    "request_key",
                ],
            ),
        ),
        tool(
            "register_history",
            "Read a cursor-paginated movement history page. Follow next_after_id when has_more is true.",
            schema(
                json!({"session_id":intprop("Session ID"),"direction":{"type":"string","enum":["in","out"]},"after_id":intprop("Exclusive cursor, default 0"),"limit":{"type":"integer","minimum":1,"maximum":999}}),
                &["session_id"],
            ),
        ),
        tool(
            "close_preview",
            "Preview a closing using the physical count. Review expected balance and difference before confirmation.",
            schema(
                json!({"session_id":intprop("Session ID"),"counted_cents":intprop("Physical count in cents")}),
                &["session_id", "counted_cents"],
            ),
        ),
        tool(
            "close_register",
            "Confirm the latest close preview using its token.",
            schema(
                json!({"session_id":intprop("Session ID"),"preview_token":strprop("Token returned by close_preview"),"confirm":{"type":"boolean"}}),
                &["session_id", "preview_token", "confirm"],
            ),
        ),
        tool(
            "report",
            "Build a bounded date-range report with session and movement detail.",
            schema(
                json!({"start_date":strprop("YYYY-MM-DD"),"end_date":strprop("YYYY-MM-DD"),"register_id":intprop("Optional register ID")}),
                &["start_date", "end_date"],
            ),
        ),
        tool(
            "print_report",
            "Print a bounded report on the configured printer after explicit confirmation.",
            schema(
                json!({"start_date":strprop("YYYY-MM-DD"),"end_date":strprop("YYYY-MM-DD"),"register_id":intprop("Optional register ID"),"confirm":{"type":"boolean"}}),
                &["start_date", "end_date", "confirm"],
            ),
        ),
        tool(
            "export_report_csv",
            "Export the report as CSV to a .csv destination.",
            schema(
                json!({"start_date":strprop("YYYY-MM-DD"),"end_date":strprop("YYYY-MM-DD"),"register_id":intprop("Optional register ID"),"destination":strprop("CSV output path")}),
                &["start_date", "end_date", "destination"],
            ),
        ),
        tool(
            "analytics_dashboard",
            "Read bounded analytics for one currency and date range.",
            schema(
                json!({"start_date":strprop("YYYY-MM-DD"),"end_date":strprop("YYYY-MM-DD"),"currency":strprop("Three-letter uppercase currency"),"register_id":intprop("Optional register ID")}),
                &["start_date", "end_date", "currency"],
            ),
        ),
        tool(
            "backup_status",
            "Read backup policy and recent backup status.",
            schema(json!({}), &[]),
        ),
        tool(
            "save_backup_settings",
            "Save backup destination and schedule. Automatic mode starts due immediately.",
            schema(
                json!({"directory":strprop("Existing backup folder"),"clear_directory":{"type":"boolean","description":"Clear the saved folder when mode is manual"},"mode":{"type":"string","enum":["manual","automatic"]},"frequency":{"type":"string","enum":["hourly","six_hours","daily","weekly"]}}),
                &["mode", "frequency"],
            ),
        ),
        tool(
            "run_backup",
            "Create a manual verified backup now.",
            schema(json!({}), &[]),
        ),
        tool(
            "restore_preview",
            "Resolve and fully validate a backup without changing the live ledger.",
            schema(json!({"path":strprop("Backup file or folder")}), &["path"]),
        ),
        tool(
            "restore_backup",
            "Restore the exact previewed backup. Explicit confirm=true and preview_token are required.",
            schema(
                json!({"path":strprop("Backup file path returned by restore_preview"),"preview_token":strprop("Token returned by restore_preview"),"confirm":{"type":"boolean"}}),
                &["path", "preview_token", "confirm"],
            ),
        ),
        tool(
            "update_preferences",
            "Read the desktop automatic update preference.",
            schema(json!({}), &[]),
        ),
        tool(
            "set_update_preferences",
            "Persist the desktop automatic update preference.",
            schema(json!({"automatic":{"type":"boolean"}}), &["automatic"]),
        ),
        tool(
            "update_status",
            "Read desktop installation status and the last updater failure.",
            schema(
                json!({"executable_path":strprop("Optional installed Cash Ledger executable path to validate")}),
                &[],
            ),
        ),
        tool(
            "update_prepare",
            "Check for and download an official verified update installer.",
            schema(json!({}), &[]),
        ),
        tool(
            "update_install",
            "Hand off the prepared installer to the validated installed desktop app. Close the desktop app before calling this tool; the MCP server exits after replying.",
            schema(
                json!({"executable_path":strprop("Installed Cash Ledger executable path"),"confirm":{"type":"boolean"}}),
                &["executable_path", "confirm"],
            ),
        ),
    ]
}
