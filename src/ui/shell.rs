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
            ui.heading(format!("MNAB — {}", state.budget_name));
            let selected = state.selected_account.is_some();
            for (label, command) in [
                (
                    "New Transaction",
                    crate::app::command::AppCommand::AddTransaction,
                ),
                ("Transfer", crate::app::command::AppCommand::CreateTransfer),
                ("Import", crate::app::command::AppCommand::Import),
                (
                    "Reconcile",
                    crate::app::command::AppCommand::ReconcileAccount,
                ),
            ] {
                let response = ui.add_enabled(selected, egui::Button::new(label));
                response
                    .clone()
                    .on_disabled_hover_text("Select an account to use this action");
                if response.clicked() {
                    actions.push(command);
                }
            }
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.search)
                    .hint_text("Search (Cmd/Ctrl+F)")
                    .desired_width(180.0)
                    .id(state.search_id),
            );
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F)) {
                response.request_focus();
            }
            ui.menu_button("Data", |ui| {
                use crate::app::command::{ApplicationAction, BudgetAction};
                if ui.button("Budget settings…").clicked() {
                    actions.push(ApplicationAction::Budget(BudgetAction::ShowRecents));
                    ui.close();
                }
                ui.separator();
                ui.label("Maintenance");
                if ui.button("Reveal data folder…").clicked() {
                    actions.push(ApplicationAction::Budget(BudgetAction::ShowRecents));
                    ui.close();
                }
                if ui.button("Reveal backup folder…").clicked() {
                    actions.push(ApplicationAction::Budget(BudgetAction::ShowRecents));
                    ui.close();
                }
            });
            if ui.button("Settings").clicked() {
                actions.push(crate::app::command::AppCommand::Settings);
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
        Workspace::Overview => super::workspaces::overview::show(ui, state),
        Workspace::Budget => super::workspaces::budget::show(ui, state, actions),
        Workspace::Categories => super::workspaces::categories::show(ui, state, actions),
        Workspace::Reports => super::workspaces::reports::show(ui, state, actions),
        Workspace::AllTransactions => super::workspaces::all_accounts::show(ui, state, actions),
        Workspace::Inbox => super::workspaces::inbox::show(ui, state, actions),
        Workspace::Account(id) => super::workspaces::register::show(ui, state, id, actions),
    });
    show_palette(ctx, state, actions);
    show_budget_dialog(ctx, state, actions);
}

fn show_budget_dialog(ctx: &egui::Context, state: &mut AppState, _actions: &mut ActionCollector) {
    let Some(dialog) = state.dialog.as_ref().map(|dialog| dialog.dialog.clone()) else {
        return;
    };
    let (title, guidance) = match dialog {
        crate::app::state::Dialog::CreateBudget => (
            "First-run setup",
            "1. Name your budget  →  2. Add your first account (name, type, current balance, balance date, group, optional note)  →  3. Choose any starter categories, including none  →  4. Review and create. Amounts are dollars; debt balances are entered as a positive amount owed. Setup is committed atomically.",
        ),
        crate::app::state::Dialog::OpenBudget => (
            "Database maintenance",
            "MNAB opens mnab-data/mnab.sqlite3 automatically; file picker workflows are retired.",
        ),
        crate::app::state::Dialog::RecentBudgets => (
            "Budget settings",
            "Rename budget metadata, back up, restore, validate, repair, or reveal data and backup folders.",
        ),
        crate::app::state::Dialog::RenameBudget => (
            "Rename budget",
            "Renaming changes only database metadata; mnab.sqlite3 keeps its fixed filename.",
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
            "Delete unavailable",
            "The fixed database lifecycle preserves data; use restore or repair for maintenance.",
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
        database_available: state.active_budget.is_some(),
        account_register: matches!(state.navigation.workspace, Workspace::Account(_)),
        categories_workspace: state.navigation.workspace == Workspace::Categories,
        budget_workspace: state.navigation.workspace == Workspace::Budget,
        overview_workspace: state.navigation.workspace == Workspace::Overview,
        mutations_disabled: state.mutations_disabled,
        has_selection: false,
        editing: false,
        dialog_open: state.dialog.is_some(),
        text_editor_owns_shortcuts: false,
        lifecycle_busy: false,
        mutation_locked: !state.operations.is_empty(),
        can_undo: state.can_undo,
        can_redo: state.can_redo,
        selected_account: state.selected_account.is_some(),
        selected_transaction: state.selected_transaction.is_some(),
        register_focused: state.register_focus.is_some(),
        import_active: matches!(state.editor, crate::app::state::EditorState::Importing(_)),
        reconciliation_active: matches!(
            state.editor,
            crate::app::state::EditorState::Reconciling(_)
        ),
    };
    let mut descriptors = crate::app::palette::commands_for(context);
    for descriptor in &mut descriptors {
        match descriptor.command {
            crate::app::command::AppCommand::Undo => {
                if let Some(label) = &state.undo_label {
                    descriptor.title = format!("Undo \"{label}\"");
                }
            }
            crate::app::command::AppCommand::Redo => {
                if let Some(label) = &state.redo_label {
                    descriptor.title = format!("Redo \"{label}\"");
                }
            }
            _ => {}
        }
    }
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
