//! Budget table and assignment-entry widgets.
use crate::{
    app::command::AppCommand,
    calculation::budget_month::{BudgetMonthResult, FundingStatus},
    domain::{BudgetMonth, CategoryGroupId, CategoryId, Money},
};
use std::collections::BTreeSet;

/// Parse an editor value as formatted USD. Blank means reset to zero; all other rules are
/// delegated to `Money`, keeping the widget boundary in dollars and the domain in integer cents.
pub fn parse_usd_input(text: &str) -> Result<Money, crate::domain::MoneyError> {
    if text.trim().is_empty() {
        Ok(Money::ZERO)
    } else {
        text.parse()
    }
}

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
    AddAssignment {
        category_id: CategoryId,
        amount: Money,
    },
    ResetAssignment {
        category_id: CategoryId,
    },
    MoveAssignment {
        from: CategoryId,
        to: CategoryId,
        amount: Money,
    },
    AssignAllReady {
        category_id: CategoryId,
    },
    DistributeSelected {
        category_ids: Vec<CategoryId>,
        amount: Money,
    },
    PreviewAutoAssign,
    ApplyApprovedAutoAssign,
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
    pub selected_categories: BTreeSet<CategoryId>,
    edit: Option<AssignmentEdit>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AssignmentEdit {
    category_id: CategoryId,
    original: String,
    value: String,
}
impl BudgetUiState {
    pub fn begin_edit(&mut self, id: CategoryId, value: Money) {
        let value = value.to_string();
        self.edit = Some(AssignmentEdit {
            category_id: id,
            original: value.clone(),
            value,
        });
        self.focused_category = Some(id);
    }
    pub fn edit_text_mut(&mut self) -> Option<&mut String> {
        self.edit.as_mut().map(|e| &mut e.value)
    }
    /// Escape rolls the editor back exactly; it never emits a financial mutation.
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
            let mut text = row.assigned.to_string();
            let response = ui.add(egui::TextEdit::singleline(&mut text).desired_width(90.0));
            if response.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && let Ok(amount) = parse_usd_input(&text)
            {
                actions.push(BudgetViewAction::ReplaceAssignment {
                    category_id: row.category_id,
                    amount,
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

    #[test]
    fn formatted_input_and_escape_restoration() {
        assert_eq!(
            parse_usd_input(" ($1,234.50) ").unwrap().minor_units(),
            -123_450
        );
        assert_eq!(parse_usd_input(" ").unwrap(), Money::ZERO);
        assert!(parse_usd_input("1.234").is_err());
        let id = CategoryId::new();
        let mut state = BudgetUiState::default();
        state.begin_edit(id, Money::from_minor_units(1234));
        *state.edit_text_mut().unwrap() = "$99.00".into();
        assert_eq!(state.cancel_edit().as_deref(), Some("$12.34"));
    }
}

/// Used by date pickers; callers commit navigation centrally with this typed action.
pub fn navigate_to(month: BudgetMonth) -> BudgetViewAction {
    BudgetViewAction::NavigateTo(month)
}
