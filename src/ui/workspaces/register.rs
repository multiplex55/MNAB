use crate::{
    app::{dispatcher::ActionCollector, state::AppState},
    domain::AccountId,
};

pub fn show(
    ui: &mut egui::Ui,
    _state: &AppState,
    account_id: AccountId,
    _commands: &mut ActionCollector,
) {
    ui.heading("Account Transactions");
    ui.small(format!("Account {account_id}"));
    ui.label("Register data is loading…");
}
