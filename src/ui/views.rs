use super::*;
use cash_counter::register::money::{format_amount, format_cad};
use gpui::prelude::FluentBuilder;
use gpui_component::{
    Disableable, Icon, IconName, Selectable,
    alert::Alert,
    button::{Button, ButtonVariants},
    form::{field, v_form},
    h_flex,
    input::Input,
    tag::Tag,
    v_flex,
};

const INK: u32 = 0x213b34;
const MUTED: u32 = 0x61716a;
const CANVAS: u32 = 0xf7f7f2;
const LINE: u32 = 0xdde3dc;
const GREEN: u32 = 0x265746;

impl Render for RegisterApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.access != Access::Authenticated {
            return div()
                .size_full()
                .bg(rgb(CANVAS))
                .font_family("Segoe UI")
                .text_color(rgb(INK))
                .key_context("CashRegister")
                .on_action(
                    cx.listener(|this, _: &SubmitForm, window, cx| this.submit_access(window, cx)),
                )
                .child(self.access_view(cx))
                .into_any_element();
        }
        if self.page != Page::Register {
            if self.page == Page::Dashboard {
                return v_flex()
                    .size_full()
                    .bg(rgb(CANVAS))
                    .font_family("Segoe UI")
                    .text_color(rgb(INK))
                    .child(self.navigation(cx))
                    .child(self.update_banner())
                    .child(self.dashboard_view(window, cx))
                    .into_any_element();
            }
            let page = if self.page == Page::Settings {
                self.settings_view(cx)
            } else {
                self.reports_view(cx)
            };
            return v_flex()
                .key_context("CashRegister")
                .on_action(cx.listener(|this, _: &SubmitForm, window, cx| {
                    if this.page == Page::Settings {
                        this.save_settings(window, cx);
                    } else {
                        this.generate_report(window, cx);
                    }
                }))
                .on_action(cx.listener(|this, _: &CancelForm, _, cx| {
                    if !this.busy {
                        this.page = Page::Register;
                        cx.notify();
                    }
                }))
                .size_full()
                .bg(rgb(CANVAS))
                .font_family("Segoe UI")
                .text_color(rgb(INK))
                .child(self.navigation(cx))
                .child(self.update_banner())
                .child(page)
                .into_any_element();
        }
        let wide = window.viewport_size().width >= px(1120.);
        let blocked = self.busy || self.failed_operation.is_some();
        let content = v_flex()
            .flex_1()
            .min_w_0()
            .size_full()
            .child(self.navigation(cx))
            .child(self.update_banner())
            .child(self.header(cx))
            .child(self.feedback(cx))
            .child(self.summary())
            .child(
                div()
                    .id("workspace-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .when(!wide, |element| element.flex_col())
                            .gap_8()
                            .p_8()
                            .min_w_0()
                            .when(wide, |element| element.child(self.ledger(cx)))
                            .child(self.form(blocked, cx))
                            .when(!wide, |element| element.child(self.ledger(cx))),
                    ),
            )
            .child(
                h_flex()
                    .px_8()
                    .py_3()
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .justify_between()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("Saved locally · CAD")
                    .child(if self.busy {
                        "Working…"
                    } else {
                        "Ctrl + Enter to submit"
                    }),
            );
        h_flex()
            .id("cash-register")
            .key_context("CashRegister")
            .on_action(cx.listener(|this, _: &SubmitForm, window, cx| this.submit(window, cx)))
            .on_action(cx.listener(|this, _: &CancelForm, window, cx| {
                this.change_mode(FormMode::Entry, window, cx)
            }))
            .size_full()
            .bg(rgb(CANVAS))
            .font_family("Segoe UI")
            .text_color(rgb(INK))
            .text_size(px(14.))
            .child(self.sidebar(cx))
            .child(content)
            .into_any_element()
    }
}

