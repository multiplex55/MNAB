use crate::{
    app::{
        command::AppCommand, dispatcher::ActionCollector, navigation::Workspace, state::AppState,
    },
    domain::Money,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverviewDestination {
    Workspace(Workspace),
    Action(AppCommand),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverviewCard {
    pub title: String,
    pub value: String,
    pub detail: String,
    pub destination: OverviewDestination,
}

/// Immutable dashboard projection assembled only from accepted application projections.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OverviewProjection {
    pub cards: Vec<OverviewCard>,
}

impl OverviewProjection {
    #[must_use]
    pub fn from_state(state: &AppState) -> Self {
        if state.active_budget.is_none() {
            return Self::default();
        }
        let mut cards = Vec::new();
        if let Some(month) = state.budget_month.last_successful.as_ref() {
            cards.push(OverviewCard {
                title: "Ready to Assign".into(),
                value: crate::ui::format::money(Money::from_minor_units(
                    month.ready_to_assign_cents,
                )),
                detail: "Plan this month's available money".into(),
                destination: OverviewDestination::Workspace(Workspace::Budget),
            });
        }
        cards.push(OverviewCard {
            title: "Needs attention".into(),
            value: state.inbox_counts.total.to_string(),
            detail: "Review imports, approvals, targets, and exceptions".into(),
            destination: OverviewDestination::Workspace(Workspace::Inbox),
        });
        let open: Vec<_> = state
            .accounts
            .iter()
            .filter(|account| !account.closed)
            .collect();
        if !open.is_empty() {
            let total = open
                .iter()
                .try_fold(Money::ZERO, |sum, account| {
                    sum.checked_add(account.working_balance)
                })
                .unwrap_or(Money::ZERO);
            cards.push(OverviewCard {
                title: "Account balances".into(),
                value: crate::ui::format::money(total),
                detail: format!("Across {} open accounts", open.len()),
                destination: OverviewDestination::Workspace(Workspace::AllTransactions),
            });
        }
        for (title, detail, command) in [
            (
                "New transaction",
                "Record spending or income",
                AppCommand::AddTransaction,
            ),
            ("Import", "Bring in a bank statement", AppCommand::Import),
            (
                "Transfer",
                "Move money between accounts",
                AppCommand::CreateTransfer,
            ),
        ] {
            let availability =
                crate::app::command::command_availability(state.action_context(), command);
            // A dashboard is not a graveyard for unfinished flows: unavailable destinations are hidden.
            if availability.enabled {
                cards.push(OverviewCard {
                    title: title.into(),
                    value: "Quick action".into(),
                    detail: detail.into(),
                    destination: OverviewDestination::Action(command),
                });
            }
        }
        Self { cards }
    }
}

pub fn activate(
    destination: OverviewDestination,
    state: &mut AppState,
    actions: &mut ActionCollector,
) {
    match destination {
        OverviewDestination::Workspace(workspace) => state.navigation.workspace = workspace,
        OverviewDestination::Action(command) => actions.push(command),
    }
}

pub fn show(ui: &mut egui::Ui, state: &mut AppState, actions: &mut ActionCollector) {
    ui.heading("Overview");
    ui.label(format!("{} at a glance", state.budget_name));
    let projection = OverviewProjection::from_state(state);
    let mut chosen = None;
    ui.horizontal_wrapped(|ui| {
        for card in &projection.cards {
            let response = ui
                .group(|ui| {
                    ui.set_min_width(190.0);
                    ui.strong(&card.title);
                    ui.heading(&card.value);
                    ui.label(&card.detail);
                })
                .response
                .interact(egui::Sense::click());
            if response.clicked() {
                chosen = Some(card.destination);
            }
        }
    });
    if projection.cards.is_empty() {
        ui.label("Open a budget to see its overview.");
    }
    if let Some(destination) = chosen {
        activate(destination, state, actions);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_overview_card_has_a_reachable_destination() {
        let budget = crate::domain::BudgetId::new();
        let mut state = AppState::default();
        state.active_budget = Some(budget);
        state.inbox_counts.total = 2;
        let projection = OverviewProjection::from_state(&state);
        assert!(!projection.cards.is_empty());
        for card in projection.cards {
            let mut actions = ActionCollector::default();
            activate(card.destination, &mut state, &mut actions);
            assert!(
                matches!(card.destination, OverviewDestination::Workspace(_))
                    || !actions.into_actions().is_empty()
            );
        }
    }
}
