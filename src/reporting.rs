//! Consistent ledger snapshots, portable exports, and native Windows printing.
use crate::database::Database;
use chrono::NaiveDate;
use rusqlite::params;

#[derive(Debug)]
pub enum Error {
    Database(rusqlite::Error),
    InvalidInput(&'static str),
    Printing(&'static str),
    Export(std::io::Error),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(_) => f.write_str("Unable to read the report"),
            Self::Export(_) => f.write_str("Unable to save the report"),
            Self::InvalidInput(s) | Self::Printing(s) => f.write_str(s),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::Export(error) => Some(error),
            _ => None,
        }
    }
}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::Database(e)
    }
}
pub type Result<T> = std::result::Result<T, Error>;

/// Publishes a complete CSV beside its destination, preserving the ledger and
/// its sidecars even when an existing file is selected in the save dialog.
pub fn export_csv(
    report: &Report,
    destination: &std::path::Path,
    database_path: &std::path::Path,
) -> Result<()> {
    use std::{
        fs,
        io::{Read, Write},
    };
    if !destination
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("csv"))
    {
        return Err(Error::InvalidInput(
            "Reports must be saved with a .csv extension",
        ));
    }
    let resolved = |path: &std::path::Path| -> std::io::Result<std::path::PathBuf> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        let name = path.file_name().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "Missing filename")
        })?;
        Ok(parent.canonicalize()?.join(name))
    };
    let target = resolved(destination).map_err(Error::Export)?;
    let ledger = resolved(database_path).map_err(Error::Export)?;
    let equal = |a: &std::path::Path, b: &std::path::Path| {
        #[cfg(windows)]
        {
            a.to_string_lossy()
                .eq_ignore_ascii_case(&b.to_string_lossy())
        }
        #[cfg(not(windows))]
        {
            a == b
        }
    };
    for protected in [
        ledger.clone(),
        std::path::PathBuf::from(format!("{}-wal", ledger.display())),
        std::path::PathBuf::from(format!("{}-shm", ledger.display())),
    ] {
        if equal(&target, &protected) {
            return Err(Error::InvalidInput(
                "A report cannot replace the application database",
            ));
        }
    }
    if let Ok(metadata) = fs::symlink_metadata(&target) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(Error::InvalidInput("Select a regular CSV destination"));
        }
        let mut header = [0u8; 16];
        let mut existing = fs::File::open(&target).map_err(Error::Export)?;
        let has_header = match existing.read_exact(&mut header) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => false,
            Err(error) => return Err(Error::Export(error)),
        };
        if has_header && &header == b"SQLite format 3\0" {
            return Err(Error::InvalidInput(
                "A report cannot replace a SQLite database",
            ));
        }
        if let Ok(canonical) = target.canonicalize()
            && equal(&canonical, &ledger)
        {
            return Err(Error::InvalidInput(
                "A report cannot replace the application database",
            ));
        }
    }
    let parent = target
        .parent()
        .ok_or(Error::InvalidInput("Invalid CSV destination"))?;
    let temporary = parent.join(format!(".cash-report-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(Error::Export)?;
    let write = || -> std::io::Result<()> {
        // Buffer small writes without allocating a second copy of the complete report.
        let mut buffered = std::io::BufWriter::new(&mut file);
        report.write_csv(&mut buffered)?;
        buffered.flush()?;
        drop(buffered);
        file.sync_all()?;
        drop(file);
        // Same-directory rename atomically replaces the file, never its contents.
        fs::rename(&temporary, &target)
    };
    let result = write();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(Error::Export)
}

#[derive(Debug, Clone)]
pub struct ReportFilter {
    pub start_date: String,
    pub end_date: String,
    pub register_id: Option<i64>,
}
#[derive(Debug, Clone)]
pub struct SessionReport {
    pub id: i64,
    pub register_name: String,
    pub currency: String,
    pub business_date: String,
    pub opening: i64,
    pub cash_in: i64,
    pub cash_out: i64,
    pub expected: i64,
    pub counted: Option<i64>,
    pub difference: Option<i64>,
}
#[derive(Debug, Clone)]
pub struct MovementReport {
    pub session_id: i64,
    pub direction: String,
    pub amount: i64,
    pub reason: String,
    pub created_at: String,
}
#[derive(Debug, Clone)]
pub struct Report {
    pub filter: ReportFilter,
    pub sessions: Vec<SessionReport>,
    pub movements: Vec<MovementReport>,
}

/// Date bounds select business dates, including both endpoints. A transaction keeps
/// session totals and movement detail in the same SQLite read snapshot.
pub fn report(db: &Database, filter: &ReportFilter) -> Result<Report> {
    let parse = |s: &str| {
        NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .ok()
            .filter(|d| d.format("%Y-%m-%d").to_string() == s)
    };
    let (Some(start), Some(end)) = (parse(&filter.start_date), parse(&filter.end_date)) else {
        return Err(Error::InvalidInput("Use dates in YYYY-MM-DD format"));
    };
    if start > end || filter.register_id.is_some_and(|id| id <= 0) {
        return Err(Error::InvalidInput("Invalid report date range or register"));
    }
    let connection = db.report_connection();
    let transaction = connection.unchecked_transaction()?;
    let sessions = {
        let mut statement=transaction.prepare("SELECT s.id,r.name,r.currency,s.business_date,s.opening,s.cash_in_total,s.cash_out_total,s.expected,s.counted,s.difference FROM register_sessions s JOIN registers r ON r.id=s.register_id WHERE s.business_date BETWEEN ?1 AND ?2 AND (?3 IS NULL OR s.register_id=?3) ORDER BY s.business_date,s.id LIMIT 10001")?;
        statement
            .query_map(
                params![filter.start_date, filter.end_date, filter.register_id],
                |r| {
                    Ok(SessionReport {
                        id: r.get(0)?,
                        register_name: r.get(1)?,
                        currency: r.get(2)?,
                        business_date: r.get(3)?,
                        opening: r.get(4)?,
                        cash_in: r.get(5)?,
                        cash_out: r.get(6)?,
                        expected: r.get(7)?,
                        counted: r.get(8)?,
                        difference: r.get(9)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    if sessions.len() > 10000 {
        return Err(Error::InvalidInput(
            "Report is too large; select a shorter date range",
        ));
    }
    let movements = {
        // Filter sessions before combining movements so unrelated ledger detail
        // does not have to be materialized by the UNION query.
        let mut statement = transaction.prepare(
            "WITH selected_sessions AS (
                SELECT id,business_date FROM register_sessions
                WHERE business_date BETWEEN ?1 AND ?2 AND (?3 IS NULL OR register_id=?3)
            ), movements AS (
                SELECT m.session_id,'In' direction,m.amount,m.reason,m.created_at,m.id,s.business_date
                FROM selected_sessions s JOIN cash_in m ON m.session_id=s.id
                UNION ALL
                SELECT m.session_id,'Out',m.amount,m.reason,m.created_at,m.id,s.business_date
                FROM selected_sessions s JOIN cash_out m ON m.session_id=s.id
            ) SELECT session_id,direction,amount,reason,created_at FROM movements
            ORDER BY business_date,session_id,created_at,direction,id LIMIT 100001",
        )?;
        statement
            .query_map(
                params![filter.start_date, filter.end_date, filter.register_id],
                |r| {
                    Ok(MovementReport {
                        session_id: r.get(0)?,
                        direction: r.get(1)?,
                        amount: r.get(2)?,
                        reason: r.get(3)?,
                        created_at: r.get(4)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    if movements.len() > 100000 {
        return Err(Error::InvalidInput(
            "Report is too large; select a shorter date range",
        ));
    }
    transaction.commit()?;
    Ok(Report {
        filter: filter.clone(),
        sessions,
        movements,
    })
}

fn csv_text(value: &str) -> String {
    // Spreadsheet applications may interpret formulas even inside quoted cells.
    let dangerous = value.trim_start().starts_with(['=', '+', '-', '@'])
        || value.starts_with(['\t', '\r', '\n']);
    format!(
        "\"{}{}\"",
        if dangerous { "'" } else { "" },
        value.replace('"', "\"\"")
    )
}
impl Report {
    /// Values are exact minor units; currency remains per session, never combined.
    pub fn to_csv(&self) -> String {
        let mut result = String::new();
        self.append_csv(|row| {
            result.push_str(row);
            Ok(())
        })
        .unwrap_or_else(|never: std::convert::Infallible| match never {});
        result
    }

    /// Streams complete CSV rows and propagates destination write failures.
    pub fn write_csv(&self, writer: &mut impl std::io::Write) -> std::io::Result<()> {
        self.append_csv(|row| writer.write_all(row.as_bytes()))
    }

    fn append_csv<E>(
        &self,
        mut append: impl FnMut(&str) -> std::result::Result<(), E>,
    ) -> std::result::Result<(), E> {
        append(
            "session_id,register,currency,business_date,opening_minor,cash_in_minor,cash_out_minor,expected_minor,counted_minor,difference_minor\r\n",
        )?;
        for s in &self.sessions {
            append(&format!(
                "{},{},{},{},{},{},{},{},{},{}\r\n",
                s.id,
                csv_text(&s.register_name),
                csv_text(&s.currency),
                csv_text(&s.business_date),
                s.opening,
                s.cash_in,
                s.cash_out,
                s.expected,
                s.counted.map(|n| n.to_string()).unwrap_or_default(),
                s.difference.map(|n| n.to_string()).unwrap_or_default()
            ))?;
        }
        append("\r\nsession_id,direction,amount_minor,reason,created_at_utc\r\n")?;
        for m in &self.movements {
            append(&format!(
                "{},{},{},{},{}\r\n",
                m.session_id,
                csv_text(&m.direction),
                m.amount,
                csv_text(&m.reason),
                csv_text(&m.created_at)
            ))?;
        }
        Ok(())
    }
    pub fn printable(&self, company_name: &str) -> String {
        let mut text = format!(
            "{}\nCash ledger report\nBusiness dates: {} to {} (inclusive)\n\n",
            clean_line(company_name),
            self.filter.start_date,
            self.filter.end_date
        );
        let mut by_session = std::collections::BTreeMap::<i64, Vec<&MovementReport>>::new();
        for movement in &self.movements {
            by_session
                .entry(movement.session_id)
                .or_default()
                .push(movement);
        }
        if self.sessions.is_empty() {
            text.push_str("No sessions in this period.\n");
        }
        for s in &self.sessions {
            text.push_str(&format!("{} | {} | {} | Session {}\nOpening {}  In {}  Out {}  Expected {}\nCounted {}  Difference {}\n",s.business_date,clean_line(&s.register_name),s.currency,s.id,money(s.opening),money(s.cash_in),money(s.cash_out),money(s.expected),s.counted.map(money).unwrap_or_else(||"Open".into()),s.difference.map(money).unwrap_or_else(||"Pending".into())));
            if let Some(movements) = by_session.get(&s.id) {
                for m in movements {
                    text.push_str(&format!(
                        "  {} {} | {} | {} UTC\n",
                        m.direction,
                        money(m.amount),
                        clean_line(&m.reason),
                        m.created_at
                    ));
                }
            }
            text.push('\n');
        }
        text
    }
}
fn clean_line(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}
fn money(amount: i64) -> String {
    let magnitude = amount.unsigned_abs();
    format!(
        "{}{}.{:02}",
        if amount < 0 { "-" } else { "" },
        magnitude / 100,
        magnitude % 100
    )
}

/// Runs synchronously; callers must dispatch this operation to their I/O worker.
pub fn print_report(text: &str, printer: Option<&str>) -> Result<()> {
    print_report_with_paper(text, printer, "A4")
}
pub fn print_report_with_paper(text: &str, printer: Option<&str>, paper_size: &str) -> Result<()> {
    if !matches!(paper_size, "A4" | "Letter") {
        return Err(Error::InvalidInput("Unsupported paper size"));
    }
    if text.len() > 16_000_000 || printer.is_some_and(|p| p.contains('\0') || p.len() > 1024) {
        return Err(Error::InvalidInput(
            "Invalid print document or printer name",
        ));
    }
    #[cfg(windows)]
    {
        native_print::print(text, printer, paper_size)
    }
    #[cfg(not(windows))]
    {
        let _ = (text, printer);
        Err(Error::Printing(
            "Native printing is supported on Windows only; export CSV on this platform",
        ))
    }
}

#[cfg(windows)]
mod native_print {
    use super::{Error, Result};
    use windows_sys::Win32::Graphics::{Gdi::*, Printing::GetDefaultPrinterW};
    use windows_sys::Win32::Storage::Xps::{
        AbortDoc, DOCINFOW, EndDoc, EndPage, StartDocW, StartPage,
    };
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    struct PrinterDc {
        dc: HDC,
        active: bool,
    }
    struct PrinterFont {
        dc: HDC,
        font: HFONT,
        previous: HGDIOBJ,
    }
    impl Drop for PrinterFont {
        fn drop(&mut self) {
            // Restore the previous object before releasing the font owned by this job.
            unsafe {
                SelectObject(self.dc, self.previous);
                DeleteObject(self.font);
            }
        }
    }
    impl Drop for PrinterDc {
        fn drop(&mut self) {
            // This guard owns the device context; failed jobs are cancelled before release.
            unsafe {
                if self.active {
                    AbortDoc(self.dc);
                }
                DeleteDC(self.dc);
            }
        }
    }
    pub fn print(text: &str, printer: Option<&str>, paper: &str) -> Result<()> {
        let name = if let Some(name) = printer.filter(|s| !s.trim().is_empty()) {
            wide(name)
        } else {
            let mut size = 0;
            // The first call asks Windows for the required UTF-16 buffer length.
            unsafe {
                GetDefaultPrinterW(std::ptr::null_mut(), &mut size);
            }
            if size == 0 || size > 32768 {
                return Err(Error::Printing("No default printer is configured"));
            }
            let mut name = vec![0; size as usize];
            if unsafe { GetDefaultPrinterW(name.as_mut_ptr(), &mut size) } == 0 {
                return Err(Error::Printing("Unable to read the default printer"));
            }
            name
        };
        let driver = wide("WINSPOOL");
        // DEVMODE is zero-initialized and only dmPaperSize is marked as supplied.
        let mut mode: DEVMODEW = unsafe { std::mem::zeroed() };
        mode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        mode.dmFields = DM_PAPERSIZE;
        mode.Anonymous1.Anonymous1.dmPaperSize = if paper == "Letter" {
            DMPAPER_LETTER as i16
        } else {
            DMPAPER_A4 as i16
        };
        let dc = unsafe { CreateDCW(driver.as_ptr(), name.as_ptr(), std::ptr::null(), &mode) };
        if dc.is_null() {
            return Err(Error::Printing("Unable to connect to the selected printer"));
        }
        let mut guard = PrinterDc { dc, active: false };
        let title = wide("Cash ledger report");
        let info = DOCINFOW {
            cbSize: std::mem::size_of::<DOCINFOW>() as i32,
            lpszDocName: title.as_ptr(),
            lpszOutput: std::ptr::null(),
            lpszDatatype: std::ptr::null(),
            fwType: 0,
        };
        if unsafe { StartDocW(dc, &info) } <= 0 {
            return Err(Error::Printing("Unable to start the print job"));
        }
        guard.active = true;
        // Printer dimensions are device pixels. Margins and line spacing scale to DPI.
        let dpi = unsafe { GetDeviceCaps(dc, LOGPIXELSY as i32) }.max(96);
        let margin = (dpi / 2).max(1);
        let width = unsafe { GetDeviceCaps(dc, HORZRES as i32) };
        let height = unsafe { GetDeviceCaps(dc, VERTRES as i32) };
        if width <= margin * 2 || height <= margin * 2 {
            return Err(Error::Printing("The printer reported an invalid page size"));
        }
        let face = wide("Consolas");
        let font = unsafe {
            CreateFontW(
                -(dpi * 10 / 72),
                0,
                0,
                0,
                400,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                DEFAULT_QUALITY as u32,
                FIXED_PITCH as u32,
                face.as_ptr(),
            )
        };
        if font.is_null() {
            return Err(Error::Printing("Unable to create the printer font"));
        }
        let previous = unsafe { SelectObject(dc, font) };
        if previous.is_null() {
            unsafe {
                DeleteObject(font);
            }
            return Err(Error::Printing("Unable to select the printer font"));
        }
        let _font_guard = PrinterFont { dc, font, previous };
        let mut metrics: TEXTMETRICW = unsafe { std::mem::zeroed() };
        if unsafe { GetTextMetricsW(dc, &mut metrics) } == 0 {
            return Err(Error::Printing("Unable to measure the printer font"));
        }
        let line_height = (dpi / 6)
            .max(metrics.tmHeight + metrics.tmExternalLeading)
            .max(1);
        if unsafe { StartPage(dc) } <= 0 {
            return Err(Error::Printing("Unable to start a printed page"));
        }
        let mut y = margin;
        for line in text.lines() {
            let cleaned = super::clean_line(line);
            let mut chunks = Vec::<Vec<u16>>::new();
            let mut current = Vec::<u16>::new();
            for scalar in cleaned.chars() {
                let previous_length = current.len();
                current.extend(scalar.encode_utf16(&mut [0; 2]).iter().copied());
                let mut extent = windows_sys::Win32::Foundation::SIZE { cx: 0, cy: 0 };
                if unsafe {
                    GetTextExtentPoint32W(dc, current.as_ptr(), current.len() as i32, &mut extent)
                } == 0
                {
                    return Err(Error::Printing("Unable to measure report text"));
                }
                if extent.cx > width - margin * 2 && previous_length > 0 {
                    let next = current.split_off(previous_length);
                    chunks.push(current);
                    current = next;
                }
            }
            chunks.push(current);
            for encoded in chunks {
                if y + line_height > height - margin {
                    if unsafe { EndPage(dc) } <= 0 || unsafe { StartPage(dc) } <= 0 {
                        return Err(Error::Printing("Unable to advance the printed page"));
                    }
                    y = margin;
                }
                if !encoded.is_empty()
                    && unsafe { TextOutW(dc, margin, y, encoded.as_ptr(), encoded.len() as i32) }
                        == 0
                {
                    return Err(Error::Printing("Unable to render the printed report"));
                }
                y += line_height;
            }
        }
        if unsafe { EndPage(dc) } <= 0 || unsafe { EndDoc(dc) } <= 0 {
            return Err(Error::Printing("Unable to submit the print job"));
        }
        guard.active = false;
        Ok(())
    }
}
