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
    let warning = if account.unreconciled { " ⚠" } else { "" };
    let kind = if account.tracking { " · tracking" } else { "" };
    let closed = if account.closed { " · closed" } else { "" };
    ui.selectable_label(
        selected,
        format!(
            "{}{}{}{}{}   {}",
            if account.favorite { "★ " } else { "" },
            account.name,
            warning,
            kind,
            closed,
            account.working_balance
        ),
    )
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

fn group(
    ui: &mut egui::Ui,
    id: AccountGroupId,
    depth: usize,
    state: &mut AppState,
) -> Option<AccountId> {
    let Some(index) = state.account_groups.iter().position(|g| g.id == id) else {
        return None;
    };
    let (name, collapsed) = {
        let g = &state.account_groups[index];
        (g.name.clone(), g.collapsed)
    };
    let response = ui
        .horizontal(|ui| {
            ui.add_space(depth as f32 * 12.0);
            if ui.small_button(if collapsed { "▶" } else { "▼" }).clicked() {
                state.account_groups[index].collapsed = !collapsed;
            }
            ui.strong(name);
        })
        .response;
    response.context_menu(|ui| {
        ui.label("Rename / move group");
        ui.label("Use Move Up / Move Down for keyboard reorder");
    });
    if collapsed {
        return None;
    }
    let mut selected = None;
    let account_ids: Vec<_> = state
        .accounts
        .iter()
        .filter(|a| a.group_id == Some(id))
        .map(|a| a.id)
        .collect();
    for account_id in account_ids {
        let account = state.accounts.iter().find(|a| a.id == account_id).unwrap();
        let r = ui
            .horizontal(|ui| {
                ui.add_space((depth + 1) as f32 * 12.0);
                account_row(ui, account, state.selected_account == Some(account.id))
            })
            .inner;
        r.context_menu(|ui| {
            for label in [
                "Rename",
                "Move…",
                if account.closed { "Reopen" } else { "Close…" },
                if account.favorite {
                    "Remove favorite"
                } else {
                    "Favorite"
                },
                "Delete if unused…",
            ] {
                ui.label(label);
            }
        });
        if r.clicked() {
            selected = Some(account_id);
        }
    }
    let mut children: Vec<_> = state
        .account_groups
        .iter()
        .filter(|g| g.parent_group_id == Some(id))
        .map(|g| g.id)
        .collect();
    children.sort_by_key(|child| {
        state
            .account_groups
            .iter()
            .find(|g| g.id == *child)
            .map(|g| g.sort_order)
            .unwrap_or(0)
    });
    for child in children {
        selected = group(ui, child, depth + 1, state).or(selected);
    }
    selected
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
pub fn show(ui: &mut egui::Ui, state: &mut AppState, actions: &mut ActionCollector) {
    ui.heading("MNAB");
    ui.label(&state.budget_name);
    ui.small(
        state
            .database_path
            .as_ref()
            .map_or_else(|| "No database".into(), |p| p.display().to_string()),
    );
    ui.separator();
    for (label, workspace) in [
        ("Overview", Workspace::Overview),
        ("Budget", Workspace::Budget),
        ("Categories", Workspace::Categories),
        ("Reports", Workspace::Reports),
        ("All Transactions", Workspace::AllTransactions),
        ("Inbox", Workspace::Inbox),
    ] {
        if ui
            .selectable_label(state.navigation.workspace == workspace, label)
            .clicked()
        {
            state.navigation.workspace = workspace;
        }
    }
    ui.separator();
    let mut roots: Vec<_> = state
        .account_groups
        .iter()
        .filter(|g| g.parent_group_id.is_none())
        .map(|g| g.id)
        .collect();
    roots.sort_by_key(|id| {
        state
            .account_groups
            .iter()
            .find(|g| g.id == *id)
            .map(|g| g.sort_order)
            .unwrap_or(0)
    });
    for root in roots {
        if let Some(id) = group(ui, root, 0, state) {
            state.selected_account = Some(id);
            state.navigation.workspace = Workspace::Account(id);
        }
    }
    ui.strong("UNGROUPED");
    for account in state.accounts.iter().filter(|a| a.group_id.is_none()) {
        if account_row(ui, account, state.selected_account == Some(account.id)).clicked() {
            state.selected_account = Some(account.id);
            state.navigation.workspace = Workspace::Account(account.id);
        }
    }
    if ui.button("＋ Add Account").clicked() {
        actions.push(AppCommand::AddAccount);
    }
    if ui.button("＋ Add Group").clicked() {
        actions.push(AppCommand::AddAccountGroup);
    }
}
#[allow(dead_code)]
fn _money(_: Money) {}

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
}
