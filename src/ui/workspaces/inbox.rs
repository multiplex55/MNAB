use crate::app::{
    command::{ApplicationAction, FinancialCommand, InboxAction, InboxCommand},
    dispatcher::ActionCollector,
    state::AppState,
};

pub fn show(ui: &mut egui::Ui, state: &AppState, commands: &mut ActionCollector) {
    ui.heading("Inbox");
    ui.label(format!("{} items to review", state.inbox_counts.total));
    let Some(item) = state.inbox_review.first() else {
        ui.add_space(24.0);
        ui.heading("Budget reviewed");
        ui.label("Inbox Zero — no unresolved actionable sources need attention.");
        return;
    };
    ui.separator();
    ui.label(format!("Item 1 of {}", state.inbox_counts.total.max(1)));
    ui.heading(&item.title);
    ui.label(format!(
        "Reasons: {}",
        item.reasons
            .iter()
            .map(|r| format!("{r:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    if let Some(date) = item.date {
        ui.label(format!("Date: {date}"));
    }
    if let Some(amount) = item.amount_cents {
        let magnitude = amount.unsigned_abs();
        ui.label(format!(
            "Amount: {}{}.{:02}",
            if amount < 0 { "-" } else { "" },
            magnitude / 100,
            magnitude % 100
        ));
    }
    for entity in &item.related_entities {
        ui.label(format!("Related: {entity}"));
    }
    ui.strong(format!("Recommended: {}", item.recommended_resolution));
    ui.horizontal_wrapped(|ui| {
        for action in &item.actions {
            if ui.button(action_label(*action)).clicked() {
                commands.push(ApplicationAction::Financial(FinancialCommand::Inbox(
                    InboxCommand::Resolve {
                        item_id: item.id.clone(),
                        action: *action,
                    },
                )));
            }
        }
    });
}

const fn action_label(action: InboxAction) -> &'static str {
    match action {
        InboxAction::Approve => "Approve",
        InboxAction::Categorize => "Categorize",
        InboxAction::Match => "Match",
        InboxAction::EnterOccurrence => "Enter",
        InboxAction::SkipOccurrence => "Skip",
        InboxAction::Clear => "Clear",
        InboxAction::Reconcile => "Reconcile",
        InboxAction::MoveMoney => "Move money",
        InboxAction::OpenTarget => "Open target",
        InboxAction::ViewFailure => "View failure",
        InboxAction::Dismiss => "Dismiss",
    }
}
