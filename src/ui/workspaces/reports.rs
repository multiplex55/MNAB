use crate::{
    app::{command::ReportAction, dispatcher::ActionCollector, state::AppState},
    domain::{
        AccountId, AccountScope, CategoryId, DateRange, ReportFilter, ReportKind, ReportRequest,
    },
};
use std::collections::BTreeSet;
use time::{Date, Month, OffsetDateTime};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReportControls {
    pub kind: ReportKind,
    pub dates: DateRange,
    pub accounts: AccountScope,
    pub account_ids: BTreeSet<AccountId>,
    pub category_ids: BTreeSet<CategoryId>,
    pub category_group_ids: BTreeSet<crate::domain::CategoryGroupId>,
    pub payee_ids: BTreeSet<crate::domain::PayeeId>,
}
impl Default for ReportControls {
    fn default() -> Self {
        let today = OffsetDateTime::now_utc().date();
        let start = Date::from_calendar_date(today.year(), Month::January, 1).expect("valid date");
        Self {
            kind: ReportKind::SpendingByCategory,
            dates: DateRange { start, end: today },
            accounts: AccountScope::Both,
            account_ids: BTreeSet::new(),
            category_ids: BTreeSet::new(),
            category_group_ids: BTreeSet::new(),
            payee_ids: BTreeSet::new(),
        }
    }
}

