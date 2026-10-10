//! Bounded, currency-specific dashboard analytics from an independent read snapshot.
use chrono::{NaiveDate, Utc};
use rusqlite::{Connection, OpenFlags, params};
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub use crate::reporting::{Error, Result};

/// Inclusive business-date bounds; monetary totals never combine currencies.
#[derive(Debug, Clone)]
pub struct DashboardFilter {
    pub start_date: String,
    pub end_date: String,
    pub currency: String,
    pub register_id: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct DashboardSummary {
    pub sessions: i64,
    pub open_sessions: i64,
    pub closed_sessions: i64,
    pub movement_count: i64,
    pub cash_in: i64,
    pub cash_out: i64,
    pub net_flow: i64,
    pub discrepancy_count: i64,
    /// Signed sum of closing differences, not an absolute exposure amount.
    pub discrepancy_total: i64,
}

#[derive(Debug, Clone)]
pub struct DailyFlow {
    pub date: String,
    pub cash_in: i64,
    pub cash_out: i64,
    pub net_flow: i64,
    pub movement_count: i64,
}

#[derive(Debug, Clone)]
pub struct RegisterRanking {
    pub register_id: i64,
    pub register_name: String,
    pub cash_in: i64,
    pub cash_out: i64,
    pub net_flow: i64,
    pub sessions: i64,
}

#[derive(Debug, Clone)]
pub struct Discrepancy {
    pub session_id: i64,
    pub register_name: String,
    pub business_date: String,
    pub expected: i64,
    pub counted: i64,
    pub difference: i64,
}

/// Every result belongs to the same SQLite snapshot. Detail lists are bounded.
#[derive(Debug, Clone)]
pub struct DashboardSnapshot {
    pub filter: DashboardFilter,
    pub summary: DashboardSummary,
    pub daily_flow: Vec<DailyFlow>,
    /// Top ten registers by cash received, with stable ID tie breaking.
    pub register_rankings: Vec<RegisterRanking>,
    /// Twenty largest closing discrepancies by magnitude.
    pub discrepancies: Vec<Discrepancy>,
    pub query_duration: Duration,
    pub generated_at: String,
}

fn validate(filter: &DashboardFilter) -> Result<(NaiveDate, NaiveDate)> {
    let parse = |value: &str| {
        NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .ok()
            .filter(|date| date.format("%Y-%m-%d").to_string() == value)
    };
    let (Some(start), Some(end)) = (parse(&filter.start_date), parse(&filter.end_date)) else {
        return Err(Error::InvalidInput("Use dates in YYYY-MM-DD format"));
    };
    if start > end || (end - start).num_days() >= 366 {
        return Err(Error::InvalidInput(
            "Select a dashboard range of at most 366 days",
        ));
    }
    if filter.currency.len() != 3
        || !filter
            .currency
            .bytes()
            .all(|byte| byte.is_ascii_uppercase())
        || filter.register_id.is_some_and(|id| id <= 0)
    {
        return Err(Error::InvalidInput(
            "Invalid dashboard currency or register",
        ));
    }
    Ok((start, end))
}

/// Reads aggregate ledger activity without loading individual movements into memory.
/// Call on a background worker; this connection never migrates or writes the ledger.
pub fn load_dashboard(path: &Path, filter: &DashboardFilter) -> Result<DashboardSnapshot> {
    let (start, end) = validate(filter)?;
    let started = Instant::now();
    let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(Duration::from_secs(2))?;
    connection.pragma_update(None, "query_only", true)?;
    // A large ledger must not monopolize a worker indefinitely. SQLite checks
    // elapsed time every 10,000 VM instructions; busy waits have a separate limit.
    connection.progress_handler(
        10_000,
        Some(move || started.elapsed() >= Duration::from_secs(10)),
    )?;
    let transaction = connection.transaction()?;
    // Select only session summaries. Correlated counts use the existing
    // session indexes; joining both movement tables directly would multiply rows.
    let selection = "WITH selected AS (
        SELECT s.*, r.name register_name,
          (SELECT count(*) FROM cash_in WHERE session_id=s.id) +
          (SELECT count(*) FROM cash_out WHERE session_id=s.id) movement_count
        FROM register_sessions s JOIN registers r ON r.id=s.register_id
        WHERE s.business_date BETWEEN ?1 AND ?2 AND r.currency=?3
          AND (?4 IS NULL OR s.register_id=?4)) ";
    let bind = params![
        filter.start_date,
        filter.end_date,
        filter.currency,
        filter.register_id
    ];
    let summary = transaction.query_row(&format!("{selection}
        SELECT count(*),coalesce(sum(closed_at IS NULL),0),coalesce(sum(closed_at IS NOT NULL),0),
        coalesce(sum(movement_count),0),coalesce(sum(cash_in_total),0),coalesce(sum(cash_out_total),0),
        coalesce(sum(cash_in_total-cash_out_total),0),coalesce(sum(difference IS NOT NULL AND difference<>0),0),
        coalesce(sum(difference),0) FROM selected"), bind, |row| Ok(DashboardSummary {
            sessions:row.get(0)?,open_sessions:row.get(1)?,closed_sessions:row.get(2)?,movement_count:row.get(3)?,
            cash_in:row.get(4)?,cash_out:row.get(5)?,net_flow:row.get(6)?,discrepancy_count:row.get(7)?,discrepancy_total:row.get(8)?,
        }))?;
    let grouped = {
        let mut statement = transaction.prepare(&format!("{selection}
            SELECT business_date,sum(cash_in_total),sum(cash_out_total),sum(cash_in_total-cash_out_total),sum(movement_count)
            FROM selected GROUP BY business_date ORDER BY business_date LIMIT 366"))?;
        statement
            .query_map(bind, |row| {
                Ok(DailyFlow {
                    date: row.get(0)?,
                    cash_in: row.get(1)?,
                    cash_out: row.get(2)?,
                    net_flow: row.get(3)?,
                    movement_count: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut grouped = grouped.into_iter().peekable();
    let mut daily_flow = Vec::with_capacity(((end - start).num_days() + 1) as usize);
    for date in start.iter_days().take_while(|date| *date <= end) {
        let date = date.format("%Y-%m-%d").to_string();
        if grouped.peek().is_some_and(|row| row.date == date) {
            if let Some(row) = grouped.next() {
                daily_flow.push(row);
            }
        } else {
            daily_flow.push(DailyFlow {
                date,
                cash_in: 0,
                cash_out: 0,
                net_flow: 0,
                movement_count: 0,
            });
        }
    }
    let register_rankings = {
        let mut statement = transaction.prepare(&format!("{selection}
            SELECT register_id,register_name,sum(cash_in_total),sum(cash_out_total),sum(cash_in_total-cash_out_total),count(*)
            FROM selected GROUP BY register_id ORDER BY sum(cash_in_total) DESC,register_id LIMIT 10"))?;
        statement
            .query_map(bind, |row| {
                Ok(RegisterRanking {
                    register_id: row.get(0)?,
                    register_name: row.get(1)?,
                    cash_in: row.get(2)?,
                    cash_out: row.get(3)?,
                    net_flow: row.get(4)?,
                    sessions: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let discrepancies = {
        let mut statement = transaction.prepare(&format!(
            "{selection}
            SELECT id,register_name,business_date,expected,counted,difference FROM selected
            WHERE difference IS NOT NULL AND difference<>0
            ORDER BY abs(difference) DESC,business_date DESC,id DESC LIMIT 20"
        ))?;
        statement
            .query_map(bind, |row| {
                Ok(Discrepancy {
                    session_id: row.get(0)?,
                    register_name: row.get(1)?,
                    business_date: row.get(2)?,
                    expected: row.get(3)?,
                    counted: row.get(4)?,
                    difference: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    transaction.commit()?;
    Ok(DashboardSnapshot {
        filter: filter.clone(),
        summary,
        daily_flow,
        register_rankings,
        discrepancies,
        query_duration: started.elapsed(),
        generated_at: Utc::now().to_rfc3339(),
    })
}

#[cfg(test)]
mod tests;
