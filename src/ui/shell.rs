use crate::app::{dispatcher::ActionCollector, navigation::Workspace, state::AppState};
pub fn show(ctx: &egui::Context, state: &mut AppState, actions: &mut ActionCollector) {
    let palette_pressed = ctx.input(|input| {
        input.events.iter().any(|event| match event {
            egui::Event::Key {
                key,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } => {
                let configured_key = state
                    .palette_shortcut
                    .trim()
                    .rsplit('+')
                    .next()
                    .unwrap_or("");
                modifiers.command
                    && !modifiers.alt
                    && modifiers.shift
                        == state
                            .palette_shortcut
                            .to_ascii_lowercase()
                            .contains("shift+")
                    && match key {
                        egui::Key::P => configured_key.eq_ignore_ascii_case("P"),
                        egui::Key::K => configured_key.eq_ignore_ascii_case("K"),
                        _ => false,
                    }
            }
            _ => false,
        })
    });
    if palette_pressed && !state.palette.open {
        let initiating = ctx.memory(|memory| memory.focused());
        state.palette.open(initiating);
    }
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
            ui.menu_button("Budget", |ui| {
                use crate::app::command::{ApplicationAction, BudgetAction};
                if ui.button("New budget…").clicked() {
                    actions.push(ApplicationAction::Budget(BudgetAction::ShowCreate));
                    ui.close();
                }
                if ui.button("Open budget…").clicked() {
                    actions.push(ApplicationAction::Budget(BudgetAction::ShowOpen));
                    ui.close();
                }
                if ui.button("Recent budgets…").clicked() {
                    actions.push(ApplicationAction::Budget(BudgetAction::ShowRecents));
                    ui.close();
                }
            });
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
    egui::CentralPanel::default().show(ctx, |ui| match state.navigation.workspace {
        Workspace::Budget => super::workspaces::budget::show(ui, state, actions),
        Workspace::Reports => super::workspaces::reports::show(ui, state, actions),
        Workspace::AllAccounts => super::workspaces::all_accounts::show(ui, state, actions),
        Workspace::Inbox => super::workspaces::inbox::show(ui, state, actions),
        Workspace::Account(id) => super::workspaces::register::show(ui, state, id, actions),
    });
    show_palette(ctx, state, actions);
    show_budget_dialog(ctx, state);
}

fn show_budget_dialog(ctx: &egui::Context, state: &mut AppState) {
    let Some(dialog) = state.dialog.as_ref().map(|dialog| dialog.dialog.clone()) else {
        return;
    };
    let (title, guidance) = match dialog {
        crate::app::state::Dialog::CreateBudget => (
            "Create a budget",
            "Choose a name, managed filename, currency, and initial month.",
        ),
        crate::app::state::Dialog::OpenBudget => (
            "Open a budget",
            "Select a managed MNAB budget. It will be validated completely before replacing the current session.",
        ),
        crate::app::state::Dialog::RecentBudgets => (
            "Recent budgets",
            "Open, rename, archive, remove, validate, repair, reveal, or delete a catalog entry.",
        ),
        crate::app::state::Dialog::RenameBudget => (
            "Rename budget",
            "Renaming changes only the visible catalog/session name.",
        ),
        crate::app::state::Dialog::ArchiveBudget => (
            "Archive budget",
            "Archiving hides this entry from recents without deleting financial data.",
        ),
        crate::app::state::Dialog::RepairBudget => (
            "Repair budget",
            "Review diagnostics and the validated backup before applying an explicit repair.",
        ),
        crate::app::state::Dialog::RecoveryChoice => (
            "Budget recovery required",
            "Opening was refused. Choose a backup or an explicit diagnostic/repair action; MNAB will not reset the database.",
        ),
        crate::app::state::Dialog::ConfirmDelete => (
            "Delete budget",
            "Type the exact current budget name to enable permanent managed-file deletion.",
        ),
        crate::app::state::Dialog::Reconcile(_)
        | crate::app::state::Dialog::Import(_)
        | crate::app::state::Dialog::Settings => return,
    };
    egui::Modal::new(egui::Id::new("budget-lifecycle-dialog")).show(ctx, |ui| {
        ui.heading(title);
        ui.label(guidance);
        if ui.button("Cancel").clicked() {
            state.dialog = None;
        }
    });
}

fn show_palette(ctx: &egui::Context, state: &mut AppState, actions: &mut ActionCollector) {
    if !state.palette.open {
        return;
    }
    let context = crate::app::palette::CommandContext {
        budget_open: state.active_budget.is_some(),
        account_register: matches!(state.navigation.workspace, Workspace::Account(_)),
        budget_workspace: state.navigation.workspace == Workspace::Budget,
        mutations_disabled: state.mutations_disabled,
        has_selection: false,
        editing: false,
        dialog_open: state.dialog.is_some(),
        text_editor_owns_shortcuts: false,
        lifecycle_busy: false,
        operation_locked: !state.operations.is_empty(),
        can_undo: false,
        can_redo: false,
    };
    let descriptors = crate::app::palette::commands_for(context);
    let matches = crate::app::palette::fuzzy(&state.palette.query, &descriptors);
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        state.palette.close(ctx);
        return;
    }
    if ctx.input(|input| input.key_pressed(egui::Key::ArrowUp)) {
        state.palette.move_up(matches.len());
    }
    if ctx.input(|input| input.key_pressed(egui::Key::ArrowDown)) {
        state.palette.move_down(matches.len());
    }
    if ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
        if let Some(item) = state.palette.selected(&matches)
            && item.enabled
        {
            actions.push(item.command);
            state.palette.close(ctx);
            return;
        }
    }
    egui::Window::new("Command palette")
        .id(egui::Id::new("command-palette"))
        .collapsible(false)
        .resizable(false)
        .default_width(480.0)
        .show(ctx, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut state.palette.query)
                    .hint_text("Type a command…")
                    .id(egui::Id::new("palette-query")),
            )
            .request_focus();
            for (index, item) in matches.iter().take(12).enumerate() {
                let label = item.shortcut.as_ref().map_or_else(
                    || item.title.clone(),
                    |key| format!("{}    {}", item.title, key),
                );
                let response = ui.add_enabled(
                    item.enabled,
                    egui::Button::new(label).selected(index == state.palette.selection),
                );
                if let Some(reason) = &item.disabled_explanation {
                    response.clone().on_disabled_hover_text(reason);
                }
                if response.clicked() {
                    actions.push(item.command);
                    state.palette.close(ctx);
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