impl ReportControls {
    pub(crate) fn request(
        &self,
        valid_categories: &BTreeSet<CategoryId>,
        valid_accounts: &BTreeSet<AccountId>,
    ) -> Result<ReportRequest, Vec<(&'static str, String)>> {
        let mut errors = vec![];
        if self.dates.start > self.dates.end {
            errors.push(("dates", "From must be on or before Through.".into()));
        }
        if let Some(id) = self
            .category_ids
            .iter()
            .find(|id| !valid_categories.contains(id))
        {
            errors.push(("categories", format!("Category {id} no longer exists.")));
        }
        if let Some(id) = self
            .account_ids
            .iter()
            .find(|id| !valid_accounts.contains(id))
        {
            errors.push(("accounts", format!("Account {id} no longer exists.")));
        }
        if !self.account_ids.is_empty() {
            let eligible = valid_accounts
                .iter()
                .any(|id| self.account_ids.contains(id));
            if !eligible {
                errors.push(("accounts", "Select at least one available account.".into()));
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        Ok(ReportRequest {
            kind: self.kind,
            filter: ReportFilter {
                dates: self.dates,
                account_ids: self.account_ids.clone(),
                category_group_ids: self.category_group_ids.clone(),
                category_ids: self.category_ids.clone(),
                payee_ids: self.payee_ids.clone(),
                accounts: self.accounts,
            },
        })
    }
}

fn edit_date(ui: &mut egui::Ui, label: &str, date: &mut Date) {
    let (mut y, mut m, mut d) = (date.year(), u8::from(date.month()), date.day());
    ui.label(label);
    ui.add(egui::DragValue::new(&mut y).range(1..=9999));
    ui.add(egui::DragValue::new(&mut m).range(1..=12));
    ui.add(egui::DragValue::new(&mut d).range(1..=31));
    if let Ok(month) = Month::try_from(m) {
        if let Ok(value) = Date::from_calendar_date(y, month, d) {
            *date = value;
        }
    }
}
pub(crate) fn format_minor_units(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let n = cents.unsigned_abs();
    format!("{sign}${}.{:02}", n / 100, n % 100)
}

pub fn show(ui: &mut egui::Ui, state: &AppState, commands: &mut ActionCollector) {
    ui.heading("Reports");
    let id = ui.id().with("report-controls");
    let mut controls = ui
        .ctx()
        .data_mut(|d| d.get_temp::<ReportControls>(id).unwrap_or_default());
    let error_id = id.with("errors");
    let mut errors = ui.ctx().data_mut(|d| {
        d.get_temp::<Vec<(&'static str, String)>>(error_id)
            .unwrap_or_default()
    });
    ui.horizontal_wrapped(|ui| {
        egui::ComboBox::from_label("Report")
            .selected_text(format!("{:?}", controls.kind))
            .show_ui(ui, |ui| {
                for (kind, label) in [
                    (ReportKind::Spending, "Spending"),
                    (ReportKind::SpendingByCategory, "Spending by category"),
                    (ReportKind::SpendingByPayee, "Spending by payee"),
                    (ReportKind::MonthlySpendingTrend, "Monthly spending trend"),
                    (ReportKind::IncomeExpense, "Income and expense"),
                    (ReportKind::NetWorth, "Net worth"),
                    (ReportKind::BudgetProgress, "Budget progress"),
                ] {
                    ui.selectable_value(&mut controls.kind, kind, label);
                }
            });
        edit_date(ui, "From (Y M D)", &mut controls.dates.start);
        edit_date(ui, "Through (Y M D)", &mut controls.dates.end);
        egui::ComboBox::from_label("Accounts")
            .selected_text(format!("{:?}", controls.accounts))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut controls.accounts,
                    AccountScope::Both,
                    "On-budget and tracking",
                );
                ui.selectable_value(
                    &mut controls.accounts,
                    AccountScope::OnBudget,
                    "On-budget only",
                );
                ui.selectable_value(
                    &mut controls.accounts,
                    AccountScope::Tracking,
                    "Tracking only",
                );
            });
    });
    let valid_categories: BTreeSet<_> = state
        .category_catalog
        .last_successful
        .as_ref()
        .into_iter()
        .flat_map(|v| &v.groups)
        .flat_map(|g| &g.categories)
        .map(|c| c.id)
        .collect();
    let valid_accounts: BTreeSet<_> = state.accounts.iter().map(|a| a.id).collect();
    if let Some(catalog) = &state.category_catalog.last_successful {
        ui.collapsing("Categories (all when none selected)", |ui| {
            for group in &catalog.groups {
                ui.label(&group.name);
                for category in &group.categories {
                    let mut selected = controls.category_ids.contains(&category.id);
                    if ui.checkbox(&mut selected, &category.name).changed() {
                        if selected {
                            controls.category_ids.insert(category.id);
                        } else {
                            controls.category_ids.remove(&category.id);
                        }
                    }
                }
            }
        });
    }
    if ui.button("Refresh").clicked() {
        match controls.request(&valid_categories, &valid_accounts) {
            Ok(request) => {
                errors.clear();
                commands.push(ReportAction::Refresh(request));
            }
            Err(found) => errors = found,
        }
    }
    for (field, message) in &errors {
        ui.colored_label(ui.visuals().error_fg_color, format!("{field}: {message}"));
    }
    ui.separator();
    let query = &state.report_query.view;
    if state.active_budget.is_none() {
        ui.strong("No budget is open");
        ui.label("Open a budget to run a report.");
    } else {
        if query.refresh_active {
            ui.spinner();
            ui.label(if query.last_successful.is_some() {
                "Refreshing report… Previous result retained."
            } else {
                "Loading report…"
            });
        }
        if let Some(message) = &query.safe_failure {
            ui.colored_label(ui.visuals().error_fg_color, message);
            if query.last_successful.is_some() {
                ui.weak("The previous result has been retained.");
            }
            if ui.button("Try again").clicked() {
                if let Some(request) = state.report_query.current_request.clone() {
                    commands.push(ReportAction::Retry(request));
                }
            }
        }
        if let Some(view) = &query.last_successful {
            ui.heading(&view.title);
            if view.points.is_empty() {
                ui.label("No aggregated results for these filters.");
            } else {
                for point in &view.points {
                    ui.horizontal(|ui| {
                        ui.label(&point.label);
                        if point.income_cents != 0 {
                            ui.monospace(format!(
                                "Income {}",
                                format_minor_units(point.income_cents)
                            ));
                        }
                        if point.expense_cents != 0 {
                            ui.monospace(format!(
                                "Expense {}",
                                format_minor_units(point.expense_cents)
                            ));
                        }
                        ui.monospace(format!("Net {}", format_minor_units(point.net_cents)));
                    });
                }
                ui.strong(format!("Total {}", format_minor_units(view.total_cents)));
            }
            if ui.button("Export CSV").clicked() {
                let destination = state
                    .database_path
                    .as_ref()
                    .and_then(|p| p.parent())
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .join("report.csv");
                commands.push(ReportAction::ExportCsv { destination });
            }
            ui.weak("Aggregated results only");
        } else if !query.refresh_active && query.safe_failure.is_none() {
            ui.strong("No report results");
            ui.label("Choose a date range and filters, then refresh.");
        }
    }
    ui.ctx().data_mut(|d| {
        d.insert_temp(id, controls);
        d.insert_temp(error_id, errors);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_build_every_kind_and_scope_without_loss() {
        for kind in [
            ReportKind::Spending,
            ReportKind::SpendingByCategory,
            ReportKind::SpendingByPayee,
            ReportKind::MonthlySpendingTrend,
            ReportKind::IncomeExpense,
            ReportKind::NetWorth,
            ReportKind::BudgetProgress,
        ] {
            for scope in [
                AccountScope::OnBudget,
                AccountScope::Tracking,
                AccountScope::Both,
            ] {
                let controls = ReportControls {
                    kind,
                    accounts: scope,
                    ..Default::default()
                };
                let request = controls
                    .request(&BTreeSet::new(), &BTreeSet::new())
                    .unwrap();
                assert_eq!(request.kind, kind);
                assert_eq!(request.filter.accounts, scope);
                assert_eq!(request.filter.dates, controls.dates);
            }
        }
    }

    #[test]
    fn inverted_dates_are_field_errors_and_cannot_build_a_request() {
        let mut controls = ReportControls::default();
        std::mem::swap(&mut controls.dates.start, &mut controls.dates.end);
        let errors = controls
            .request(&BTreeSet::new(), &BTreeSet::new())
            .unwrap_err();
        assert!(errors.iter().any(|(field, _)| *field == "dates"));
    }

    #[test]
    fn signed_minor_units_never_use_floating_point() {
        assert_eq!(format_minor_units(-123_456), "-$1234.56");
        assert_eq!(format_minor_units(5), "$0.05");
        assert_eq!(format_minor_units(i64::MIN), "-$92233720368547758.08");
    }
}