impl RegisterApp {
    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.session.as_ref());
        let title = session
            .map(|session| friendly_date(&session.business_date))
            .unwrap_or_else(|| "Cash register".into());
        let closed = session.is_some_and(|session| session.closed);
        let can_open = self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.register.is_some() && snapshot.active_session.is_none()
        });
        let blocked = self.busy || self.failed_operation.is_some();
        h_flex()
            .px_8()
            .py_6()
            .gap_4()
            .justify_between()
            .items_center()
            .child(
                v_flex()
                    .gap_1()
                    .min_w_0()
                    .child(
                        div()
                            .id("day-heading")
                            .role(Role::Heading)
                            .aria_label(title.clone())
                            .text_size(px(28.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .when(session.is_some(), |element| {
                        element.child(if closed {
                            Tag::secondary().child("Closed day")
                        } else {
                            Tag::success().child("Open day")
                        })
                    })
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child(if session.is_none() {
                                "A clear record of every cash movement."
                            } else if closed {
                                "Closed day · final count recorded"
                            } else {
                                "Open day · ready to record cash"
                            }),
                    ),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        Button::new("refresh")
                            .label("Refresh")
                            .icon(IconName::RotateCw)
                            .disabled(blocked)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.run(Operation::Refresh, window, cx)
                            })),
                    )
                    .when(can_open, |element| {
                        element.child(
                            Button::new("open-day")
                                .primary()
                                .label("Open day")
                                .disabled(blocked)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.change_mode(FormMode::Open, window, cx)
                                })),
                        )
                    })
                    .when(session.is_some() && !closed, |element| {
                        element.child(
                            Button::new("close-day")
                                .label("Close day")
                                .disabled(blocked)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.change_mode(FormMode::Close, window, cx)
                                })),
                        )
                    }),
            )
    }

    fn feedback(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .px_8()
            .when_some(self.notice.clone(), |element, notice| {
                element.child(
                    div()
                        .id("save-status")
                        .role(Role::Status)
                        .aria_label(notice.clone())
                        .py_3()
                        .child(Alert::success("save-status-message", notice)),
                )
            })
            .when_some(self.error.clone(), |element, error| {
                element.child(
                    v_flex()
                        .gap_3()
                        .mb_4()
                        .child(
                            div()
                                .id("operation-error")
                                .role(Role::Alert)
                                .aria_label(error.clone())
                                .child(Alert::error("operation-error-message", error)),
                        )
                        .when(self.failed_operation.is_some(), |element| {
                            element.child(
                                Button::new("retry-save")
                                    .label("Retry the same save")
                                    .disabled(self.busy)
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.submit(window, cx)),
                                    ),
                            )
                        }),
                )
            })
    }

    fn summary(&self) -> impl IntoElement {
        let balance = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.balance.as_ref());
        h_flex()
            .flex_wrap()
            .px_8()
            .py_6()
            .gap_8()
            .bg(rgb(0xedf0e9))
            .border_y_1()
            .border_color(rgb(LINE))
            .child(metric(
                "Expected in drawer",
                balance.map(|balance| balance.expected),
                true,
            ))
            .child(metric(
                "Opening cash",
                balance.map(|balance| balance.opening),
                false,
            ))
            .child(metric(
                "Cash in",
                balance.map(|balance| balance.cash_in),
                false,
            ))
            .child(metric(
                "Cash out",
                balance.map(|balance| balance.cash_out),
                false,
            ))
            .when_some(
                balance.and_then(|balance| balance.difference),
                |element, difference| {
                    element.child(metric("Count difference", Some(difference), false))
                },
            )
    }
}

pub(super) fn input_field(
    label: &'static str,
    state: &Entity<InputState>,
    disabled: bool,
) -> impl IntoElement {
    v_form().child(
        field().label(label).child(
            Input::new(state)
                .aria_label(label)
                .when(matches!(label, "Password" | "Confirm password"), |input| {
                    input.mask_toggle()
                })
                .disabled(disabled)
                .h(px(42.)),
        ),
    )
}

