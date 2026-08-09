use crate::{
    app::{command::ReportAction, dispatcher::ActionCollector, state::AppState},
    domain::{
        AccountId, AccountScope, CategoryId, DateRange, ReportFilter, ReportKind, ReportRequest,
    },
};
use std::collections::BTreeSet;
use time::{Date, Duration, Month, OffsetDateTime};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TimeframePreset {
    ThisMonth,
    LastMonth,
    LastThreeMonths,
    ThisYear,
    AllDates,
    Custom,
}
impl TimeframePreset {
    const ALL: [(Self, &'static str); 6] = [
        (Self::ThisMonth, "This Month"),
        (Self::LastMonth, "Last Month"),
        (Self::LastThreeMonths, "Last 3 Months"),
        (Self::ThisYear, "This Year"),
        (Self::AllDates, "All Dates"),
        (Self::Custom, "Custom"),
    ];

    fn label(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(value, _)| *value == self)
            .unwrap()
            .1
    }

    fn dates(self, today: Date, custom: DateRange) -> DateRange {
        let month_start = Date::from_calendar_date(today.year(), today.month(), 1).unwrap();
        let year_start = Date::from_calendar_date(today.year(), Month::January, 1).unwrap();
        match self {
            Self::ThisMonth => DateRange {
                start: month_start,
                end: today,
            },
            Self::LastMonth => {
                let end = month_start - Duration::DAY;
                DateRange {
                    start: Date::from_calendar_date(end.year(), end.month(), 1).unwrap(),
                    end,
                }
            }
            Self::LastThreeMonths => {
                let mut start = month_start;
                for _ in 0..2 {
                    let previous = start - Duration::DAY;
                    start = Date::from_calendar_date(previous.year(), previous.month(), 1).unwrap();
                }
                DateRange { start, end: today }
            }
            Self::ThisYear => DateRange {
                start: year_start,
                end: today,
            },
            Self::AllDates => DateRange {
                start: Date::from_calendar_date(1, Month::January, 1).unwrap(),
                end: today,
            },
            Self::Custom => custom,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReportControls {
    pub kind: ReportKind,
    pub timeframe: TimeframePreset,
    pub custom_start: String,
    pub custom_end: String,
    pub accounts: AccountScope,
    pub account_ids: BTreeSet<AccountId>,
    pub category_ids: BTreeSet<CategoryId>,
    pub category_group_ids: BTreeSet<crate::domain::CategoryGroupId>,
    pub payee_ids: BTreeSet<crate::domain::PayeeId>,
}
impl Default for ReportControls {
    fn default() -> Self {
        let today = OffsetDateTime::now_utc().date();
        Self {
            kind: ReportKind::SpendingByCategory,
            timeframe: TimeframePreset::ThisMonth,
            custom_start: crate::ui::format::date(
                Date::from_calendar_date(today.year(), today.month(), 1).unwrap(),
            ),
            custom_end: crate::ui::format::date(today),
            accounts: AccountScope::Both,
            account_ids: BTreeSet::new(),
            category_ids: BTreeSet::new(),
            category_group_ids: BTreeSet::new(),
            payee_ids: BTreeSet::new(),
        }
    }
}

impl ReportControls {
    fn custom_dates(&self) -> Result<DateRange, Vec<(&'static str, String)>> {
        let parse =
            |text: &str| crate::app::transaction_editor::parse_transaction_date(text).map(|d| d.0);
        let start = parse(&self.custom_start)
            .map_err(|message| vec![("dates", format!("From: {message}"))])?;
        let end = parse(&self.custom_end)
            .map_err(|message| vec![("dates", format!("Through: {message}"))])?;
        if start > end {
            return Err(vec![("dates", "From must be on or before Through.".into())]);
        }
        Ok(DateRange { start, end })
    }

    pub(crate) fn request_at(
        &self,
        today: Date,
        valid_categories: &BTreeSet<CategoryId>,
        valid_accounts: &BTreeSet<AccountId>,
    ) -> Result<ReportRequest, Vec<(&'static str, String)>> {
        let custom = if self.timeframe == TimeframePreset::Custom {
            self.custom_dates()?
        } else {
            DateRange {
                start: today,
                end: today,
            }
        };
        let mut errors = vec![];
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
        if !self.account_ids.is_empty()
            && !valid_accounts
                .iter()
                .any(|id| self.account_ids.contains(id))
        {
            errors.push(("accounts", "Select at least one available account.".into()));
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        Ok(ReportRequest {
            kind: self.kind,
            filter: ReportFilter {
                dates: self.timeframe.dates(today, custom),
                account_ids: self.account_ids.clone(),
                category_group_ids: self.category_group_ids.clone(),
                category_ids: self.category_ids.clone(),
                payee_ids: self.payee_ids.clone(),
                accounts: self.accounts,
            },
        })
    }

    fn request(
        &self,
        categories: &BTreeSet<CategoryId>,
        accounts: &BTreeSet<AccountId>,
    ) -> Result<ReportRequest, Vec<(&'static str, String)>> {
        self.request_at(OffsetDateTime::now_utc().date(), categories, accounts)
    }
}

pub(crate) fn format_minor_units(cents: i64) -> String {
    crate::ui::format::money(crate::domain::Money::from_minor_units(cents))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChartKind {
    Donut,
    GroupedBars,
    Trend,
    NetWorth,
}

fn chart_for(kind: ReportKind) -> ChartKind {
    match kind {
        ReportKind::SpendingByCategory | ReportKind::SpendingByPayee => ChartKind::Donut,
        ReportKind::IncomeExpense => ChartKind::GroupedBars,
        ReportKind::NetWorth => ChartKind::NetWorth,
        ReportKind::Spending | ReportKind::MonthlySpendingTrend | ReportKind::BudgetProgress => {
            ChartKind::Trend
        }
    }
}

fn show_chart(
    ui: &mut egui::Ui,
    kind: ChartKind,
    points: &[crate::app::view_model::ReportPointView],
) {
    let tokens = crate::ui::style::SemanticTokens::from_visuals(ui.visuals());
    let height = ui.spacing().interact_size.y * 6.0;
    let (response, painter) = ui.allocate_painter(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let rect = response.rect.shrink(8.0);
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, tokens.border),
        egui::StrokeKind::Inside,
    );
    if points.is_empty() {
        return;
    }
    let values: Vec<i64> = points
        .iter()
        .map(|p| match kind {
            ChartKind::GroupedBars => p.income_cents.max(p.expense_cents),
            ChartKind::Donut => p
                .expense_cents
                .saturating_abs()
                .max(p.net_cents.saturating_abs()),
            _ => p.net_cents,
        })
        .collect();
    let maximum = values
        .iter()
        .map(|v| v.unsigned_abs())
        .max()
        .unwrap_or(1)
        .max(1) as f32;
    let baseline = if matches!(kind, ChartKind::NetWorth | ChartKind::Trend) {
        rect.center().y
    } else {
        rect.bottom()
    };
    let step = rect.width() / points.len().max(1) as f32;
    let position = |index: usize, value: i64| {
        egui::pos2(
            rect.left() + (index as f32 + 0.5) * step,
            baseline
                - value as f32 / maximum
                    * rect.height()
                    * if baseline == rect.bottom() { 0.9 } else { 0.45 },
        )
    };
    match kind {
        ChartKind::GroupedBars => {
            for (index, point) in points.iter().enumerate() {
                let center = rect.left() + (index as f32 + 0.5) * step;
                for (offset, value, color) in [
                    (-step * 0.18, point.income_cents, tokens.positive_money),
                    (step * 0.04, point.expense_cents, tokens.negative_money),
                ] {
                    let top = position(index, value.abs()).y;
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(center + offset, top),
                            egui::pos2(center + offset + step * 0.14, baseline),
                        ),
                        2.0,
                        color,
                    );
                }
            }
        }
        ChartKind::Donut => {
            let total: u64 = values.iter().map(|v| v.unsigned_abs()).sum::<u64>().max(1);
            let palette = [
                tokens.negative_money,
                tokens.warning,
                tokens.ready_to_assign,
                tokens.positive_money,
                tokens.selection,
            ];
            let center = rect.center();
            let radius = rect.height().min(rect.width()) * 0.38;
            let mut angle = -std::f32::consts::FRAC_PI_2;
            for (index, value) in values.iter().enumerate() {
                let sweep = std::f32::consts::TAU * value.unsigned_abs() as f32 / total as f32;
                let segments = ((sweep * 16.0).ceil() as usize).max(1);
                let mut wedge = Vec::with_capacity(segments + 2);
                wedge.push(center);
                for segment in 0..=segments {
                    let a = angle + sweep * segment as f32 / segments as f32;
                    wedge.push(center + egui::vec2(a.cos(), a.sin()) * radius);
                }
                painter.add(egui::Shape::convex_polygon(
                    wedge,
                    palette[index % palette.len()],
                    egui::Stroke::NONE,
                ));
                angle += sweep;
            }
            painter.circle_filled(center, radius * 0.52, tokens.panel);
        }
        ChartKind::Trend | ChartKind::NetWorth => {
            let color = if kind == ChartKind::NetWorth {
                tokens.ready_to_assign
            } else {
                tokens.negative_money
            };
            for index in 1..points.len() {
                painter.line_segment(
                    [
                        position(index - 1, points[index - 1].net_cents),
                        position(index, points[index].net_cents),
                    ],
                    egui::Stroke::new(2.0, color),
                );
            }
            for (index, point) in points.iter().enumerate() {
                painter.circle_filled(position(index, point.net_cents), 3.0, color);
            }
        }
    }
    if kind == ChartKind::Donut {
        let palette = [
            tokens.negative_money,
            tokens.warning,
            tokens.ready_to_assign,
            tokens.positive_money,
            tokens.selection,
        ];
        ui.horizontal_wrapped(|ui| {
            for (index, point) in points.iter().enumerate() {
                ui.colored_label(
                    palette[index % palette.len()],
                    format!(
                        "● {} {}",
                        point.label,
                        format_minor_units(point.expense_cents.saturating_abs())
                    ),
                );
            }
        });
    }
}

fn show_table(ui: &mut egui::Ui, points: &[crate::app::view_model::ReportPointView]) {
    ui.strong("Report data (accessible chart alternative)");
    egui::Grid::new("report-data-table")
        .striped(true)
        .show(ui, |ui| {
            ui.strong("Period / group");
            ui.strong("Income");
            ui.strong("Expense");
            ui.strong("Net");
            ui.end_row();
            for point in points {
                ui.label(&point.label);
                ui.monospace(format_minor_units(point.income_cents));
                ui.monospace(format_minor_units(point.expense_cents));
                ui.monospace(format_minor_units(point.net_cents));
                ui.end_row();
            }
        });
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
        for (kind, label) in [
            (ReportKind::SpendingByCategory, "Spending by Category"),
            (ReportKind::SpendingByPayee, "Spending by Payee"),
            (ReportKind::IncomeExpense, "Income vs Expense"),
            (ReportKind::NetWorth, "Net Worth"),
        ] {
            ui.selectable_value(&mut controls.kind, kind, label);
        }
        egui::ComboBox::from_id_salt("more-reports")
            .selected_text("More reports")
            .show_ui(ui, |ui| {
                for (kind, label) in [
                    (ReportKind::Spending, "Spending"),
                    (ReportKind::MonthlySpendingTrend, "Monthly trend"),
                    (ReportKind::BudgetProgress, "Budget progress"),
                ] {
                    ui.selectable_value(&mut controls.kind, kind, label);
                }
            });
    });
    ui.horizontal_wrapped(|ui| {
        egui::ComboBox::from_label("Timeframe")
            .selected_text(controls.timeframe.label())
            .show_ui(ui, |ui| {
                for (preset, label) in TimeframePreset::ALL {
                    ui.selectable_value(&mut controls.timeframe, preset, label);
                }
            });
        if controls.timeframe == TimeframePreset::Custom {
            ui.label("From");
            crate::ui::widgets::date_picker(ui, &mut controls.custom_start);
            ui.label("Through");
            crate::ui::widgets::date_picker(ui, &mut controls.custom_end);
        }
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
                let model = crate::ui::empty_state::model(
                    crate::ui::empty_state::EmptyState::ReportsWithoutData,
                    true,
                    !state.accounts.is_empty(),
                );
                crate::ui::empty_state::show(ui, &model, commands);
                show_table(ui, &view.points);
            } else {
                let kind = state
                    .report_query
                    .current_request
                    .as_ref()
                    .map_or(controls.kind, |r| r.kind);
                show_chart(ui, chart_for(kind), &view.points);
                show_table(ui, &view.points);
                ui.strong(format!("Total {}", format_minor_units(view.total_cents)));
            }
            if ui.button("Export CSV").clicked()
                && let Some(destination) = rfd::FileDialog::new()
                    .set_file_name("report.csv")
                    .save_file()
            {
                commands.push(ReportAction::ExportCsv { destination });
            }
            ui.weak("Aggregated results only");
        } else if !query.refresh_active && query.safe_failure.is_none() {
            ui.strong("No report results");
            ui.label("Choose a timeframe and filters, then refresh.");
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
    fn date(y: i32, m: Month, d: u8) -> Date {
        Date::from_calendar_date(y, m, d).unwrap()
    }
    fn request(preset: TimeframePreset, today: Date) -> DateRange {
        let controls = ReportControls {
            timeframe: preset,
            ..Default::default()
        };
        controls
            .request_at(today, &BTreeSet::new(), &BTreeSet::new())
            .unwrap()
            .filter
            .dates
    }

    #[test]
    fn presets_handle_month_and_year_boundaries() {
        assert_eq!(
            request(TimeframePreset::LastMonth, date(2025, Month::January, 15)),
            DateRange {
                start: date(2024, Month::December, 1),
                end: date(2024, Month::December, 31)
            }
        );
        assert_eq!(
            request(
                TimeframePreset::LastThreeMonths,
                date(2025, Month::January, 15)
            ),
            DateRange {
                start: date(2024, Month::November, 1),
                end: date(2025, Month::January, 15)
            }
        );
        assert_eq!(
            request(TimeframePreset::ThisYear, date(2025, Month::December, 31)).start,
            date(2025, Month::January, 1)
        );
    }
    #[test]
    fn presets_handle_leap_years() {
        assert_eq!(
            request(TimeframePreset::LastMonth, date(2024, Month::March, 1)).end,
            date(2024, Month::February, 29)
        );
    }
    #[test]
    fn custom_ranges_are_validated_and_use_transaction_date_format() {
        let valid = ReportControls {
            timeframe: TimeframePreset::Custom,
            custom_start: "02/29/2024".into(),
            custom_end: "03/01/2024".into(),
            ..Default::default()
        };
        assert_eq!(
            valid
                .request_at(
                    date(2024, Month::April, 1),
                    &BTreeSet::new(),
                    &BTreeSet::new()
                )
                .unwrap()
                .filter
                .dates
                .start,
            date(2024, Month::February, 29)
        );
        let invalid = ReportControls {
            custom_start: "03/02/2024".into(),
            custom_end: "03/01/2024".into(),
            ..valid
        };
        assert!(
            invalid
                .request_at(
                    date(2024, Month::April, 1),
                    &BTreeSet::new(),
                    &BTreeSet::new()
                )
                .is_err()
        );
    }
    #[test]
    fn every_tab_maps_to_its_typed_query() {
        for kind in [
            ReportKind::SpendingByCategory,
            ReportKind::SpendingByPayee,
            ReportKind::IncomeExpense,
            ReportKind::NetWorth,
        ] {
            let controls = ReportControls {
                kind,
                ..Default::default()
            };
            assert_eq!(
                controls
                    .request_at(
                        date(2025, Month::May, 1),
                        &BTreeSet::new(),
                        &BTreeSet::new()
                    )
                    .unwrap()
                    .kind,
                kind
            );
        }
    }
    #[test]
    fn chart_kind_is_derived_without_mutating_projection() {
        let points = vec![crate::app::view_model::ReportPointView {
            label: "A".into(),
            income_cents: 10,
            expense_cents: 4,
            net_cents: 6,
        }];
        let copy = points.clone();
        assert_eq!(chart_for(ReportKind::IncomeExpense), ChartKind::GroupedBars);
        assert_eq!(points, copy);
    }
    #[test]
    fn empty_projection_has_a_chart_mapping() {
        assert_eq!(chart_for(ReportKind::NetWorth), ChartKind::NetWorth);
        let points: Vec<crate::app::view_model::ReportPointView> = vec![];
        assert!(points.is_empty());
    }
    #[test]
    fn signed_minor_units_use_shared_money_formatter() {
        assert_eq!(format_minor_units(-123_456), "-$1,234.56");
        assert_eq!(format_minor_units(i64::MIN), "-$92,233,720,368,547,758.08");
    }
}
