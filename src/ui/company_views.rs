use super::views::input_field;
use super::*;
use cash_counter::company::{CompanyProfile, CompanyService, SetupCompany};
use gpui_component::{
    Disableable, Selectable,
    button::{Button, ButtonVariants},
    h_flex,
    scroll::Scrollbar,
    v_flex,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Access {
    Loading,
    Company,
    Administrator,
    Login,
    Authenticated,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Page {
    Register,
    Settings,
    Reports,
}

impl RegisterApp {
    pub(super) fn load_company(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.busy = true;
        let path = self.path.clone();
        let work = cx.background_executor().spawn(async move {
            let path = path?;
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent)
                    .map_err(|_| "Could not create application storage.".to_string())?;
            }
            CompanyService::open(path)
                .and_then(|service| service.profile())
                .map_err(|error| error.to_string())
        });
        self.task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = work.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.busy = false;
                match result {
                    Ok(profile) => {
                        view.access = if profile.is_some() {
                            Access::Login
                        } else {
                            Access::Company
                        };
                        view.profile = profile;
                        view.populate_settings(window, cx);
                        let input = if view.access == Access::Company {
                            &view.company
                        } else {
                            &view.username
                        };
                        input.update(cx, |input, cx| input.focus(window, cx));
                    }
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn populate_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(profile) = &self.profile {
            for (input, value) in [
                (&self.company, profile.name.clone()),
                (&self.address, profile.address.clone()),
                (&self.rules, profile.rules.clone()),
                (
                    &self.tolerance,
                    cash_counter::register::money::format_amount(
                        profile.closing_variance_threshold_cents,
                    ),
                ),
                (&self.printer, profile.printer_name.clone()),
                (&self.paper, profile.paper_size.clone()),
            ] {
                input.update(cx, |input, cx| input.set_value(value, window, cx));
            }
        }
    }

    pub(super) fn edited_profile(&self, cx: &App) -> Result<CompanyProfile, String> {
        let name = self.company.read(cx).value().trim().to_string();
        if name.is_empty() || name.len() > 160 {
            return Err("Enter a company name with 1–160 bytes.".into());
        }
        if self.address.read(cx).value().len() > 1000
            || self.rules.read(cx).value().len() > 4000
            || self.printer.read(cx).value().len() > 256
        {
            return Err("Address, rules, or printer name is too long.".into());
        }
        if !matches!(self.paper.read(cx).value().trim(), "A4" | "Letter") {
            return Err("Use A4 or Letter for paper size.".into());
        }
        let threshold =
            parse_amount(self.tolerance.read(cx).value().as_ref()).map_err(str::to_string)?;
        if threshold > 100_000_000 {
            return Err("Variance warning must be between 0 and 1,000,000 CAD.".into());
        }
        Ok(CompanyProfile {
            name,
            address: self.address.read(cx).value().trim().to_string(),
            rules: self.rules.read(cx).value().trim().to_string(),
            closing_variance_threshold_cents: threshold,
            printer_name: self.printer.read(cx).value().trim().to_string(),
            paper_size: self.paper.read(cx).value().trim().to_string(),
        })
    }

    pub(super) fn submit_access(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if self.access == Access::Loading {
            self.load_company(window, cx);
            return;
        }
        if self.access == Access::Company {
            match self.edited_profile(cx) {
                Ok(_) => {
                    self.access = Access::Administrator;
                    self.error = None;
                    self.username
                        .update(cx, |input, cx| input.focus(window, cx));
                }
                Err(error) => self.error = Some(error),
            }
            cx.notify();
            return;
        }
        let setup = self.access == Access::Administrator;
        let username = self.username.read(cx).value().to_string();
        let password = self.password.read(cx).value().to_string();
        if setup && password != self.confirmation.read(cx).value().as_ref() {
            self.error = Some("The passwords do not match. Enter them again.".into());
            cx.notify();
            return;
        }
        let profile = if setup {
            match self.edited_profile(cx) {
                Ok(profile) => Some(profile),
                Err(error) => {
                    self.error = Some(error);
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        let path = self.path.clone();
        self.busy = true;
        self.error = None;
        let work = cx.background_executor().spawn(async move {
            let mut service = CompanyService::open(path?).map_err(|error| error.to_string())?;
            if let Some(profile) = profile {
                service
                    .setup(SetupCompany {
                        profile,
                        username,
                        password,
                    })
                    .map_err(|error| error.to_string())?;
                service.profile().map_err(|error| error.to_string())
            } else {
                let authenticated = service
                    .authenticate(&username, &password)
                    .map_err(|error| error.to_string())?;
                if authenticated {
                    service.profile().map_err(|error| error.to_string())
                } else {
                    Ok(None)
                }
            }
        });
        self.task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = work.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.busy = false;
                view.password.update(cx, |input, cx| input.set_value("", window, cx));
                view.confirmation.update(cx, |input, cx| input.set_value("", window, cx));
                match result {
                    Ok(Some(profile)) if setup => {
                        view.notice = Some("Company created. Sign in with your administrator account.".into());
                        view.access = Access::Login;
                        view.profile = Some(profile);
                        view.password.update(cx, |input, cx| input.focus(window, cx));
                    }
                    Ok(Some(profile)) => {
                        view.profile = Some(profile);
                        view.populate_settings(window, cx);
                        view.access = Access::Authenticated;
                        view.run(Operation::Refresh, window, cx);
                    }
                    Ok(None) => view.error = Some("Sign in failed. Check your username and password. After repeated attempts, wait 30 seconds before retrying.".into()),
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn save_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.access != Access::Authenticated {
            return;
        }
        let profile = match self.edited_profile(cx) {
            Ok(profile) => profile,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let saved = profile.clone();
        let path = self.path.clone();
        self.busy = true;
        self.error = None;
        self.notice = None;
        let work = cx.background_executor().spawn(async move {
            CompanyService::open(path?)
                .and_then(|service| service.save_settings(&profile))
                .map_err(|error| error.to_string())
        });
        self.task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = work.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.busy = false;
                match result {
                    Ok(_) => {
                        view.profile = Some(saved);
                        view.notice = Some("Settings saved.".into());
                    }
                    Err(error) => view.error = Some(error),
                }
                view.resume_ready_update(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn logout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.failed_operation.is_some() {
            return;
        }
        self.access = Access::Login;
        self.page = Page::Register;
        self.snapshot = None;
        self.report_data = None;
        self.selection = Selection::default();
        self.closing_review = None;
        self.entry_pages.clear();
        self.session_pages.clear();
        self.error = None;
        self.notice = None;
        for input in [
            &self.amount,
            &self.reason,
            &self.count,
            &self.password,
            &self.confirmation,
        ] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.username
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub(super) fn access_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let company = self.access == Access::Company;
        let setup = self.access == Access::Administrator;
        let title = if company {
            "Set up your company"
        } else if setup {
            "Create an administrator"
        } else {
            "Sign in"
        };
        let mut form = v_flex().w(px(470.)).max_w_full().flex_shrink_0().gap_5().p_8()
            .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("Cash Ledger"))
            .child(div().font_family("Georgia").text_size(px(30.)).child(title))
            .child(div().text_color(rgb(0x61716a)).child(if company { "Company details and cash handling guidance for your local workspace." } else if setup { "This account protects access on this computer. Use a password of at least 12 characters." } else { "Enter your local account to open the cash register." }));
        if company {
            form = form.child(self.company_fields());
        } else if self.access != Access::Loading {
            form = form
                .child(input_field("Username", &self.username, self.busy))
                .child(input_field("Password", &self.password, self.busy));
            if setup {
                form = form.child(input_field(
                    "Confirm password",
                    &self.confirmation,
                    self.busy,
                ));
            }
        }
        form = form
            .child(self.update_banner())
            .child(self.access_feedback())
            .child(
                Button::new("access-submit")
                    .primary()
                    .label(if company {
                        "Continue"
                    } else if setup {
                        "Create company & account"
                    } else if self.access == Access::Loading {
                        "Retry loading"
                    } else {
                        "Sign in"
                    })
                    .loading(self.busy)
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| this.submit_access(window, cx))),
            );
        if setup {
            form = form.child(
                Button::new("back-company")
                    .label("Back to company details")
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.access = Access::Company;
                        cx.notify();
                    })),
            );
        }
        // Keep the form at its natural height inside a viewport-sized scroll region.
        div()
            .relative()
            .size_full()
            .child(
                div()
                    .id("access-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.access_scroll)
                    .child(h_flex().w_full().items_start().justify_center().child(form)),
            )
            .child(Scrollbar::vertical(&self.access_scroll))
            .into_any_element()
    }

    pub(super) fn access_feedback(&self) -> impl IntoElement {
        v_flex()
            .gap_2()
            .children(self.error.clone().map(|text| {
                div()
                    .id("access-error")
                    .role(Role::Alert)
                    .text_color(rgb(0x9b3030))
                    .child(text)
            }))
            .children(self.notice.clone().map(|text| div().child(text)))
    }

    pub(super) fn company_fields(&self) -> impl IntoElement {
        v_flex().gap_5()
            .child(v_flex().gap_4().child(div().font_weight(FontWeight::SEMIBOLD).child("Company")).child(input_field("Company name", &self.company, self.busy)).child(input_field("Company address", &self.address, self.busy)))
            .child(v_flex().gap_4().child(div().font_weight(FontWeight::SEMIBOLD).child("Cash handling")).child(input_field("Cash handling rules", &self.rules, self.busy)).child(input_field("Closing variance warning · CAD", &self.tolerance, self.busy)).child(div().text_sm().child("Warn when the final count differs by more than this amount. Rules are guidance for staff.")))
            .child(v_flex().gap_4().child(div().font_weight(FontWeight::SEMIBOLD).child("Printing")).child(input_field("Printer name · optional", &self.printer, self.busy)).child(input_field("Paper size · A4 or Letter", &self.paper, self.busy)))
    }

    pub(super) fn settings_view(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("settings-scroll")
            .flex_1()
            .overflow_y_scroll()
            .p_8()
            .child(
                v_flex()
                    .max_w(px(660.))
                    .gap_5()
                    .child(
                        div()
                            .font_family("Georgia")
                            .text_size(px(28.))
                            .child("Settings"),
                    )
                    .child(div().child(
                        "Configure your company, cash handling guidance and report printing.",
                    ))
                    .child(self.company_fields())
                    .child(self.access_feedback())
                    .child(
                        Button::new("save-settings")
                            .primary()
                            .label("Save settings")
                            .loading(self.busy)
                            .disabled(self.busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.save_settings(window, cx)),
                            ),
                    )
                    .child(self.updates_view(cx)),
            )
            .into_any_element()
    }

    pub(super) fn navigation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_3()
            .flex_wrap()
            .px_8()
            .py_3()
            .child(
                div().flex_1().child(
                    self.profile
                        .as_ref()
                        .map(|p| p.name.clone())
                        .unwrap_or_default(),
                ),
            )
            .child(
                Button::new("nav-register")
                    .label("Cash register")
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.page = Page::Register;
                        this.error = None;
                        this.notice = None;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("nav-reports")
                    .label("Reports")
                    .disabled(self.busy || self.failed_operation.is_some())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.page = Page::Reports;
                        this.error = None;
                        this.notice = None;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("nav-settings")
                    .label("Settings")
                    .disabled(self.busy || self.failed_operation.is_some())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.page = Page::Settings;
                        this.error = None;
                        this.notice = None;
                        this.populate_settings(window, cx);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("logout")
                    .label("Sign out")
                    .disabled(self.busy || self.failed_operation.is_some())
                    .on_click(cx.listener(|this, _, window, cx| this.logout(window, cx))),
            )
    }

    pub(super) fn generate_report(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.access != Access::Authenticated {
            return;
        }
        let filter = cash_counter::reporting::ReportFilter {
            start_date: self.report_start.read(cx).value().to_string(),
            end_date: self.report_end.read(cx).value().to_string(),
            register_id: self.report_register,
        };
        let path = self.path.clone();
        self.busy = true;
        self.error = None;
        self.notice = None;
        self.report_data = None;
        let work = cx.background_executor().spawn(async move {
            let database = cash_counter::database::Database::open(path?)
                .map_err(|_| "Could not open the report database.".to_string())?;
            cash_counter::reporting::report(&database, &filter).map_err(|error| error.to_string())
        });
        self.task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = work.await;
            let _ = view.update_in(cx, |view, _, cx| {
                view.busy = false;
                match result {
                    Ok(report) => view.report_data = Some(report),
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn print_report(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.access != Access::Authenticated {
            return;
        }
        let (Some(report), Some(profile)) = (&self.report_data, &self.profile) else {
            return;
        };
        let text = report.printable(&profile.name);
        let printer = profile.printer_name.clone();
        let paper = profile.paper_size.clone();
        self.busy = true;
        self.error = None;
        self.notice = None;
        let work = cx.background_executor().spawn(async move {
            cash_counter::reporting::print_report_with_paper(
                &text,
                if printer.is_empty() {
                    None
                } else {
                    Some(&printer)
                },
                &paper,
            )
            .map_err(|error| error.to_string())
        });
        self.task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = work.await;
            let _ =
                view.update_in(cx, |view, _, cx| {
                    view.busy = false;
                    match result {
                        Ok(_) => view.notice = Some(
                            "Report sent to the printer. Check the printer queue for completion."
                                .into(),
                        ),
                        Err(error) => view.error = Some(error),
                    }
                    cx.notify();
                });
        }));
        cx.notify();
    }

    pub(super) fn export_report(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.access != Access::Authenticated {
            return;
        }
        let Some(report) = &self.report_data else {
            return;
        };
        let report = report.clone();
        let database_path = self.path.clone();
        let directory = self
            .path
            .as_ref()
            .ok()
            .and_then(|path| path.parent())
            .unwrap_or(std::path::Path::new("."));
        let picker = cx.prompt_for_new_path(directory, Some("cash-report.csv"));
        self.busy = true;
        self.error = None;
        self.notice = None;
        self.task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = match picker.await {
                Ok(Ok(Some(path))) => {
                    cx.background_executor()
                        .spawn(async move {
                            cash_counter::reporting::export_csv(&report, &path, &database_path?)
                                .map(|_| true)
                                .map_err(|error| error.to_string())
                        })
                        .await
                }
                Ok(Ok(None)) => Ok(false),
                _ => Err("Could not open the file picker. Try again.".into()),
            };
            let _ = view.update_in(cx, |view, _, cx| {
                view.busy = false;
                match result {
                    Ok(true) => view.notice = Some("CSV report saved.".into()),
                    Ok(false) => {}
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn reports_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut register_filter = h_flex().gap_3().flex_wrap().child(
            Button::new("report-all-registers")
                .label("All registers")
                .selected(self.report_register.is_none())
                .disabled(self.busy)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.report_register = None;
                    this.report_data = None;
                    cx.notify();
                })),
        );
        if let Some(snapshot) = &self.snapshot {
            for register in &snapshot.registers {
                let id = register.id;
                register_filter = register_filter.child(
                    Button::new(("report-register", id as u64))
                        .label(register.name.clone())
                        .selected(self.report_register == Some(id))
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.report_register = Some(id);
                            this.report_data = None;
                            cx.notify();
                        })),
                );
            }
        }
        let mut body = v_flex()
            .gap_5()
            .child(
                div()
                    .font_family("Georgia")
                    .text_size(px(28.))
                    .child("Reports"),
            )
            .child(
                div().child(
                    "Review complete saved days and movements within a business date range.",
                ),
            )
            .child(
                h_flex()
                    .gap_4()
                    .flex_wrap()
                    .child(input_field(
                        "From · YYYY-MM-DD",
                        &self.report_start,
                        self.busy,
                    ))
                    .child(input_field(
                        "Through · YYYY-MM-DD",
                        &self.report_end,
                        self.busy,
                    )),
            )
            .child(register_filter)
            .child(div().text_sm().child(if self.report_register.is_none() {
                "Filter: all registers"
            } else {
                "Filter: selected register"
            }))
            .child(
                h_flex()
                    .gap_3()
                    .flex_wrap()
                    .child(
                        Button::new("generate-report")
                            .primary()
                            .label("Generate report")
                            .disabled(self.busy || self.selection.register.is_none())
                            .loading(self.busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.generate_report(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("export-report")
                            .label("Export CSV")
                            .disabled(self.busy || self.report_data.is_none())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.export_report(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("print-report")
                            .label("Print report")
                            .disabled(self.busy || self.report_data.is_none())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.print_report(window, cx)),
                            ),
                    ),
            )
            .child(self.access_feedback());
        if let Some(report) = &self.report_data {
            body = body.child(div().text_sm().child(format!(
                "{} saved days · {} movements · {} to {}",
                report.sessions.len(),
                report.movements.len(),
                report.filter.start_date,
                report.filter.end_date
            )));
            if report.sessions.is_empty() {
                body = body.child(div().child("No saved days match this date range."));
            }
            if report.sessions.len() > 100 {
                body = body.child(div().text_sm().child("Preview shows the first 100 days. CSV and printing include the complete report."));
            }
            for session in report.sessions.iter().take(100) {
                body = body.child(
                    v_flex()
                        .gap_2()
                        .py_4()
                        .border_b_1()
                        .border_color(rgb(0xdde3dc))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(format!(
                            "{} · {}",
                            session.business_date, session.register_name
                        )))
                        .child(div().font_family("Consolas").child(format!(
                            "Opening {} · In {} · Out {} · Expected {} ({})",
                            cash_counter::register::money::format_amount(session.opening),
                            cash_counter::register::money::format_amount(session.cash_in),
                            cash_counter::register::money::format_amount(session.cash_out),
                            cash_counter::register::money::format_amount(session.expected),
                            session.currency
                        )))
                        .child(
                            div().child(
                                session
                                    .counted
                                    .map(|counted| {
                                        format!(
                                            "Counted {} · Difference {}",
                                            cash_counter::register::money::format_amount(counted),
                                            session
                                                .difference
                                                .map(cash_counter::register::money::format_amount)
                                                .unwrap_or_default()
                                        )
                                    })
                                    .unwrap_or_else(|| "Open day · final count pending".into()),
                            ),
                        ),
                );
            }
        } else {
            body = body.child(div().text_color(rgb(0x61716a)).child(
                "Generate a report to preview totals. Exports include every matching movement.",
            ));
        }
        div()
            .id("reports-scroll")
            .flex_1()
            .overflow_y_scroll()
            .p_8()
            .child(body)
            .into_any_element()
    }
}
