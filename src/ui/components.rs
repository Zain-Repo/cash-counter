//! Shared component state and reporting shortcuts for the native workspace.

use chrono::{Datelike, Days, NaiveDate};
use gpui::{App, Div, SharedString, Styled as _, div};
use gpui_component::{ActiveTheme, searchable_list::SearchableListItem};

/// A theme-aware content surface for the register's cards and panels.
pub(super) fn panel(cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .bg(cx.theme().background)
        .border_1()
        .border_color(cx.theme().border)
        .rounded(cx.theme().radius_lg)
}

/// Register names are presentation only; selection always uses the database identity.
#[derive(Clone)]
pub(super) struct RegisterChoice {
    pub id: Option<i64>,
    pub name: SharedString,
}

impl SearchableListItem for RegisterChoice {
    type Value = Option<i64>;

    fn title(&self) -> SharedString {
        self.name.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

#[derive(Clone, Copy)]
pub(super) enum ReportPeriod {
    Today,
    LastSevenDays,
    ThisMonth,
}

impl ReportPeriod {
    pub fn range(self, today: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
        let start = match self {
            Self::Today => today,
            Self::LastSevenDays => today.checked_sub_days(Days::new(6))?,
            Self::ThisMonth => today.with_day(1)?,
        };
        Some((start, today))
    }
}
