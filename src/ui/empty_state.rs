//! Consistent, action-oriented empty-state models.
use crate::app::command::{AppCommand, ApplicationAction, CategoryAction};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmptyState {
    NoAccounts,
    EmptyRegister,
    BudgetWithoutCategories,
    ReportsWithoutData,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Model {
    pub title: &'static str,
    pub guidance: &'static str,
    /// Only commands which can actually be handled in the current context are exposed.
    pub actions: Vec<(&'static str, ApplicationAction)>,
}

#[must_use]
pub fn model(state: EmptyState, database_available: bool, account_available: bool) -> Model {
    match state {
        EmptyState::NoAccounts => Model {
            title: "No accounts yet",
            guidance: "Add an account to start tracking transactions.",
            actions: database_available
                .then_some(("Add Account", AppCommand::AddAccount.into()))
                .into_iter()
                .collect(),
        },
        EmptyState::EmptyRegister => Model {
            title: "No transactions yet",
            guidance: "Add a transaction or import a statement to get started.",
            actions: if account_available {
                vec![
                    ("Add Transaction", AppCommand::AddTransaction.into()),
                    ("Import", AppCommand::Import.into()),
                ]
            } else {
                vec![]
            },
        },
        EmptyState::BudgetWithoutCategories => Model {
            title: "Build your budget",
            guidance: "Add a category group, then organize categories in Categories.",
            actions: database_available
                .then_some((
                    "Add Category Group",
                    ApplicationAction::Category(CategoryAction::NewGroup),
                ))
                .into_iter()
                .collect(),
        },
        EmptyState::ReportsWithoutData => Model {
            title: "No report data",
            guidance: "Add transactions or adjust the report filters, then refresh.",
            actions: vec![],
        },
    }
}

pub fn show(
    ui: &mut egui::Ui,
    model: &Model,
    actions: &mut crate::app::dispatcher::ActionCollector,
) {
    ui.group(|ui| {
        ui.strong(model.title);
        ui.label(model.guidance);
        ui.horizontal(|ui| {
            for (label, action) in &model.actions {
                if ui.button(*label).clicked() {
                    actions.push(action.clone());
                }
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actions_are_hidden_when_unavailable() {
        assert!(
            model(EmptyState::NoAccounts, false, false)
                .actions
                .is_empty()
        );
        assert!(
            model(EmptyState::EmptyRegister, true, false)
                .actions
                .is_empty()
        );
        assert_eq!(
            model(EmptyState::EmptyRegister, true, true).actions.len(),
            2
        );
        assert!(
            model(EmptyState::ReportsWithoutData, true, true)
                .actions
                .is_empty()
        );
    }
}
