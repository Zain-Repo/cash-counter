//! Bounded analytics reads use a separate connection and never hold the ledger mutex.

use super::*;
use cash_counter::analytics::{self, DashboardFilter, DashboardSnapshot};
use cash_counter::register::money::format_cad;
use chrono::Days;
use gpui::prelude::FluentBuilder;
use gpui_component::{
    Disableable, Selectable,
    alert::Alert,
    button::Button,
    chart::{BarChart, LineChart},
    h_flex,
    tag::Tag,
    v_flex,
};
use std::time::Duration;

const MUTED: u32 = 0x61716a;
const LINE: u32 = 0xdde3dc;
const GREEN: u32 = 0x265746;

pub(super) struct Dashboard {
    days: u64,
    pub(super) working: bool,
    pub(super) generation: u64,
    pub(super) data: Option<DashboardSnapshot>,
    pub(super) error: Option<String>,
}

impl Dashboard {
    pub(super) fn invalidate(&mut self) {
        self.generation += 1;
    }
}

impl Default for Dashboard {
    fn default() -> Self {
        Self {
            days: 30,
            working: false,
            generation: 0,
            data: None,
            error: None,
        }
    }
}

impl RegisterApp {
    pub(super) fn clear_dashboard(&mut self) {
        self.dashboard_timer = None;
        self.dashboard.invalidate();
        self.dashboard.data = None;
        self.dashboard.error = None;
    }

