//! Shared component state and reporting shortcuts for the native workspace.

use chrono::{Datelike, Days, NaiveDate};
use gpui::SharedString;
use gpui_component::searchable_list::SearchableListItem;

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
