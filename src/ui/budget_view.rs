//! Budget table and assignment-entry widgets.
use crate::{
    app::command::AppCommand,
    calculation::budget_month::{BudgetMonthResult, FundingStatus},
    domain::{BudgetMonth, CategoryGroupId, CategoryId, Money},
};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetRow {
    pub category_id: CategoryId,
    pub group: String,
    pub category: String,
    pub assigned: Money,
    pub activity: Money,
    pub available: Money,
    pub funding: FundingStatus,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BudgetViewAction {
    ReplaceAssignment {
        category_id: CategoryId,
        amount: Money,
    },
    NavigateTo(BudgetMonth),
    AddGroup {
        name: String,
    },
    RenameGroup {
        group_id: CategoryGroupId,
        name: String,
    },
    ReorderGroup {
        group_id: CategoryGroupId,
        before: Option<CategoryGroupId>,
    },
    AddCategory {
        group_id: CategoryGroupId,
        name: String,
    },
    RenameCategory {
        category_id: CategoryId,
        name: String,
    },
    MoveCategory {
        category_id: CategoryId,
        group_id: CategoryGroupId,
        before: Option<CategoryId>,
    },
    SetCategoryHidden {
        category_id: CategoryId,
        hidden: bool,
    },
    ArchiveCategory {
        category_id: CategoryId,
    },
    DeleteCategoryIfUnused {
        category_id: CategoryId,
    },
}

/// Ephemeral presentation preferences; intentionally absent from repositories.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BudgetUiState {
    collapsed_groups: BTreeSet<CategoryGroupId>,
    pub focused_category: Option<CategoryId>,
}
impl BudgetUiState {
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
            self.focused_category = visible.first().copied();
        }
    }
}

pub fn show(
    ui: &mut egui::Ui,
    month: BudgetMonth,
    rows: &[BudgetRow],
    result: &BudgetMonthResult,
    commands: &mut Vec<AppCommand>,
    actions: &mut Vec<BudgetViewAction>,
) {
    ui.horizontal(|ui| {
        if ui.button("◀").on_hover_text("Previous month").clicked() {
            commands.push(AppCommand::PreviousMonth);
        }
        ui.heading(format!("{}-{:02}", month.year(), month.month()));
        if ui.button("▶").on_hover_text("Next month").clicked() {
            commands.push(AppCommand::NextMonth);
        }
    });
    ui.label(format!("Ready to Assign: {}", result.ready_to_assign));
    egui::Grid::new("budget-grid").striped(true).show(ui, |ui| {
        for title in ["Category", "Assigned", "Activity", "Available", "Funding"] {
            ui.strong(title);
        }
        ui.end_row();
        for row in rows {
            ui.label(format!("{} / {}", row.group, row.category));
            let mut text = row.assigned.minor_units().to_string();
            let response = ui.add(egui::TextEdit::singleline(&mut text).desired_width(90.0));
            if response.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && let Ok(cents) = text.parse::<i64>()
            {
                actions.push(BudgetViewAction::ReplaceAssignment {
                    category_id: row.category_id,
                    amount: Money::from_minor_units(cents),
                });
            }
            ui.label(row.activity.to_string());
            ui.label(row.available.to_string());
            ui.label(match row.funding {
                FundingStatus::NoTarget => "—".into(),
                FundingStatus::Funded => "Funded".into(),
                FundingStatus::Underfunded(v) => format!("{v} needed"),
            });
            ui.end_row();
        }
    });
}

pub fn empty(ui: &mut egui::Ui) {
    ui.label("Open or create a budget to begin.");
}

#[cfg(test)]
mod ui_state_tests {
    use super::*;
    #[test]
    fn stable_id_focus_restores_without_indices() {
        let removed = CategoryId::new();
        let remaining = CategoryId::new();
        let mut state = BudgetUiState {
            focused_category: Some(removed),
            ..Default::default()
        };
        state.restore_focus(&[remaining]);
        assert_eq!(state.focused_category, Some(remaining));
    }

    #[test]
    fn semantic_assignment_action_carries_entity_id() {
        let id = CategoryId::new();
        let action = BudgetViewAction::ReplaceAssignment {
            category_id: id,
            amount: Money::from_minor_units(25),
        };
        assert!(
            matches!(action, BudgetViewAction::ReplaceAssignment { category_id, .. } if category_id == id)
        );
    }
}

/// Used by date pickers; callers commit navigation centrally with this typed action.
pub fn navigate_to(month: BudgetMonth) -> BudgetViewAction {
    BudgetViewAction::NavigateTo(month)
}