    fn save_backup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.access != Access::Authenticated || self.backup_working || self.busy {
            return;
        }
        let source = match &self.path {
            Ok(path) => path.clone(),
            Err(error) => {
                self.dashboard.error = Some(error.clone());
                cx.notify();
                return;
            }
        };
        let filename = format!(
            "ledger-backup-{}.sqlite",
            Local::now().format("%Y%m%d-%H%M%S")
        );
        let picker = cx.prompt_for_new_path(
            source.parent().unwrap_or(std::path::Path::new(".")),
            Some(&filename),
        );
        self.backup_working = true;
        self.notice = None;
        self.backup_task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = match picker.await {
                Ok(Ok(Some(destination))) => cx.background_executor().spawn(async move { cash_counter::backup::backup_database(&source, &destination).map(Some) }).await,
                Ok(Ok(None)) => Ok(None),
                _ => Err("Could not open the backup file picker. Try again.".into()),
            };
            let _ = view.update_in(cx, |view, _, cx| {
                view.backup_working = false;
                match result {
                    Ok(Some(summary)) => view.notice = Some(format!("Verified backup saved to {} ({} bytes). Keep this file private; it contains your ledger and account data.", summary.path.display(), summary.size_bytes)),
                    Ok(None) => {},
                    Err(error) => view.dashboard.error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub(super) fn refresh_dashboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dashboard.working
            || self.access != Access::Authenticated
            || self.page != Page::Dashboard
        {
            return;
        }
        self.dashboard_timer = None;
        let path = match &self.path {
            Ok(path) => path.clone(),
            Err(error) => {
                self.dashboard.error = Some(error.clone());
                cx.notify();
                return;
            }
        };
        let today = Local::now().date_naive();
        let Some(start) = today.checked_sub_days(Days::new(self.dashboard.days - 1)) else {
            return;
        };
        let filter = DashboardFilter {
            start_date: start.to_string(),
            end_date: today.to_string(),
            currency: "CAD".into(),
            register_id: None,
        };
        self.dashboard.generation += 1;
        let generation = self.dashboard.generation;
        self.dashboard.working = true;
        self.dashboard.error = None;
        let work = cx.background_executor().spawn(async move {
            analytics::load_dashboard(&path, &filter).map_err(|error| match error {
                analytics::Error::InvalidInput(message) => message.to_string(),
                _ => "Unable to refresh the dashboard. Try again or select a shorter range.".into(),
            })
        });
        self.dashboard_task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = work.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.dashboard.working = false;
                if view.access != Access::Authenticated || view.page != Page::Dashboard {
                    return;
                }
                if view.dashboard.generation != generation {
                    view.refresh_dashboard(window, cx);
                    return;
                }
                match result {
                    Ok(data) => view.dashboard.data = Some(data),
                    Err(error) => view.dashboard.error = Some(error),
                }
                // A single timer follows each completion; slow queries cannot accumulate work.
                view.dashboard_timer = Some(cx.spawn_in(window, async move |view, cx| {
                    cx.background_executor()
                        .timer(Duration::from_secs(15))
                        .await;
                    let _ =
                        view.update_in(cx, |view, window, cx| view.refresh_dashboard(window, cx));
                }));
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn dashboard_view(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let mut content =
            v_flex().gap_6().p_8().min_w_0().child(
                h_flex()
                    .flex_wrap()
                    .gap_4()
                    .justify_between()
                    .child(
                        v_flex()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(28.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Live cash dashboard"),
                            )
                            .child(div().text_sm().text_color(rgb(MUTED)).child(
                                "All registers · CAD · refreshes every 15 seconds while open",
                            )),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .children([7_u64, 30, 90, 365].map(|days| {
                                Button::new(("dashboard-range", days))
                                    .label(format!("{days} days"))
                                    .selected(self.dashboard.days == days)
                                    .disabled(self.dashboard.working)
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.dashboard.days = days;
                                        view.dashboard.data = None;
                                        view.refresh_dashboard(window, cx);
                                    }))
                            }))
                            .child(
                                Button::new("dashboard-refresh")
                                    .label("Refresh now")
                                    .loading(self.dashboard.working)
                                    .disabled(self.dashboard.working)
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.refresh_dashboard(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("dashboard-backup")
                                    .label("Save verified backup")
                                    .loading(self.backup_working)
                                    .disabled(self.backup_working || self.busy)
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.save_backup(window, cx)
                                    })),
                            ),
                    ),
            );
        if let Some(error) = &self.dashboard.error {
            content = content.child(Alert::error("dashboard-error", error.clone()));
        }
        if let Some(notice) = &self.notice {
            content = content.child(Alert::success("dashboard-notice", notice.clone()));
        }
        if let Some(data) = &self.dashboard.data {
            let summary = &data.summary;
            content = content.child(
                h_flex()
                    .gap_6()
                    .flex_wrap()
                    .py_4()
                    .border_y_1()
                    .border_color(rgb(LINE))
                    .child(value("Cash in", format_cad(summary.cash_in)))
                    .child(value("Cash out", format_cad(summary.cash_out)))
                    .child(value("Net movement", format_cad(summary.net_flow)))
                    .child(value("Movements", summary.movement_count.to_string()))
                    .child(value("Open days", summary.open_sessions.to_string())),
            );
            content = content.child(
                h_flex()
                    .gap_3()
                    .flex_wrap()
                    .child(Tag::secondary().child(format!(
                        "{} to {}",
                        data.filter.start_date, data.filter.end_date
                    )))
                    .child(div().text_xs().text_color(rgb(MUTED)).child(format!(
                        "Query {:.1} ms · snapshot {}",
                        data.query_duration.as_secs_f64() * 1000.,
                        data.generated_at
                    ))),
            );
            if summary.movement_count == 0 {
                content = content.child(Alert::info("dashboard-empty", "No cash movements in this period. Choose a longer range or record a movement in the cash register."));
            } else {
                let daily: Vec<_> = data
                    .daily_flow
                    .iter()
                    .map(|day| (day.date.clone(), day.net_flow as f64 / 100.))
                    .collect();
                let rankings: Vec<_> = data
                    .register_rankings
                    .iter()
                    .map(|row| (row.register_name.clone(), row.cash_in as f64 / 100.))
                    .collect();
                content = content.child(
                    v_flex()
                        .gap_3()
                        .child(heading("Daily net cash movement"))
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(MUTED))
                                .child("CAD · cash in minus cash out. Hover a date for its value."),
                        )
                        .child(
                            div().h(px(240.)).w_full().child(
                                LineChart::new(daily)
                                    .id("daily-cash-chart")
                                    .name("Net CAD")
                                    .x(|d| d.0.clone())
                                    .y(|d| d.1)
                                    .linear()
                                    .stroke(rgb(GREEN))
                                    .tick_margin((data.daily_flow.len() / 7).max(1)),
                            ),
                        ),
                );
                content = content.child(
                    v_flex()
                        .gap_3()
                        .child(heading("Registers by cash received"))
                        .child(
                            div().h(px(220.)).w_full().child(
                                BarChart::new(rankings)
                                    .id("register-cash-chart")
                                    .name("Cash in CAD")
                                    .band(|d| d.0.clone())
                                    .value(|d| d.1)
                                    .fill(|_, _, _, _| rgb(GREEN)),
                            ),
                        ),
                );
            }
            let wide = window.viewport_size().width >= px(1000.);
            content = content.child(v_flex().gap_3().child(heading("Closing count exceptions"))
                .child(div().text_sm().text_color(rgb(MUTED)).child(format!("{} days with a count difference · total difference {} CAD · 20 largest differences shown", summary.discrepancy_count, format_cad(summary.discrepancy_total))))
                .when(data.discrepancies.is_empty(), |element|element.child("No closing count differences in this period."))
                .children(data.discrepancies.iter().map(|row| h_flex().when(!wide, |element|element.flex_col()).gap_3().py_3().border_b_1().border_color(rgb(LINE))
                    .child(div().flex_1().child(format!("{} · {}",row.register_name,row.business_date)))
                    .child(div().child(format!("Expected {} · Counted {} · Difference {} CAD",format_cad(row.expected),format_cad(row.counted),format_cad(row.difference)))))));
        } else if self.dashboard.working {
            content = content.child(Alert::info(
                "dashboard-loading",
                "Calculating cash flow and closing exceptions in the background…",
            ));
        }
        div()
            .id("dashboard-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(content)
            .into_any_element()
    }
}

fn heading(label: &'static str) -> impl IntoElement {
    div()
        .text_lg()
        .font_weight(FontWeight::SEMIBOLD)
        .child(label)
}
fn value(label: &'static str, value: String) -> impl IntoElement {
    v_flex()
        .gap_2()
        .min_w(px(135.))
        .flex_1()
        .child(div().text_sm().text_color(rgb(MUTED)).child(label))
        .child(div().text_lg().font_family("Consolas").child(value))
}
