use crate::app::{
    command::AppCommand, dispatcher::ActionCollector, navigation::Workspace, state::AppState,
};
pub fn show(ctx: &egui::Context, state: &mut AppState, actions: &mut ActionCollector) {
    let modal = state.dialog.is_some();
    let editor = ctx.memory(|m| m.focused().is_some_and(|id| id == state.search_id));
    let mut keyboard_commands = Vec::new();
    super::keyboard::route(
        ctx,
        super::keyboard::Scope {
            modal,
            text_editor: editor,
            command_enabled: true,
        },
        &mut keyboard_commands,
    );
    for command in keyboard_commands {
        actions.push(command);
    }
    egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.heading("MNAB — Multi Needs A Budget");
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.search)
                    .hint_text("Search (Cmd/Ctrl+F)")
                    .id(state.search_id),
            );
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F)) {
                response.request_focus();
            }
        });
    });
    let left = egui::SidePanel::left("navigation")
        .resizable(true)
        .default_width(state.sidebar_width)
        .min_width(160.0)
        .show(ctx, |ui| super::sidebar::show(ui, state, actions));
    state.sidebar_width = left.response.rect.width();
    if state.inspector_visible {
        let right = egui::SidePanel::right("inspector")
            .resizable(true)
            .default_width(state.inspector_width)
            .min_width(190.0)
            .show(ctx, |ui| super::inspector::show(ui, state, actions));
        state.inspector_width = right.response.rect.width();
    }
    egui::CentralPanel::default().show(ctx, |ui| {
        match state.navigation.workspace {
            Workspace::Budget => ui.heading(format!(
                "Budget · {}-{:02}",
                state.selected_month.year(),
                state.selected_month.month()
            )),
            Workspace::Reports => ui.heading("Reports"),
            Workspace::AllAccounts => ui.heading("All Accounts"),
            Workspace::Account(_) => ui.heading("Account Transactions"),
        };
        if state.active_budget.is_none() {
            ui.label("Open or create a budget to begin.");
            if ui.button("Create budget").clicked() {
                actions.push(AppCommand::CreateBudget);
            }
        } else if state.accounts.is_empty() {
            ui.label("No rows yet. Use the actions below from the keyboard or mouse.");
            if ui.button("Add account").clicked() {
                actions.push(AppCommand::AddAccount);
            }
            if ui.button("Import transactions").clicked() {
                actions.push(AppCommand::Import);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn minimum_frame_renders_and_width_survives_toggle() {
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 360.0),
            )),
            ..Default::default()
        });
        let mut s = AppState::default();
        s.inspector_width = 237.0;
        show(&ctx, &mut s, &mut ActionCollector::default());
        let _ = ctx.end_pass();
        let saved = s.inspector_width;
        s.inspector_visible = false;
        ctx.begin_pass(Default::default());
        show(&ctx, &mut s, &mut ActionCollector::default());
        let _ = ctx.end_pass();
        assert_eq!(s.inspector_width, saved);
    }
}
