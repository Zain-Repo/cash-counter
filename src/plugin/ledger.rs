use super::*;

impl Server {
    pub(super) fn registers(&self) -> Result<Value, RpcError> {
        let db = self.database()?;
        let rows = db
            .registers()
            .map_err(db_error)?
            .into_iter()
            .map(|r| json!({"id":r.id,"name":r.name,"currency":r.currency}))
            .collect::<Vec<_>>();
        Ok(json!({"registers":rows}))
    }
    pub(super) fn sessions(&self, a: &Value) -> Result<Value, RpcError> {
        let register = integer(a, "register_id")?;
        let before = optional_integer(a, "before_id")?;
        let limit = optional_integer(a, "limit")?.unwrap_or(25);
        if !(1..=999).contains(&limit) {
            return Err(RpcError::invalid("limit must be from 1 to 999"));
        }
        let db = self.database()?;
        let mut rows = db
            .sessions(
                register,
                before,
                u32::try_from(limit + 1).map_err(|_| RpcError::invalid("Invalid limit"))?,
            )
            .map_err(db_error)?;
        let more = rows.len() > limit as usize;
        rows.truncate(limit as usize);
        let next_before_id = rows.last().map(|row| row.id);
        Ok(
            json!({"register_id":register,"sessions":rows.iter().map(|s|json!({"id":s.id,"business_date":s.business_date,"closed":s.closed})).collect::<Vec<_>>(),"has_more":more,"next_before_id":next_before_id}),
        )
    }
    pub(super) fn session_balance(&self, a: &Value) -> Result<Value, RpcError> {
        let session = integer(a, "session_id")?;
        let db = self.database()?;
        let b = db.balance(session).map_err(db_error)?;
        Ok(
            json!({"session_id":session,"opening_cents":b.opening,"cash_in_cents":b.cash_in,"cash_out_cents":b.cash_out,"expected_cents":b.expected,"counted_cents":b.counted,"difference_cents":b.difference}),
        )
    }
    pub(super) fn record_movement(&self, a: &Value) -> Result<Value, RpcError> {
        let session = integer(a, "session_id")?;
        let direction = match string(a, "direction")? {
            "in" => Direction::In,
            "out" => Direction::Out,
            _ => return Err(RpcError::invalid("direction must be 'in' or 'out'")),
        };
        let amount = cents(a, "amount_cents")?;
        let reason = string(a, "reason")?;
        let key = string(a, "request_key")?;
        let mut db = self.database()?;
        let ids = db
            .record_entries(
                session,
                &[NewEntry {
                    direction,
                    amount,
                    reason,
                    request_key: key,
                }],
            )
            .map_err(db_error)?;
        Ok(
            json!({"entry_id":ids.first(),"session_id":session,"direction":if direction==Direction::In{"in"}else{"out"},"amount_cents":amount}),
        )
    }
    pub(super) fn history(&self, a: &Value) -> Result<Value, RpcError> {
        let session = integer(a, "session_id")?;
        let direction = match a.get("direction").and_then(Value::as_str).unwrap_or("in") {
            "in" => Direction::In,
            "out" => Direction::Out,
            _ => return Err(RpcError::invalid("direction must be 'in' or 'out'")),
        };
        let after = optional_integer(a, "after_id")?.unwrap_or(0);
        let limit = optional_integer(a, "limit")?.unwrap_or(25);
        if after < 0 || !(1..=999).contains(&limit) {
            return Err(RpcError::invalid(
                "after_id must be non-negative and limit must be from 1 to 999",
            ));
        }
        let db = self.database()?;
        let mut rows = db
            .entries(
                session,
                direction,
                after,
                u32::try_from(limit + 1).map_err(|_| RpcError::invalid("Invalid limit"))?,
            )
            .map_err(db_error)?;
        let more = rows.len() > limit as usize;
        rows.truncate(limit as usize);
        let next_after_id = rows.last().map(|row| row.id);
        Ok(
            json!({"session_id":session,"direction":if direction==Direction::In{"in"}else{"out"},"entries":rows.iter().map(|e|json!({"id":e.id,"amount_cents":e.amount,"reason":e.reason,"request_key":e.request_key,"created_at":e.created_at})).collect::<Vec<_>>(),"has_more":more,"next_after_id":next_after_id}),
        )
    }
    pub(super) fn report(&self, name: &str, a: &Value) -> Result<Value, RpcError> {
        let filter = ReportFilter {
            start_date: string(a, "start_date")?.into(),
            end_date: string(a, "end_date")?.into(),
            register_id: optional_integer(a, "register_id")?,
        };
        let db = self.database()?;
        let report =
            reporting::report(&db, &filter).map_err(|e| RpcError::invalid(e.to_string()))?;
        if name == "export_report_csv" {
            let path = PathBuf::from(string(a, "destination")?);
            reporting::export_csv(&report, &path, &self.database_path)
                .map_err(|e| RpcError::internal(e.to_string()))?;
            return Ok(
                json!({"exported":true,"path":path,"sessions":report.sessions.len(),"movements":report.movements.len()}),
            );
        }
        Ok(
            json!({"filter":{"start_date":filter.start_date,"end_date":filter.end_date,"register_id":filter.register_id},"sessions":report.sessions.iter().map(|s|json!({"id":s.id,"register_name":s.register_name,"currency":s.currency,"business_date":s.business_date,"opening_cents":s.opening,"cash_in_cents":s.cash_in,"cash_out_cents":s.cash_out,"expected_cents":s.expected,"counted_cents":s.counted,"difference_cents":s.difference})).collect::<Vec<_>>(),"movements":report.movements.iter().map(|m|json!({"session_id":m.session_id,"direction":m.direction,"amount_cents":m.amount,"reason":m.reason,"created_at":m.created_at})).collect::<Vec<_>>()}),
        )
    }
    pub(super) fn print_report(&self, a: &Value) -> Result<Value, RpcError> {
        if a.get("confirm").and_then(Value::as_bool) != Some(true) {
            return Err(RpcError::invalid(
                "Set confirm=true to send this report to the selected printer",
            ));
        }
        let filter = ReportFilter {
            start_date: string(a, "start_date")?.into(),
            end_date: string(a, "end_date")?.into(),
            register_id: optional_integer(a, "register_id")?,
        };
        let db = self.database()?;
        let report =
            reporting::report(&db, &filter).map_err(|e| RpcError::invalid(e.to_string()))?;
        let profile = CompanyService::open(&self.database_path)
            .map_err(company_error)?
            .profile()
            .map_err(company_error)?
            .ok_or_else(|| RpcError::invalid("Company setup is required"))?;
        reporting::print_report_with_paper(
            &report.printable(&profile.name),
            (!profile.printer_name.is_empty()).then_some(profile.printer_name.as_str()),
            &profile.paper_size,
        )
        .map_err(|e| RpcError::internal(e.to_string()))?;
        Ok(
            json!({"printed":true,"sessions":report.sessions.len(),"movements":report.movements.len()}),
        )
    }
    pub(super) fn analytics(&self, a: &Value) -> Result<Value, RpcError> {
        let f = DashboardFilter {
            start_date: string(a, "start_date")?.into(),
            end_date: string(a, "end_date")?.into(),
            currency: string(a, "currency")?.into(),
            register_id: optional_integer(a, "register_id")?,
        };
        let x = analytics::load_dashboard(&self.database_path, &f)
            .map_err(|e| RpcError::invalid(e.to_string()))?;
        Ok(
            json!({"filter":{"start_date":f.start_date,"end_date":f.end_date,"currency":f.currency,"register_id":f.register_id},"summary":{"sessions":x.summary.sessions,"open_sessions":x.summary.open_sessions,"closed_sessions":x.summary.closed_sessions,"movement_count":x.summary.movement_count,"cash_in_cents":x.summary.cash_in,"cash_out_cents":x.summary.cash_out,"net_flow_cents":x.summary.net_flow,"discrepancy_count":x.summary.discrepancy_count,"discrepancy_total_cents":x.summary.discrepancy_total},"daily_flow":x.daily_flow.iter().map(|d|json!({"date":d.date,"cash_in_cents":d.cash_in,"cash_out_cents":d.cash_out,"net_flow_cents":d.net_flow,"movement_count":d.movement_count})).collect::<Vec<_>>(),"register_rankings":x.register_rankings.iter().map(|r|json!({"register_id":r.register_id,"register_name":r.register_name,"cash_in_cents":r.cash_in,"cash_out_cents":r.cash_out,"net_flow_cents":r.net_flow,"sessions":r.sessions})).collect::<Vec<_>>(),"discrepancies":x.discrepancies.iter().map(|d|json!({"session_id":d.session_id,"register_name":d.register_name,"business_date":d.business_date,"expected_cents":d.expected,"counted_cents":d.counted,"difference_cents":d.difference})).collect::<Vec<_>>(),"generated_at":x.generated_at,"query_ms":x.query_duration.as_millis()}),
        )
    }
}
