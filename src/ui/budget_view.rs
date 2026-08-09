//! Budget workspace built exclusively from the immutable, authoritative `BudgetMonthView`.
//!
//! Widget state lives in [`BudgetUiState`].  In particular, an assignment draft is not removed
//! when a command is submitted: the runtime may reject validation, persistence, or a stale
//! revision and the user must not lose their input.
use crate::{
    app::{
        command::{
            ApplicationAction, AssignmentBatch, AssignmentBatchChange, AssignmentCommand,
            FinancialCommand,
        },
        dispatcher::ActionCollector,
        view_model::{BudgetMonthView, CategoryRowView},
    },
    domain::{BudgetAssignment, BudgetMonth, CategoryGroupId, CategoryId, Money},
    service::assignment_service::{
        AutoAssignInput, AutoAssignPreview, AutoAssignStrategy, propose_auto_assign_at_revision,
    },
};
use std::collections::BTreeSet;

pub fn parse_usd_input(text: &str) -> Result<Money, crate::domain::MoneyError> {
    if text.trim().is_empty() {
        Ok(Money::ZERO)
    } else {
        text.parse()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssignmentKey {
    Enter,
    Escape,
    Tab,
    BackTab,
    Up,
    Down,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AssignmentEdit {
    category_id: CategoryId,
    original: String,
    draft: String,
    error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoveMoneyPreview {
    pub from: CategoryId,
    pub to: CategoryId,
    pub amount: Money,
    pub changes: [AssignmentBatchChange; 2],
}

/// Session-only presentation state, keyed by stable domain identities rather than row indices.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetUiState {
    collapsed_groups: BTreeSet<CategoryGroupId>,
    pub focused_category: Option<CategoryId>,
    pub selected_categories: BTreeSet<CategoryId>,
    edit: Option<AssignmentEdit>,
    pending_assignments: BTreeSet<CategoryId>,
    projection_revision: Option<u64>,
    pub auto_strategy: AutoAssignStrategy,
    pub auto_preview: Option<AutoAssignPreview>,
    pub move_preview: Option<MoveMoneyPreview>,
}
impl Default for BudgetUiState {
    fn default() -> Self {
        Self {
            collapsed_groups: BTreeSet::new(),
            focused_category: None,
            selected_categories: BTreeSet::new(),
            edit: None,
            pending_assignments: BTreeSet::new(),
            projection_revision: None,
            auto_strategy: AutoAssignStrategy::Underfunded,
            auto_preview: None,
            move_preview: None,
        }
    }
}
impl BudgetUiState {
    pub fn begin_edit(&mut self, id: CategoryId, value: Money) {
        let value = crate::ui::format::money(value);
        self.edit = Some(AssignmentEdit {
            category_id: id,
            original: value.clone(),
            draft: value,
            error: None,
        });
        self.focused_category = Some(id);
    }
    pub fn edit_text_mut(&mut self) -> Option<&mut String> {
        self.edit.as_mut().map(|e| &mut e.draft)
    }
    pub fn edit_error(&self) -> Option<&str> {
        self.edit.as_ref().and_then(|e| e.error.as_deref())
    }
    pub fn report_commit_failure(&mut self, message: impl Into<String>) {
        if let Some(edit) = &mut self.edit {
            edit.error = Some(message.into());
            self.pending_assignments.remove(&edit.category_id);
        }
    }
    pub fn cancel_edit(&mut self) -> Option<String> {
        self.edit.take().map(|edit| edit.original)
    }
    pub fn toggle_selected(&mut self, id: CategoryId) {
        if !self.selected_categories.remove(&id) {
            self.selected_categories.insert(id);
        }
    }
    pub fn set_collapsed(&mut self, id: CategoryGroupId, collapsed: bool) {
        if collapsed {
            self.collapsed_groups.insert(id);
        } else {
            self.collapsed_groups.remove(&id);
        }
    }
    pub fn is_collapsed(&self, id: CategoryGroupId) -> bool {
        self.collapsed_groups.contains(&id)
    }
    pub fn restore_focus(&mut self, visible: &[CategoryId]) {
        if self
            .focused_category
            .is_some_and(|id| !visible.contains(&id))
        {
            self.focused_category = None;
        }
    }
    /// Reconciles ephemeral editor state with a newly accepted projection. Collapse state is
    /// deliberately untouched: it belongs to stable group identities, not a projection revision.
    pub fn accept_projection(&mut self, view: &BudgetMonthView) {
        if self.projection_revision != Some(view.calculation_revision) {
            self.pending_assignments.clear();
            self.projection_revision = Some(view.calculation_revision);
        }
        let categories: Vec<_> = view.rows.iter().map(|row| row.category_id).collect();
        self.restore_focus(&categories);
        self.selected_categories
            .retain(|id| categories.contains(id));
        if self
            .edit
            .as_ref()
            .is_some_and(|edit| !categories.contains(&edit.category_id))
        {
            self.edit = None;
        }
    }
    /// Handles editor keys without destroying a draft. Enter returns a reversible replacement;
    /// navigation commits first and then opens the adjacent category.
    pub fn assignment_key(
        &mut self,
        key: AssignmentKey,
        visible: &[(CategoryId, Money)],
    ) -> Result<Option<(CategoryId, Money)>, crate::domain::MoneyError> {
        let Some(edit) = self.edit.as_mut() else {
            return Ok(None);
        };
        if key == AssignmentKey::Escape {
            self.cancel_edit();
            return Ok(None);
        }
        let amount = parse_usd_input(&edit.draft)
            .inspect_err(|_| edit.error = Some("Enter a valid dollar amount".into()))?;
        let committed = (edit.category_id, amount);
        if key != AssignmentKey::Escape && !self.pending_assignments.insert(edit.category_id) {
            return Ok(None);
        }
        if key != AssignmentKey::Enter {
            let at = visible
                .iter()
                .position(|(id, _)| *id == edit.category_id)
                .unwrap_or(0);
            let next = match key {
                AssignmentKey::Tab | AssignmentKey::Down => {
                    (at + 1).min(visible.len().saturating_sub(1))
                }
                AssignmentKey::BackTab | AssignmentKey::Up => at.saturating_sub(1),
                _ => at,
            };
            if let Some((id, value)) = visible.get(next).copied() {
                self.begin_edit(id, value);
            }
        }
        Ok(Some(committed))
    }
    pub fn propose_auto_assign(
        &mut self,
        view: &BudgetMonthView,
    ) -> Result<(), crate::domain::MoneyError> {
        let inputs: Vec<_> = view
            .rows
            .iter()
            .filter(|r| self.selected_categories.contains(&r.category_id))
            .map(|r| AutoAssignInput {
                category_id: r.category_id,
                assigned: cents(r.assigned_cents),
                available: cents(r.available_cents),
                underfunded: cents(r.underfunded_cents),
                scheduled_this_month: Money::ZERO,
                assigned_history: vec![],
                spent_history: vec![],
            })
            .collect();
        self.auto_preview = Some(propose_auto_assign_at_revision(
            view.calculation_revision,
            view.month,
            cents(view.ready_to_assign_cents),
            &inputs,
            self.auto_strategy,
        )?);
        Ok(())
    }
    pub fn preview_move(
        &mut self,
        view: &BudgetMonthView,
        from: CategoryId,
        to: CategoryId,
        amount: Money,
    ) -> Result<(), crate::domain::MoneyError> {
        if from == to || amount <= Money::ZERO {
            return Err(crate::domain::MoneyError::Invalid);
        }
        let source = view
            .rows
            .iter()
            .find(|r| r.category_id == from)
            .ok_or(crate::domain::MoneyError::Invalid)?;
        let destination = view
            .rows
            .iter()
            .find(|r| r.category_id == to)
            .ok_or(crate::domain::MoneyError::Invalid)?;
        let from_after = cents(source.assigned_cents).checked_sub(amount)?;
        let to_after = cents(destination.assigned_cents).checked_add(amount)?;
        self.move_preview = Some(MoveMoneyPreview {
            from,
            to,
            amount,
            changes: [
                assignment_change(from, from_after),
                assignment_change(to, to_after),
            ],
        });
        Ok(())
    }
}

fn cents(value: i64) -> Money {
    Money::from_minor_units(value)
}
fn assignment_change(category_id: CategoryId, amount: Money) -> AssignmentBatchChange {
    if amount == Money::ZERO {
        AssignmentBatchChange::Remove { category_id }
    } else {
        AssignmentBatchChange::Set {
            category_id,
            amount,
        }
    }
}
fn submit_batch(
    actions: &mut ActionCollector,
    month: BudgetMonth,
    revision: u64,
    changes: Vec<AssignmentBatchChange>,
) {
    actions.push(ApplicationAction::Financial(FinancialCommand::Assignment(
        AssignmentCommand::Batch(AssignmentBatch {
            month,
            expected_source_revision: revision,
            changes,
        }),
    )));
}

fn submit_assignment(
    actions: &mut ActionCollector,
    month: BudgetMonth,
    category_id: CategoryId,
    amount: Money,
) {
    actions.push(ApplicationAction::Financial(FinancialCommand::Assignment(
        AssignmentCommand::Set(BudgetAssignment {
            category_id,
            month,
            amount,
        }),
    )));
}

pub fn show(
    ui: &mut egui::Ui,
    view: &BudgetMonthView,
    state: &mut BudgetUiState,
    context: crate::app::command::CommandAvailabilityContext,
    actions: &mut ActionCollector,
) {
    state.accept_projection(view);
    if view.rows.is_empty() {
        empty_budget(ui, actions);
        return;
    }
    render_header(ui, view, actions);
    render_assignment_tools(ui, view, state, context, actions);
    render_grid(ui, view, state, actions);
}

fn render_header(ui: &mut egui::Ui, view: &BudgetMonthView, actions: &mut ActionCollector) {
    ui.horizontal(|ui| {
        if ui.button("◀").on_hover_text("Previous month").clicked() {
            actions.push(crate::app::command::AppCommand::PreviousMonth);
        }
        ui.heading(format!(
            "Budget · {}-{:02}",
            view.month.year(),
            view.month.month()
        ));
        if ui.button("Today").clicked() {
            actions.push(crate::app::command::AppCommand::NavigateBudget);
        }
        if ui.button("▶").on_hover_text("Next month").clicked() {
            actions.push(crate::app::command::AppCommand::NextMonth);
        }
        ui.separator();
        let (color, icon, meaning) = ready_to_assign_semantics(view.ready_to_assign_cents);
        ui.colored_label(
            color,
            egui::RichText::new(format!(
                "{icon} Ready to Assign: {} ({meaning})",
                cents(view.ready_to_assign_cents)
            ))
            .strong()
            .size(18.0),
        );
    });
}

fn render_assignment_tools(
    ui: &mut egui::Ui,
    view: &BudgetMonthView,
    state: &mut BudgetUiState,
    context: crate::app::command::CommandAvailabilityContext,
    actions: &mut ActionCollector,
) {
    ui.horizontal(|ui| {
        let auto = crate::app::command::command_availability(
            context,
            crate::app::command::AppCommand::AutoAssign,
        );
        ui.add_enabled_ui(auto.enabled, |ui| {
            egui::ComboBox::from_label("Auto-Assign")
                .selected_text(format!("{:?}", state.auto_strategy))
                .show_ui(ui, |ui| {
                    for strategy in [
                        AutoAssignStrategy::Underfunded,
                        AutoAssignStrategy::ScheduledThisMonth,
                        AutoAssignStrategy::AssignedLastMonth,
                        AutoAssignStrategy::SpentLastMonth,
                        AutoAssignStrategy::ResetAssigned,
                        AutoAssignStrategy::ResetAvailable,
                    ] {
                        ui.selectable_value(
                            &mut state.auto_strategy,
                            strategy,
                            format!("{strategy:?}"),
                        );
                    }
                });
        });
        if crate::ui::widgets::action_button(
            ui,
            "Preview selected",
            crate::app::command::AppCommand::AutoAssign,
            context,
            actions,
        )
        .clicked()
        {
            let _ = state.propose_auto_assign(view);
        }
        let moving = crate::app::command::command_availability(
            context,
            crate::app::command::AppCommand::MoveMoney,
        );
        ui.add_enabled_ui(moving.enabled, |ui| {
            ui.menu_button("Move Money", |ui| {
                ui.label("Select categories, then preview Auto-Assign or Move Money.");
            });
        });
    });
    if let Some(preview) = state.auto_preview.clone() {
        ui.group(|ui| {
            ui.strong("Auto-Assign preview (no changes applied)");
            ui.label(format!(
                "Ready to Assign: {} → {}",
                cents(view.ready_to_assign_cents),
                preview.remaining_rta
            ));
            for warning in &preview.warnings {
                ui.colored_label(ui.visuals().warn_fg_color, warning);
            }
            if ui.button("Confirm atomic assignment batch").clicked() {
                submit_batch(
                    actions,
                    preview.month,
                    preview.source_revision,
                    preview
                        .changes
                        .iter()
                        .map(|c| assignment_change(c.category_id, c.after))
                        .collect(),
                );
            }
        });
    }
}

fn render_grid(
    ui: &mut egui::Ui,
    view: &BudgetMonthView,
    state: &mut BudgetUiState,
    actions: &mut ActionCollector,
) {
    let rows = ordered_rows(view);
    let visible: Vec<_> = rows
        .iter()
        .filter(|r| !state.is_collapsed(r.group_id))
        .map(|r| (r.category_id, cents(r.assigned_cents)))
        .collect();
    egui::Grid::new("budget-authoritative-grid")
        .striped(true)
        .show(ui, |ui| {
            for title in [
                "",
                "Category",
                "Assigned",
                "Activity",
                "Available",
                "Target status",
            ] {
                ui.strong(title);
            }
            ui.end_row();
            let mut previous_group = None;
            for row in rows {
                if previous_group != Some(row.group_id) {
                    previous_group = Some(row.group_id);
                    let collapsed = state.is_collapsed(row.group_id);
                    if ui.button(if collapsed { "▶" } else { "▼" }).clicked() {
                        state.set_collapsed(row.group_id, !collapsed);
                    }
                    ui.strong(&row.group_name);
                    ui.label("");
                    ui.label("");
                    ui.label("");
                    ui.label("");
                    ui.end_row();
                }
                if state.is_collapsed(row.group_id) {
                    continue;
                }
                let mut selected = state.selected_categories.contains(&row.category_id);
                if ui.checkbox(&mut selected, "").changed() {
                    state.toggle_selected(row.category_id);
                }
                let category_clicked = ui
                    .horizontal(|ui| {
                        ui.add_space(16.0);
                        ui.selectable_label(
                            state.focused_category == Some(row.category_id),
                            &row.name,
                        )
                        .clicked()
                    })
                    .inner;
                if category_clicked {
                    state.focused_category = Some(row.category_id);
                }
                assignment_cell(ui, row, view, state, &visible, actions);
                ui.label(crate::ui::format::money(cents(row.activity_cents)))
                    .on_hover_text("Activity is calculated from the budget projection");
                let (color, label) = availability(row);
                ui.colored_label(color, label)
                    .on_hover_text("Available is calculated from the budget projection");
                if row.underfunded_cents > 0 {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        format!("Underfunded · {} needed", cents(row.underfunded_cents)),
                    );
                } else {
                    ui.label(if row.target_id.is_some() {
                        &row.target_status
                    } else {
                        "No target"
                    });
                }
                ui.end_row();
            }
        });
}

fn ordered_rows(view: &BudgetMonthView) -> Vec<&CategoryRowView> {
    let mut rows: Vec<_> = view.rows.iter().collect();
    rows.sort_by_key(|row| (row.group_sort, row.category_sort));
    rows
}

fn assignment_cell(
    ui: &mut egui::Ui,
    row: &CategoryRowView,
    view: &BudgetMonthView,
    state: &mut BudgetUiState,
    visible: &[(CategoryId, Money)],
    actions: &mut ActionCollector,
) {
    let editing = state
        .edit
        .as_ref()
        .is_some_and(|e| e.category_id == row.category_id);
    if !editing {
        if ui
            .button(crate::ui::format::money(cents(row.assigned_cents)))
            .clicked()
        {
            state.begin_edit(row.category_id, cents(row.assigned_cents));
        }
        return;
    }
    let response = ui.add(
        egui::TextEdit::singleline(state.edit_text_mut().expect("active edit")).desired_width(90.0),
    );
    response.request_focus();
    let key = ui.input(|i| {
        if i.key_pressed(egui::Key::Escape) {
            Some(AssignmentKey::Escape)
        } else if i.key_pressed(egui::Key::Enter) {
            Some(AssignmentKey::Enter)
        } else if i.key_pressed(egui::Key::Tab) {
            Some(if i.modifiers.shift {
                AssignmentKey::BackTab
            } else {
                AssignmentKey::Tab
            })
        } else if i.key_pressed(egui::Key::ArrowUp) {
            Some(AssignmentKey::Up)
        } else if i.key_pressed(egui::Key::ArrowDown) {
            Some(AssignmentKey::Down)
        } else {
            None
        }
    });
    if let Some(key) = key {
        if let Ok(Some((category_id, amount))) = state.assignment_key(key, visible) {
            submit_assignment(actions, view.month, category_id, amount);
        }
    }
    if let Some(error) = state.edit_error() {
        ui.colored_label(egui::Color32::RED, error);
    }
}

fn availability(row: &CategoryRowView) -> (egui::Color32, String) {
    if row.overspending_cents < 0 {
        return if row.credit_card_payment {
            (
                egui::Color32::from_rgb(205, 125, 25),
                format!("⚠ {} · credit overspent", cents(row.available_cents)),
            )
        } else {
            (
                egui::Color32::from_rgb(190, 45, 45),
                format!("⛔ {} · cash overspent", cents(row.available_cents)),
            )
        };
    }
    if row.underfunded_cents > 0 {
        return (
            egui::Color32::from_rgb(205, 125, 25),
            format!("△ {} · underfunded", cents(row.available_cents)),
        );
    }
    if row.available_cents == 0 {
        return (egui::Color32::GRAY, "○ $0.00 · zero".into());
    }
    if row.available_cents > 0 {
        return (
            egui::Color32::from_rgb(35, 130, 70),
            format!("✓ {} · available", cents(row.available_cents)),
        );
    }
    (
        egui::Color32::from_rgb(205, 125, 25),
        format!("△ {} · underfunded", cents(row.available_cents)),
    )
}

fn ready_to_assign_semantics(cents: i64) -> (egui::Color32, &'static str, &'static str) {
    match cents.cmp(&0) {
        std::cmp::Ordering::Greater => (
            egui::Color32::from_rgb(35, 130, 70),
            "✓",
            "available to assign",
        ),
        std::cmp::Ordering::Equal => (egui::Color32::GRAY, "○", "fully assigned"),
        std::cmp::Ordering::Less => (egui::Color32::from_rgb(190, 45, 45), "⛔", "over-assigned"),
    }
}

pub fn empty(ui: &mut egui::Ui) {
    ui.label("Open or create a budget to begin.");
}

fn empty_budget(ui: &mut egui::Ui, actions: &mut ActionCollector) {
    ui.vertical_centered(|ui| {
        ui.heading("Build your budget");
        ui.label("Add a category group, then organize categories in the Categories workspace.");
        if ui.button("Add Category Group").clicked() {
            actions.push(ApplicationAction::Category(
                crate::app::command::CategoryAction::NewGroup,
            ));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{command::ApplicationAction, view_model::ViewVersion};

    fn row(
        group_id: CategoryGroupId,
        category_id: CategoryId,
        group_sort: i64,
        category_sort: i64,
    ) -> CategoryRowView {
        CategoryRowView {
            group_id,
            category_id,
            group_name: format!("group {group_sort}"),
            name: format!("category {category_sort}"),
            group_sort,
            category_sort,
            group_collapsed: false,
            assigned_cents: 0,
            activity_cents: 0,
            available_cents: 0,
            overspending_cents: 0,
            underfunded_cents: 0,
            target_id: None,
            target_amount_cents: None,
            target_remaining_cents: None,
            target_due_date: None,
            target_status: String::new(),
            credit_card_payment: false,
            protected: false,
            hidden: false,
            archived: false,
            inspector: String::new(),
        }
    }

    fn view(revision: u64, rows: Vec<CategoryRowView>) -> BudgetMonthView {
        BudgetMonthView {
            version: ViewVersion {
                generation: 1,
                revision,
            },
            month: BudgetMonth::new(2026, 8).unwrap(),
            calculation_revision: revision,
            ready_to_assign_cents: 1234,
            assigned_cents: 0,
            activity_cents: 0,
            available_cents: 0,
            overspending_cents: 0,
            cash_overspending_cents: 0,
            credit_card_overspending_cents: 0,
            rows,
            inspector: vec![],
        }
    }
    #[test]
    fn keyboard_commit_cancel_and_navigation_preserve_draft_on_failure() {
        let a = CategoryId::new();
        let b = CategoryId::new();
        let rows = [(a, cents(100)), (b, cents(200))];
        let mut s = BudgetUiState::default();
        s.begin_edit(a, cents(100));
        *s.edit_text_mut().unwrap() = "$9.00".into();
        assert_eq!(
            s.assignment_key(AssignmentKey::Tab, &rows).unwrap(),
            Some((a, cents(900)))
        );
        assert_eq!(s.focused_category, Some(b));
        *s.edit_text_mut().unwrap() = "bad".into();
        assert!(s.assignment_key(AssignmentKey::Enter, &rows).is_err());
        assert_eq!(s.edit_text_mut().unwrap(), "bad");
        s.report_commit_failure("disk full");
        assert_eq!(s.edit_error(), Some("disk full"));
        assert_eq!(s.cancel_edit().as_deref(), Some("$2.00"));
    }
    #[test]
    fn collapse_and_selection_use_stable_ids() {
        let g = CategoryGroupId::new();
        let c = CategoryId::new();
        let mut s = BudgetUiState::default();
        s.set_collapsed(g, true);
        s.toggle_selected(c);
        assert!(s.is_collapsed(g));
        assert!(s.selected_categories.contains(&c));
    }

    #[test]
    fn projection_order_and_collapse_survive_refresh() {
        let first_group = CategoryGroupId::new();
        let second_group = CategoryGroupId::new();
        let a = CategoryId::new();
        let b = CategoryId::new();
        let projection = view(
            1,
            vec![row(second_group, b, 20, 0), row(first_group, a, 10, 0)],
        );
        assert_eq!(
            ordered_rows(&projection)
                .iter()
                .map(|row| row.category_id)
                .collect::<Vec<_>>(),
            vec![a, b]
        );

        let mut state = BudgetUiState::default();
        state.set_collapsed(first_group, true);
        state.accept_projection(&projection);
        let refreshed = view(2, projection.rows.clone());
        state.accept_projection(&refreshed);
        assert!(state.is_collapsed(first_group));
    }

    #[test]
    fn focus_is_stable_while_collapsed_and_clears_only_when_removed() {
        let group = CategoryGroupId::new();
        let category = CategoryId::new();
        let mut state = BudgetUiState {
            focused_category: Some(category),
            ..BudgetUiState::default()
        };
        state.set_collapsed(group, true);
        state.accept_projection(&view(1, vec![row(group, category, 0, 0)]));
        assert_eq!(state.focused_category, Some(category));
        state.accept_projection(&view(2, vec![]));
        assert_eq!(state.focused_category, None);
    }

    #[test]
    fn assignment_submission_is_set_and_gated_until_a_new_projection() {
        let category = CategoryId::new();
        let month = BudgetMonth::new(2026, 8).unwrap();
        let mut actions = ActionCollector::default();
        submit_assignment(&mut actions, month, category, cents(725));
        assert_eq!(
            actions.into_actions(),
            vec![ApplicationAction::Financial(FinancialCommand::Assignment(
                AssignmentCommand::Set(BudgetAssignment {
                    category_id: category,
                    month,
                    amount: cents(725)
                })
            ))]
        );

        let rows = [(category, Money::ZERO)];
        let mut state = BudgetUiState::default();
        state.begin_edit(category, Money::ZERO);
        *state.edit_text_mut().unwrap() = "1.00".into();
        assert!(
            state
                .assignment_key(AssignmentKey::Enter, &rows)
                .unwrap()
                .is_some()
        );
        assert!(
            state
                .assignment_key(AssignmentKey::Enter, &rows)
                .unwrap()
                .is_none()
        );
        state.accept_projection(&view(2, vec![row(CategoryGroupId::new(), category, 0, 0)]));
        assert!(
            state
                .assignment_key(AssignmentKey::Enter, &rows)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn ready_to_assign_semantics_use_the_projection_value_sign() {
        assert_eq!(ready_to_assign_semantics(1).1, "✓");
        assert_eq!(ready_to_assign_semantics(0).1, "○");
        assert_eq!(ready_to_assign_semantics(-1).1, "⛔");
    }
}