impl RegisterApp {
    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let blocked = self.busy || self.failed_operation.is_some();
        let mut sidebar = v_flex()
            .w(px(230.))
            .flex_shrink_0()
            .h_full()
            .bg(rgb(0xeef0e9))
            .border_r_1()
            .border_color(rgb(LINE))
            .p_5()
            .gap_5()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .size(px(36.))
                            .rounded_md()
                            .bg(rgb(GREEN))
                            .text_color(rgb(0xffffff))
                            .flex()
                            .items_center()
                            .justify_center()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(Icon::new(IconName::Building2)),
                    )
                    .child(
                        v_flex()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Cash Ledger"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child("Cash register"),
                            ),
                    ),
            )
            .child(section_label("Registers"));
        if let Some(snapshot) = &self.snapshot {
            sidebar = sidebar.child(v_flex().gap_2().children(snapshot.registers.iter().map(
                |register| {
                    let id = register.id;
                    Button::new(("register", id as u64))
                        .label(register.name.clone())
                        .w_full()
                        .selected(Some(id) == self.selection.register)
                        .disabled(blocked)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.selection = Selection {
                                register: Some(id),
                                ..Selection::default()
                            };
                            this.entry_pages.clear();
                            this.session_pages.clear();
                            this.mode = FormMode::Entry;
                            this.run(Operation::Refresh, window, cx);
                        }))
                },
            )));
        }
        sidebar = sidebar
            .child(
                Button::new("add-register")
                    .ghost()
                    .icon(IconName::Plus)
                    .label("Add register")
                    .disabled(blocked)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.change_mode(FormMode::Setup, window, cx)
                    })),
            )
            .child(section_label("Daily history"));
        let mut history = v_flex()
            .id("session-history")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .gap_2();
        if let Some(snapshot) = &self.snapshot {
            for session in &snapshot.sessions {
                let id = session.id;
                history = history.child(
                    Button::new(("session", id as u64))
                        .w_full()
                        .label(format!(
                            "{} · {}",
                            short_date(&session.business_date),
                            if session.closed { "Closed" } else { "Open" }
                        ))
                        .selected(Some(id) == self.selection.session)
                        .disabled(blocked)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.selection.session = Some(id);
                            this.selection.before_entry = None;
                            this.entry_pages.clear();
                            this.mode = FormMode::Entry;
                            this.closing_review = None;
                            this.run(Operation::Refresh, window, cx);
                        })),
                );
            }
            if snapshot.sessions.is_empty() {
                history = history.child(
                    div()
                        .text_sm()
                        .text_color(rgb(MUTED))
                        .child("Your saved days will appear here."),
                );
            }
        }
        sidebar
            .child(history)
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("newer-days")
                            .label("Newer")
                            .disabled(blocked || self.session_pages.is_empty())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.selection.before_session = this.session_pages.pop().flatten();
                                this.run(Operation::Refresh, window, cx);
                            })),
                    )
                    .child(
                        Button::new("older-days")
                            .label("Older")
                            .disabled(
                                blocked
                                    || !self
                                        .snapshot
                                        .as_ref()
                                        .is_some_and(|snapshot| snapshot.more_sessions),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                let cursor = this
                                    .snapshot
                                    .as_ref()
                                    .and_then(|snapshot| snapshot.sessions.last())
                                    .map(|session| session.id);
                                this.session_pages.push(this.selection.before_session);
                                this.selection.before_session = cursor;
                                this.run(Operation::Refresh, window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("Local workspace\nNo cloud account required"),
            )
    }

    fn ledger(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let blocked = self.busy || self.failed_operation.is_some();
        let mut ledger = v_flex()
            .flex_1()
            .min_w_0()
            .gap_4()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Cash movements"),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("cash-in-tab")
                                    .label("Cash in")
                                    .selected(self.selection.direction == Direction::In)
                                    .disabled(blocked)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.change_direction(Direction::In, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("cash-out-tab")
                                    .label("Cash out")
                                    .selected(self.selection.direction == Direction::Out)
                                    .disabled(blocked)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.change_direction(Direction::Out, window, cx)
                                    })),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .py_3()
                    .gap_4()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(div().w(px(70.)).child("Time"))
                    .child(div().flex_1().child("Reason"))
                    .child(div().w(px(130.)).text_right().child("Amount (CAD)")),
            );
        if let Some(snapshot) = &self.snapshot {
            for entry in &snapshot.entries {
                ledger = ledger.child(
                    h_flex()
                        .id(("movement", entry.id as u64))
                        .role(Role::Group)
                        .aria_label(format!(
                            "{}: {} CAD. {}. Local time {}.",
                            if self.selection.direction == Direction::In {
                                "Cash in"
                            } else {
                                "Cash out"
                            },
                            format_cad(entry.amount),
                            entry.reason,
                            local_time(&entry.created_at)
                        ))
                        .py_4()
                        .gap_4()
                        .items_start()
                        .border_b_1()
                        .border_color(rgb(LINE))
                        .child(
                            div()
                                .w(px(70.))
                                .flex_shrink_0()
                                .text_color(rgb(MUTED))
                                .child(local_time(&entry.created_at)),
                        )
                        .child(div().min_w_0().flex_1().child(entry.reason.clone()))
                        .child(
                            div()
                                .w(px(130.))
                                .flex_shrink_0()
                                .text_right()
                                .font_family("Consolas")
                                .text_color(rgb(GREEN))
                                .child(format!(
                                    "{}${}",
                                    if self.selection.direction == Direction::In {
                                        "+"
                                    } else {
                                        "−"
                                    },
                                    format_amount(entry.amount)
                                )),
                        ),
                );
            }
        }
        if self
            .snapshot
            .as_ref()
            .is_none_or(|snapshot| snapshot.entries.is_empty())
        {
            ledger = ledger.child(
                v_flex()
                    .py_12()
                    .gap_2()
                    .items_center()
                    .child(div().text_lg().child(if self.busy {
                        "Loading movements…"
                    } else {
                        "No movements yet"
                    }))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child("Saved cash entries appear here. Start with an opening count."),
                    ),
            );
        }
        ledger.child(
            h_flex()
                .justify_between()
                .items_center()
                .gap_3()
                .pt_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child("Newest first · local time"),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("previous-entries")
                                .label("Newer")
                                .disabled(blocked || self.entry_pages.is_empty())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.selection.before_entry = this.entry_pages.pop().flatten();
                                    this.run(Operation::Refresh, window, cx);
                                })),
                        )
                        .child(
                            Button::new("next-entries")
                                .label("Older")
                                .disabled(
                                    blocked
                                        || !self
                                            .snapshot
                                            .as_ref()
                                            .is_some_and(|snapshot| snapshot.more_entries),
                                )
                                .on_click(cx.listener(|this, _, window, cx| {
                                    let cursor = this
                                        .snapshot
                                        .as_ref()
                                        .and_then(|snapshot| snapshot.entries.last())
                                        .map(|entry| entry.id);
                                    this.entry_pages.push(this.selection.before_entry);
                                    this.selection.before_entry = cursor;
                                    this.run(Operation::Refresh, window, cx);
                                })),
                        ),
                ),
        )
    }

    fn form(&self, blocked: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let closed = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.session.as_ref())
            .is_some_and(|session| session.closed);
        let has_session = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.session.as_ref())
            .is_some();
        let mut form = v_flex().w(px(300.)).flex_shrink_0().gap_5();
        let (title, description, submit) = match self.mode {
            FormMode::Setup => (
                "Set up your register",
                "Name the drawer or counter you’ll track. This register uses Canadian dollars.",
                "Create register",
            ),
            FormMode::Open => (
                "Open a day",
                "Count your starting cash before recording any movements.",
                "Save opening count",
            ),
            FormMode::Entry if closed => ("Final count", "This day's cash record is complete.", ""),
            FormMode::Entry => (
                if self.selection.direction == Direction::In {
                    "Record cash in"
                } else {
                    "Record cash out"
                },
                "Record an amount and a clear reason. Entries stay in your daily history.",
                "Save movement",
            ),
            FormMode::Close => (
                "Close the day",
                "Count the cash physically in your drawer. Review the difference before closing.",
                if self.closing_review.is_some() {
                    "Confirm & close day"
                } else {
                    "Review count"
                },
            ),
        };
        form = form.child(
            v_flex()
                .gap_2()
                .child(
                    div()
                        .id("form-heading")
                        .role(Role::Heading)
                        .aria_label(title)
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(div().text_sm().text_color(rgb(MUTED)).child(description)),
        );
        match self.mode {
            FormMode::Setup => {
                form = form
                    .child(input_field("Register name", &self.name, blocked))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child("Currency · CAD"),
                    )
            }
            FormMode::Open => {
                form = form
                    .child(input_field(
                        "Business date · YYYY-MM-DD",
                        &self.date,
                        blocked,
                    ))
                    .child(input_field("Opening cash · CAD", &self.amount, blocked))
            }
            FormMode::Entry if closed || !has_session => {
                return form.child(
                    v_flex()
                        .gap_3()
                        .py_4()
                        .child(if closed {
                            "This day is closed. Its records are read-only."
                        } else {
                            "Open a day to begin recording cash."
                        })
                        .when_some(
                            self.snapshot
                                .as_ref()
                                .and_then(|snapshot| snapshot.balance.as_ref())
                                .and_then(|balance| balance.counted),
                            |element, counted| {
                                element.child(
                                    div()
                                        .font_family("Consolas")
                                        .id("closed-count")
                                        .role(Role::Group)
                                        .aria_label(format!(
                                            "Final counted cash: {} CAD",
                                            format_cad(counted)
                                        ))
                                        .child(format!("Counted · {}", format_cad(counted))),
                                )
                            },
                        ),
                );
            }
            FormMode::Entry => {
                form = form
                    .child(input_field("Amount · CAD", &self.amount, blocked))
                    .child(input_field("Reason", &self.reason, blocked))
            }
            FormMode::Close => {
                form = form.child(input_field(
                    "Counted cash · CAD",
                    &self.count,
                    blocked || self.closing_review.is_some(),
                ));
                if let Some(counted) = self.closing_review {
                    let expected = self
                        .snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.balance.as_ref())
                        .map(|balance| balance.expected)
                        .unwrap_or(0);
                    form = form.child(
                        v_flex()
                            .id("closing-review")
                            .role(Role::Group)
                            .aria_label(format!("Review before closing. Expected {} CAD. Counted {} CAD. Difference {} CAD. Closing is final.", format_cad(expected), format_cad(counted), counted.checked_sub(expected).map(format_cad).unwrap_or_else(|| "outside the supported range".into())))
                            .gap_3()
                            .py_4()
                            .border_y_1()
                            .border_color(rgb(LINE))
                            .child(div().child(format!("Expected · {}", format_cad(expected))))
                            .child(div().child(format!("Counted · {}", format_cad(counted))))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(
                                match counted.checked_sub(expected) {
                                    Some(difference) => {
                                        format!("Difference · {}", format_cad(difference))
                                    }
                                    None => {
                                        "Difference is outside the supported amount range.".into()
                                    }
                                },
                            ))
                            .child(div().text_sm().text_color(rgb(MUTED)).child(
                                "Closing is final. New movements cannot be added to this day.",
                            ))
                            .when(self.profile.as_ref().is_some_and(|profile| counted.checked_sub(expected).is_some_and(|difference| difference.unsigned_abs() > profile.closing_variance_threshold_cents as u64)), |element| {
                                element.child(div().id("variance-warning").role(Role::Alert).text_color(rgb(0x9b3030)).child("The count difference exceeds your company's warning amount. Check the count before confirming."))
                            })
                            .when_some(self.profile.as_ref().filter(|profile| !profile.rules.is_empty()), |element, profile| {
                                element.child(div().text_sm().child(format!("Company rules: {}", profile.rules)))
                            }),
                    );
                }
            }
        }
        form.child(
            Button::new("submit-form")
                .primary()
                .w_full()
                .h(px(42.))
                .label(if self.failed_operation.is_some() {
                    "Retry the same save"
                } else {
                    submit
                })
                .loading(self.busy)
                .disabled(self.busy || self.snapshot.is_none())
                .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
        )
        .when(self.mode != FormMode::Entry && has_session, |element| {
            element.child(
                Button::new("cancel-form")
                    .ghost()
                    .label("Cancel")
                    .disabled(blocked)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.change_mode(FormMode::Entry, window, cx)
                    })),
            )
        })
        .child(
            div()
                .text_xs()
                .text_color(rgb(MUTED))
                .child("Amounts are saved exactly to the cent."),
        )
    }
}

