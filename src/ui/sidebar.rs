use crate::{
    app::{
        command::AppCommand,
        dispatcher::ActionCollector,
        navigation::Workspace,
        state::{AccountSummary, AppState},
    },
    domain::{AccountGroup, AccountGroupId, AccountId, Money},
};

fn account_row(ui: &mut egui::Ui, account: &AccountSummary, selected: bool) -> egui::Response {
    ui.horizontal(|ui| {
        let label = format!(
            "{}{}{}",
            if account.favorite { "★ " } else { "" },
            account.name,
            if account.unreconciled { "  ⚠" } else { "" },
        );
        let response = ui.selectable_label(selected, label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            crate::ui::format::money_cell(ui, account.working_balance);
        });
        response
    })
    .inner
}

/// Stable traversal order used by arrow-key navigation. Collapsed descendants are omitted, but
/// accounts (including zero-balance accounts) are never filtered by balance.
#[must_use]
pub fn traversal(groups: &[AccountGroup], accounts: &[AccountSummary]) -> Vec<AccountId> {
    fn visit(
        parent: Option<AccountGroupId>,
        groups: &[AccountGroup],
        accounts: &[AccountSummary],
        out: &mut Vec<AccountId>,
    ) {
        let mut children: Vec<_> = groups
            .iter()
            .filter(|g| g.parent_group_id == parent)
            .collect();
        children.sort_by_key(|g| (g.sort_order, &g.name, g.id));
        let mut direct: Vec<_> = accounts.iter().filter(|a| a.group_id == parent).collect();
        direct.sort_by_key(|a| (&a.name, a.id));
        out.extend(direct.into_iter().map(|a| a.id));
        for group in children {
            if !group.collapsed {
                visit(Some(group.id), groups, accounts, out);
            }
        }
    }
    let mut out = vec![];
    visit(None, groups, accounts, &mut out);
    out
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountSection {
    OnBudget,
    Tracking,
    Closed,
}

#[must_use]
pub fn account_section(account: &AccountSummary) -> AccountSection {
    if account.closed {
        AccountSection::Closed
    } else if account.tracking {
        AccountSection::Tracking
    } else {
        AccountSection::OnBudget
    }
}
#[must_use]
pub fn navigation_items(inbox_count: usize) -> Vec<(String, Workspace)> {
    vec![
        ("Budget".into(), Workspace::Budget),
        ("Reports".into(), Workspace::Reports),
        ("All Transactions".into(), Workspace::AllTransactions),
        (format!("Inbox ({inbox_count})"), Workspace::Inbox),
        ("Manage Categories".into(), Workspace::Categories),
    ]
}

/// Deterministic ordering prevents refreshes from moving keyboard targets under the user.
#[must_use]
pub fn ordered_accounts(
    accounts: &[AccountSummary],
    section: AccountSection,
) -> Vec<&AccountSummary> {
    let mut result: Vec<_> = accounts
        .iter()
        .filter(|account| account_section(account) == section)
        .collect();
    result.sort_by_key(|account| {
        (
            !account.favorite,
            account.name.to_ascii_lowercase(),
            account.id,
        )
    });
    result
}

#[must_use]
pub fn section_balance(accounts: &[&AccountSummary]) -> Option<Money> {
    accounts
        .iter()
        .try_fold(Money::ZERO, |sum, account| {
            sum.checked_add(account.working_balance)
        })
        .ok()
}

pub fn show(ui: &mut egui::Ui, state: &mut AppState, actions: &mut ActionCollector) {
    for (index, (label, workspace)) in navigation_items(state.inbox_counts.total)
        .into_iter()
        .enumerate()
    {
        if index == 4 {
            ui.add_space(4.0);
            ui.weak("ADMINISTRATION");
        }
        if ui
            .selectable_label(state.navigation.workspace == workspace, label)
            .clicked()
        {
            state.navigation.workspace = workspace;
        }
    }
    ui.separator();
    for (title, section) in [
        ("BUDGET ACCOUNTS", AccountSection::OnBudget),
        ("TRACKING", AccountSection::Tracking),
        ("CLOSED", AccountSection::Closed),
    ] {
        let accounts = ordered_accounts(&state.accounts, section);
        if accounts.is_empty() {
            continue;
        }
        ui.horizontal(|ui| {
            ui.strong(title);
            if let Some(total) = section_balance(&accounts) {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    crate::ui::format::money_cell(ui, total);
                });
            }
        });
        for account in accounts {
            if account_row(
                ui,
                account,
                state.navigation.workspace == Workspace::Account(account.id),
            )
            .clicked()
            {
                state.selected_account = Some(account.id);
                state.navigation.workspace = Workspace::Account(account.id);
            }
        }
        ui.add_space(6.0);
    }
    ui.separator();
    if state.accounts.is_empty() {
        let model = crate::ui::empty_state::model(
            crate::ui::empty_state::EmptyState::NoAccounts,
            state.active_budget.is_some(),
            false,
        );
        crate::ui::empty_state::show(ui, &model, actions);
    } else {
        if ui.button("＋ Add Account").clicked() {
            actions.push(AppCommand::AddAccount);
        }
        if ui.small_button("Add account group").clicked() {
            actions.push(AppCommand::AddAccountGroup);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AccountId;
    #[test]
    fn closed_always_appears_in_closed_section() {
        let account = AccountSummary {
            id: AccountId::new(),
            name: "Old".into(),
            working_balance: Money::ZERO,
            unreconciled: false,
            tracking: true,
            closed: true,
            group_id: None,
            favorite: false,
            cleared_balance: Money::ZERO,
            account_type: crate::domain::AccountType::Asset,
        };
        assert_eq!(account_section(&account), AccountSection::Closed);
    }

    #[test]
    fn collapsed_groups_persist_and_traversal_keeps_zero_balances() {
        let budget = crate::domain::BudgetId::new();
        let mut g = AccountGroup::new(budget, "Banks");
        g.collapsed = true;
        let account = AccountSummary {
            id: AccountId::new(),
            name: "Zero".into(),
            working_balance: Money::ZERO,
            unreconciled: false,
            tracking: false,
            closed: false,
            group_id: Some(g.id),
            favorite: false,
            cleared_balance: Money::ZERO,
            account_type: crate::domain::AccountType::Checking,
        };
        assert!(traversal(&[g.clone()], &[account.clone()]).is_empty());
        let mut open = g;
        open.collapsed = false;
        let account_id = account.id;
        assert_eq!(traversal(&[open], &[account]), vec![account_id]);
    }

    #[test]
    fn every_sidebar_item_has_a_workspace_destination() {
        let items = navigation_items(7);
        assert_eq!(items.len(), 5);
        assert!(
            items
                .iter()
                .any(|(label, route)| label == "Inbox (7)" && *route == Workspace::Inbox)
        );
        assert!(items.iter().all(|(label, _)| !label.is_empty()));
        assert_eq!(items[0].1, Workspace::Budget);
        assert_eq!(items[1].1, Workspace::Reports);
        assert_eq!(items[2].1, Workspace::AllTransactions);
        assert_eq!(items[4].1, Workspace::Categories);
    }

    #[test]
    fn classification_and_ordering_are_stable() {
        let make = |name: &str, tracking, closed, favorite| AccountSummary {
            id: AccountId::new(),
            name: name.into(),
            working_balance: Money::ZERO,
            unreconciled: false,
            tracking,
            closed,
            group_id: None,
            favorite,
            cleared_balance: Money::ZERO,
            account_type: crate::domain::AccountType::Checking,
        };
        let accounts = vec![
            make("zebra", false, false, false),
            make("Alpha", false, false, false),
            make("Favorite", false, false, true),
            make("Tracker", true, false, false),
            make("Closed", false, true, false),
        ];
        assert_eq!(account_section(&accounts[3]), AccountSection::Tracking);
        assert_eq!(account_section(&accounts[4]), AccountSection::Closed);
        let names: Vec<_> = ordered_accounts(&accounts, AccountSection::OnBudget)
            .into_iter()
            .map(|account| account.name.as_str())
            .collect();
        assert_eq!(names, ["Favorite", "Alpha", "zebra"]);
    }
}
