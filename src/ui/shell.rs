use crate::app::{command::AppCommand, navigation::Workspace, state::AppState};
pub fn show(ctx: &egui::Context, state: &mut AppState, commands: &mut Vec<AppCommand>) {
    let modal = state.dialog.is_some();
    let editor = ctx.memory(|m| m.focused().is_some_and(|id| id == state.search_id));
    super::keyboard::route(
        ctx,
        super::keyboard::Scope {
            modal,
            text_editor: editor,
            command_enabled: true,
        },
        commands,
    );
    egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.heading("MNAB — Multi Needs A Budget");
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.search)
                    .hint_text("Search (Cmd/Ctrl+F)")
                    .id(state.search_id),
            );
            if commands.contains(&AppCommand::FocusSearch) {
                response.request_focus();
            }
        });
    });
    let left = egui::SidePanel::left("navigation")
        .resizable(true)
        .default_width(state.sidebar_width)
        .min_width(160.0)
        .show(ctx, |ui| super::sidebar::show(ui, state, commands));
    state.sidebar_width = left.response.rect.width();
    if state.inspector_visible {
        let right = egui::SidePanel::right("inspector")
            .resizable(true)
            .default_width(state.inspector_width)
            .min_width(190.0)
            .show(ctx, |ui| super::inspector::show(ui, state, commands));
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
                commands.push(AppCommand::CreateBudget);
            }
        } else if state.accounts.is_empty() {
            ui.label("No rows yet. Use the actions below from the keyboard or mouse.");
            if ui.button("Add account").clicked() {
                commands.push(AppCommand::AddAccount);
            }
            if ui.button("Import transactions").clicked() {
                commands.push(AppCommand::Import);
            }
        }
    });
    // Global actions are executed here, never by individual widgets. Consuming the
    // toggle also prevents a queued command from being replayed next frame.
    let toggles = commands
        .iter()
        .filter(|command| **command == AppCommand::ToggleInspector)
        .count();
    if toggles % 2 == 1 {
        state.inspector_visible = !state.inspector_visible;
    }
    commands.retain(|command| *command != AppCommand::ToggleInspector);
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
        show(&ctx, &mut s, &mut vec![]);
        let _ = ctx.end_pass();
        let saved = s.inspector_width;
        s.inspector_visible = false;
        ctx.begin_pass(Default::default());
        show(&ctx, &mut s, &mut vec![]);
        let _ = ctx.end_pass();
        assert_eq!(s.inspector_width, saved);
    }
}
