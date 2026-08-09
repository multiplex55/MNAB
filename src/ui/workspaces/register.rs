//! Account-register orchestration. Rendering details live in `ui::register`.
use crate::{
    app::{dispatcher::ActionCollector, state::AppState},
    domain::AccountId,
};

#[allow(unused_imports)]
pub use crate::ui::register::{
    ACCOUNT_COLUMNS, ALL_TRANSACTION_COLUMNS, RegisterColumn, RegisterFilter, RegisterRow,
    RegisterState, SortDirection, TransferEditor, TransferSummary, editor_from_row,
    transaction_commit_available,
};

pub fn show(
    ui: &mut egui::Ui,
    state: &mut AppState,
    account_id: AccountId,
    commands: &mut ActionCollector,
) {
    if let Some(account) = state
        .accounts
        .iter()
        .find(|account| account.id == account_id)
    {
        let header = crate::ui::account_header::format(
            &account.name,
            account.account_type,
            account.working_balance,
            account.cleared_balance,
        );
        ui.heading(header.name);
        ui.horizontal(|ui| {
            ui.label(format!("Working: {}", header.working));
            ui.label(format!("Cleared: {}", header.cleared));
            ui.label(format!("Uncleared: {}", header.uncleared));
        });
    } else {
        ui.heading("Account Transactions");
    }
    crate::ui::register::toolbar::show(ui, state, commands, false);
    load_state(ui, state, "No transactions in this account", commands);
}

pub fn show_register_header(ui: &mut egui::Ui, columns: &[RegisterColumn]) {
    crate::ui::register::table::show_header_preview(ui, columns);
}

pub fn load_state(
    ui: &mut egui::Ui,
    state: &mut AppState,
    empty_title: &str,
    commands: &mut ActionCollector,
) {
    crate::ui::register::table::show(ui, state, empty_title, commands);
}
