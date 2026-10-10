//! Native GPUI workspace. Storage operations are serialized on the background executor.

mod company_views;
mod components;
mod dashboard_views;
#[cfg(test)]
mod tests;
mod update_views;
mod views;
use company_views::{Access, Page};
use components::{RegisterChoice, ReportPeriod};

use cash_counter::database::Direction;
use cash_counter::register::{
    Operation, RegisterService, Reply, Selection, Snapshot, money::parse_amount,
};
use chrono::{Local, NaiveDate};
use gpui::*;
use gpui_component::input::{InputEvent, InputState};
use gpui_component::{
    IndexPath,
    searchable_list::SearchableVec,
    select::{SelectEvent, SelectState},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

actions!(cash_register, [SubmitForm, CancelForm]);

#[derive(Clone, Copy, PartialEq, Eq)]
enum FormMode {
    Entry,
    Open,
    Setup,
    Close,
}

pub struct RegisterApp {
    access: Access,
    access_scroll: ScrollHandle,
    page: Page,
    dashboard: dashboard_views::Dashboard,
    dashboard_task: Option<Task<()>>,
    dashboard_timer: Option<Task<()>>,
    backup_task: Option<Task<()>>,
    backup_working: bool,
    profile: Option<cash_counter::company::CompanyProfile>,
    company: Entity<InputState>,
    address: Entity<InputState>,
    rules: Entity<InputState>,
    tolerance: Entity<InputState>,
    printer: Entity<InputState>,
    paper: Entity<SelectState<Vec<&'static str>>>,
    settings_sections: Vec<usize>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    confirmation: Entity<InputState>,
    report_start: Entity<InputState>,
    report_end: Entity<InputState>,
    report_data: Option<cash_counter::reporting::Report>,
    report_register: Option<i64>,
    report_selector: Entity<SelectState<SearchableVec<RegisterChoice>>>,
    service: Arc<Mutex<Option<RegisterService>>>,
    path: Result<PathBuf, String>,
    snapshot: Option<Snapshot>,
    selection: Selection,
    mode: FormMode,
    busy: bool,
    error: Option<String>,
    notice: Option<String>,
    failed_operation: Option<Operation>,
    task: Option<Task<()>>,
    updates: update_views::Updates,
    update_task: Option<Task<()>>,
    update_idle_task: Option<Task<()>>,
    amount: Entity<InputState>,
    reason: Entity<InputState>,
    date: Entity<InputState>,
    name: Entity<InputState>,
    count: Entity<InputState>,
    closing_review: Option<i64>,
    entry_pages: Vec<Option<i64>>,
    session_pages: Vec<Option<i64>>,
}

impl RegisterApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut app = Self::with_path(database_path(), window, cx);
        app.initialize_updates(window, cx);
        app
    }

    fn with_path(
        path: Result<PathBuf, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let mut app = Self {
            access_scroll: ScrollHandle::default(),
            access: Access::Loading,
            page: Page::Register,
            dashboard: dashboard_views::Dashboard::default(),
            dashboard_task: None,
            dashboard_timer: None,
            backup_task: None,
            backup_working: false,
            profile: None,
            company: cx.new(|cx| InputState::new(window, cx).placeholder("Company name")),
            address: cx.new(|cx| InputState::new(window, cx).placeholder("Address (optional)")),
            rules: cx
                .new(|cx| InputState::new(window, cx).placeholder("Cash handling instructions")),
            tolerance: cx.new(|cx| InputState::new(window, cx).default_value("0.00")),
            printer: cx
                .new(|cx| InputState::new(window, cx).placeholder("Blank uses the system default")),
            paper: cx.new(|cx| {
                SelectState::new(vec!["A4", "Letter"], Some(IndexPath::default()), window, cx)
            }),
            settings_sections: vec![0],
            username: cx.new(|cx| InputState::new(window, cx).placeholder("Username")),
            password: cx.new(|cx| InputState::new(window, cx).masked(true)),
            confirmation: cx.new(|cx| InputState::new(window, cx).masked(true)),
            report_start: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(Local::now().format("%Y-%m-01").to_string())
            }),
            report_end: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(Local::now().format("%Y-%m-%d").to_string())
            }),
            report_data: None,
            report_register: None,
            report_selector: cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(vec![RegisterChoice {
                        id: None,
                        name: "All registers".into(),
                    }]),
                    Some(IndexPath::default()),
                    window,
                    cx,
                )
                .searchable(true)
            }),
            service: Arc::new(Mutex::new(None)),
            path,
            snapshot: None,
            selection: Selection::default(),
            mode: FormMode::Entry,
            busy: false,
            error: None,
            notice: None,
            failed_operation: None,
            task: None,
            updates: update_views::Updates::default(),
            update_task: None,
            update_idle_task: None,
            amount: cx.new(|cx| InputState::new(window, cx).placeholder("0.00")),
            reason: cx
                .new(|cx| InputState::new(window, cx).placeholder("e.g. Cash sale or petty cash")),
            date: cx.new(|cx| {
                let mut input = InputState::new(window, cx);
                input.set_value(today, window, cx);
                input
            }),
            name: cx.new(|cx| InputState::new(window, cx).placeholder("e.g. Front desk")),
            count: cx.new(|cx| InputState::new(window, cx).placeholder("0.00")),
            closing_review: None,
            entry_pages: Vec::new(),
            session_pages: Vec::new(),
        };
        // Single-line inputs consume Enter; use their event rather than a parent binding.
        for input in [&app.username, &app.password] {
            cx.subscribe_in(input, window, |app, _, event: &InputEvent, window, cx| {
                if app.access == Access::Login
                    && matches!(
                        event,
                        InputEvent::PressEnter {
                            secondary: false,
                            shift: false
                        }
                    )
                {
                    app.submit_access(window, cx);
                }
            })
            .detach();
        }
        app.load_company(window, cx);
        for input in [&app.report_start, &app.report_end] {
            cx.subscribe_in(input, window, |app, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) && !app.busy {
                    // A preview must always correspond to the visible report filters.
                    app.report_data = None;
                    cx.notify();
                }
            })
            .detach();
        }
        cx.subscribe_in(
            &app.paper,
            window,
            |_, _, _: &SelectEvent<Vec<&'static str>>, _, cx| cx.notify(),
        )
        .detach();
        cx.subscribe_in(
            &app.report_selector,
            window,
            |app, _, event: &SelectEvent<SearchableVec<RegisterChoice>>, _, cx| {
                if app.busy || app.access != Access::Authenticated {
                    return;
                }
                let SelectEvent::Confirm(Some(register)) = event else {
                    return;
                };
                app.report_register = *register;
                app.report_data = None;
                cx.notify();
            },
        )
        .detach();
        app
    }

    fn run(&mut self, operation: Operation, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.access != Access::Authenticated {
            return;
        }
        self.busy = true;
        self.error = None;
        self.notice = None;
        let service = self.service.clone();
        let path = self.path.clone();
        let selection = self.selection.clone();
        let background_operation = operation.clone();
        let work = cx.background_executor().spawn(async move {
            let mut service = match service.lock() {
                Ok(service) => service,
                Err(_) => return Reply { applied: false, retryable: false, snapshot: Err("Storage is unavailable. Restart the application.".into()) },
            };
            if service.is_none() {
                let initialization = path.and_then(|path| {
                    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|_| "Could not create the local storage directory.".to_string())?; }
                    RegisterService::open(path).map_err(|_| "Could not open the local database. Check that the storage location is available.".to_string())
                });
                match initialization {
                    Ok(initialized) => *service = Some(initialized),
                    Err(error) => return Reply { applied: false, retryable: false, snapshot: Err(error) },
                }
            }
            match service.as_mut() {
                Some(service) => service.execute(&background_operation, &selection),
                None => Reply { applied: false, retryable: false, snapshot: Err("Storage could not be initialized.".into()) },
            }
        });
        self.task = Some(cx.spawn_in(window, async move |view, cx| {
            let reply = work.await;
            // The window may close while a bounded database operation completes.
            let _ = view.update_in(cx, |view, window, cx| {
                view.receive(reply, operation, window, cx)
            });
        }));
        cx.notify();
    }

    fn receive(
        &mut self,
        reply: Reply,
        operation: Operation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.busy = false;
        if reply.applied {
            self.failed_operation = None;
            self.notice = match operation {
                Operation::Refresh => None,
                Operation::CreateRegister { .. } => {
                    Some("Register created. Open a day to begin.".into())
                }
                Operation::Open { .. } => Some("Day opened. Your starting cash is saved.".into()),
                Operation::Record { .. } => Some("Cash movement saved.".into()),
                Operation::Close { .. } => Some("Day closed. The final count is saved.".into()),
            };
            if let Operation::Record { direction, .. } = &operation {
                self.selection.direction = *direction;
                self.selection.before_entry = None;
                self.entry_pages.clear();
                self.amount
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.reason
                    .update(cx, |input, cx| input.set_value("", window, cx));
            }
            if !matches!(operation, Operation::Refresh | Operation::Record { .. }) {
                self.mode = FormMode::Entry;
                self.closing_review = None;
                self.selection.before_session = None;
                self.selection.before_entry = None;
                self.entry_pages.clear();
                self.session_pages.clear();
                self.amount
                    .update(cx, |input, cx| input.set_value("", window, cx));
            }
        } else if reply.retryable {
            // Preserve the exact payload and retry key after a failed write.
            self.failed_operation = Some(operation.clone());
        }
        match reply.snapshot {
            Ok(snapshot) => {
                self.selection.register = snapshot.register.as_ref().map(|register| register.id);
                self.selection.session = snapshot.session.as_ref().map(|session| session.id);
                if snapshot.register.is_none() {
                    self.mode = FormMode::Setup;
                } else if snapshot.session.is_none() {
                    self.mode = FormMode::Open;
                }
                self.refresh_report_registers(&snapshot, window, cx);
                self.snapshot = Some(snapshot);
            }
            Err(error) => {
                self.error = Some(
                    if reply.applied && !matches!(operation, Operation::Refresh) {
                        format!(
                            "Saved, but the view could not refresh. Use Refresh to load the latest records. {error}"
                        )
                    } else {
                        error
                    },
                );
            }
        }
        self.resume_ready_update(window, cx);
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.access != Access::Authenticated {
            self.submit_access(window, cx);
            return;
        }
        if self.page != Page::Register {
            return;
        }
        if self.busy {
            return;
        }
        if let Some(operation) = self.failed_operation.clone() {
            self.run(operation, window, cx);
            return;
        }
        let operation = match self.prepare_operation(cx) {
            Ok(operation) => operation,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        if let Operation::Close { counted, .. } = operation
            && self.closing_review.is_none()
        {
            self.closing_review = Some(counted);
            self.error = None;
            cx.notify();
            return;
        }
        self.run(operation, window, cx);
    }

    fn prepare_operation(&self, cx: &App) -> Result<Operation, String> {
        let amount = || parse_amount(self.amount.read(cx).value().as_ref()).map_err(str::to_string);
        match self.mode {
            FormMode::Setup => {
                let name = self.name.read(cx).value().trim().to_string();
                if name.is_empty() || name.chars().count() > 100 {
                    return Err("Enter a register name with 1–100 characters.".into());
                }
                Ok(Operation::CreateRegister { name })
            }
            FormMode::Open => {
                let date = self.date.read(cx).value().to_string();
                let parsed = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                    .map_err(|_| "Enter a valid date in YYYY-MM-DD format.".to_string())?;
                if parsed.format("%Y-%m-%d").to_string() != date {
                    return Err("Use YYYY-MM-DD for the business date.".into());
                }
                Ok(Operation::Open {
                    register: self.selection.register.ok_or("Select a register first.")?,
                    date,
                    opening: amount()?,
                })
            }
            FormMode::Entry => {
                if self
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.session.as_ref())
                    .is_none_or(|session| session.closed)
                {
                    return Err("Select an open day before recording cash.".into());
                }
                let amount = amount()?;
                if amount == 0 {
                    return Err("Cash movements must be greater than zero.".into());
                }
                let reason = self.reason.read(cx).value().trim().to_string();
                if reason.is_empty() || reason.chars().count() > 500 {
                    return Err("Enter a reason with 1–500 characters.".into());
                }
                Ok(Operation::Record {
                    session: self
                        .selection
                        .session
                        .ok_or("Open a day before recording cash.")?,
                    direction: self.selection.direction,
                    amount,
                    reason,
                    request_key: uuid::Uuid::new_v4().to_string(),
                })
            }
            FormMode::Close => {
                let counted = self.closing_review.map(Ok).unwrap_or_else(|| {
                    parse_amount(self.count.read(cx).value().as_ref()).map_err(str::to_string)
                })?;
                let expected = self
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.balance.as_ref())
                    .ok_or("Select an open day first.")?
                    .expected;
                if counted.checked_sub(expected).is_none() {
                    return Err(
                        "The count difference is outside the supported amount range.".into(),
                    );
                }
                Ok(Operation::Close {
                    session: self.selection.session.ok_or("Select an open day first.")?,
                    counted,
                    expected,
                })
            }
        }
    }

    fn change_mode(&mut self, mode: FormMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.failed_operation.is_some() {
            return;
        }
        self.mode = mode;
        self.closing_review = None;
        self.error = None;
        self.notice = None;
        if mode == FormMode::Open {
            self.amount
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.date.update(cx, |input, cx| {
                input.set_value(Local::now().format("%Y-%m-%d").to_string(), window, cx)
            });
        }
        if mode == FormMode::Close {
            self.count
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        let input = match mode {
            FormMode::Setup => &self.name,
            FormMode::Open => &self.date,
            FormMode::Close => &self.count,
            FormMode::Entry => &self.amount,
        };
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn change_direction(
        &mut self,
        direction: Direction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || self.failed_operation.is_some() {
            return;
        }
        self.selection.direction = direction;
        self.selection.before_entry = None;
        self.entry_pages.clear();
        self.run(Operation::Refresh, window, cx);
    }
}

fn database_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("CASH_REGISTER_DB") {
        if path.is_empty() {
            return Err("The configured database path is empty.".into());
        }
        return Ok(PathBuf::from(path));
    }
    directories::ProjectDirs::from("ca", "Local", "Cash Register")
        .map(|directories| directories.data_local_dir().join("register.sqlite3"))
        .ok_or_else(|| "Could not find the local application-data directory.".into())
}