fn metric(label: &'static str, amount: Option<i64>, primary: bool) -> impl IntoElement {
    let text = amount.map(format_cad).unwrap_or_else(|| "—".into());
    v_flex()
        .id(label)
        .role(Role::Group)
        .aria_label(format!("{label}: {text} CAD"))
        .gap_2()
        .min_w(px(130.))
        .flex_1()
        .child(div().text_sm().text_color(rgb(MUTED)).child(label))
        .child(
            div()
                .font_family("Consolas")
                .text_size(px(if primary && text.len() < 16 { 30. } else { 22. }))
                .font_weight(if primary {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                })
                .child(text),
        )
}

fn section_label(label: &'static str) -> impl IntoElement {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(MUTED))
        .child(label)
}
fn friendly_date(date: &str) -> String {
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|date| date.format("%A, %B %-d").to_string())
        .unwrap_or_else(|_| date.into())
}
fn short_date(date: &str) -> String {
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|date| date.format("%d %b %Y").to_string())
        .unwrap_or_else(|_| date.into())
}
fn local_time(timestamp: &str) -> String {
    chrono::NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d %H:%M:%S")
        .map(|time| {
            time.and_utc()
                .with_timezone(&Local)
                .format("%H:%M")
                .to_string()
        })
        .unwrap_or_else(|_| timestamp.into())
}
