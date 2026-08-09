use crate::app::{dispatcher::ActionCollector, navigation::Workspace, state::AppState};

#[must_use]
pub const fn account_toolbar_actions() -> [(&'static str, crate::app::command::AppCommand); 4] {
    [
        ("New", crate::app::command::AppCommand::ContextualNew),
        ("Import", crate::app::command::AppCommand::Import),
        ("Transfer", crate::app::command::AppCommand::CreateTransfer),
        (
            "Reconcile",
            crate::app::command::AppCommand::ReconcileAccount,
        ),
    ]
}
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
            popup: false,
        },
        &mut keyboard_commands,
    );
    for command in keyboard_commands {
        actions.push(command);
    }
    egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.heading(format!("MNAB — {}", state.budget_name));
            let account_context = matches!(state.navigation.workspace, Workspace::Account(_));
            let availability = state.action_context();
            if account_context {
                for (label, command) in account_toolbar_actions() {
                    super::widgets::action_button(ui, label, command, availability, actions);
                }
                if let Workspace::Account(id) = state.navigation.workspace
                    && let Some(account) = state.accounts.iter().find(|account| account.id == id)
                {
                    ui.strong(format!("{} · {}", account.name, account.working_balance));
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
            if ui
                .button(format!("Inbox ({})", state.inbox_counts.total))
                .clicked()
            {
                state.navigation.workspace = Workspace::Inbox;
            }
            ui.menu_button("Data", |ui| {
                use crate::app::command::{ApplicationAction, DataAction};
                if ui.button("Budget settings…").clicked() {
                    state.maintenance_budget_name.clone_from(&state.budget_name);
                    state.open_dialog(
                        crate::app::state::Dialog::BudgetMaintenance,
                        egui::Id::new("data-menu"),
                        egui::Id::new("toolbar"),
                    );
                    ui.close();
                }
                ui.separator();
                ui.label("Maintenance");
                if ui.button("Create validated backup").clicked() {
                    actions.push(ApplicationAction::Data(DataAction::CreateBackup));
                    ui.close();
                }
                if ui.button("Restore from backup…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("MNAB backup metadata", &["json"])
                        .pick_file()
                    {
                        actions.push(ApplicationAction::Data(DataAction::RestoreBackup {
                            metadata_path: path,
                            confirmed: false,
                        }));
                    }
                    ui.close();
                }
                if ui.button("Validate database").clicked() {
                    actions.push(ApplicationAction::Data(DataAction::Validate));
                    ui.close();
                }
                if ui.button("Reveal data folder…").clicked() {
                    actions.push(ApplicationAction::Data(DataAction::RevealDataDirectory));
                    ui.close();
                }
                if ui.button("Reveal backup folder…").clicked() {
                    actions.push(ApplicationAction::Data(DataAction::RevealBackupDirectory));
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
        Workspace::Overview => super::workspaces::overview::show(ui, state, actions),
        Workspace::Budget => super::workspaces::budget::show(ui, state, actions),
        Workspace::Categories => super::workspaces::categories::show(ui, state, actions),
        Workspace::Reports => super::workspaces::reports::show(ui, state, actions),
        Workspace::AllTransactions => super::workspaces::all_accounts::show(ui, state, actions),
        Workspace::Inbox => super::workspaces::inbox::show(ui, state, actions),
        Workspace::Account(id) => super::workspaces::register::show(ui, state, id, actions),
    });
    show_notifications(ctx, state, actions);
    show_palette(ctx, state, actions);
    show_workflow_editor(ctx, state, actions);
    show_budget_dialog(ctx, state, actions);
}

fn show_notifications(ctx: &egui::Context, state: &mut AppState, actions: &mut ActionCollector) {
    if state.notifications.is_empty() {
        return;
    }
    let mut dismiss = None;
    egui::TopBottomPanel::bottom("application-notifications").show(ctx, |ui| {
        for (index, notice) in state.notifications.iter().enumerate() {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    let color = match notice.kind {
                        crate::app::state::NotificationKind::Error => egui::Color32::RED,
                        crate::app::state::NotificationKind::Warning => egui::Color32::YELLOW,
                        crate::app::state::NotificationKind::Information => {
                            ui.visuals().strong_text_color()
                        }
                    };
                    ui.colored_label(color, &notice.title);
                    if notice.persistent && ui.button("Retry").clicked() {
                        actions.push(crate::app::command::AppCommand::RetryOperation);
                    }
                    if notice.persistent {
                        ui.collapsing("Details", |ui| {
                            ui.label(&notice.detail);
                        });
                    } else {
                        ui.label(&notice.detail);
                    }
                    if ui.button("Dismiss").clicked() {
                        dismiss = Some(index);
                    }
                });
            });
        }
    });
    if let Some(index) = dismiss {
        state.dismiss_notification(index);
    }
}

/// Renders the runtime-owned editor state.  Editors remain visible while a typed worker request is
/// in flight or after a retryable failure, so a failed operation never discards user input.
fn show_workflow_editor(ctx: &egui::Context, state: &mut AppState, actions: &mut ActionCollector) {
    use crate::app::state::{CommitState, EditorState};
    if !state.editor.is_active() {
        return;
    }
    let mut cancel = false;
    let mut commit = false;
    egui::Window::new("Workflow")
        .id(egui::Id::new("workflow-editor"))
        .collapsible(false).resizable(true).show(ctx, |ui| {
        match &mut state.editor {
            EditorState::CreatingAccount(editor) | EditorState::EditingAccount(editor) => {
                let creating = editor.account_id.is_none();
                ui.heading(if creating { "Create account" } else { "Edit account" });
                ui.label("Name"); ui.text_edit_singleline(&mut editor.form.name);
                egui::ComboBox::from_label("Type").selected_text(editor.form.account_type.map_or("Choose…".into(), |v| format!("{v:?}"))).show_ui(ui, |ui| {
                    for kind in [crate::domain::AccountType::Checking, crate::domain::AccountType::Savings, crate::domain::AccountType::Cash, crate::domain::AccountType::CreditCard, crate::domain::AccountType::Loan, crate::domain::AccountType::Asset, crate::domain::AccountType::Liability] { ui.selectable_value(&mut editor.form.account_type, Some(kind), format!("{kind:?}")); }
                });
                if creating { ui.label("Opening balance (positive magnitude)"); ui.text_edit_singleline(&mut editor.form.opening_magnitude); ui.label("Opening date (YYYY-MM-DD)"); ui.text_edit_singleline(&mut editor.form.opening_date); }
                egui::ComboBox::from_label("Group").selected_text(editor.form.group_id.and_then(|id| state.account_groups.iter().find(|g| g.id == id).map(|g| g.name.clone())).unwrap_or_else(|| "Ungrouped".into())).show_ui(ui, |ui| {
                    ui.selectable_value(&mut editor.form.group_id, None, "Ungrouped");
                    for group in &state.account_groups { ui.selectable_value(&mut editor.form.group_id, Some(group.id), &group.name); }
                });
                ui.label("Note"); ui.text_edit_multiline(&mut editor.form.note);
                ui.checkbox(&mut editor.form.favorite, "Favorite");
            }
            EditorState::CreatingTransfer(editor) => {
                ui.heading("Transfer");
                account_picker(ui, "From account", &mut editor.draft.from_account, &state.accounts);
                account_picker(ui, "To account", &mut editor.draft.to_account, &state.accounts);
                ui.label("Amount"); ui.text_edit_singleline(&mut editor.draft.amount);
                ui.label("Date (YYYY-MM-DD)"); ui.text_edit_singleline(&mut editor.draft.date);
                ui.label("Memo"); ui.text_edit_singleline(&mut editor.draft.memo);
                ui.small("Optional category/goal effect is applied to the destination account.");
                if let Ok(summary) = editor.draft.summary() { ui.separator(); ui.strong("Balance effect preview"); ui.label(format!("From: {} · To: {} · Goal/category: {}", summary.cash_decreases, summary.savings_increases, summary.goal_increases)); }
            }
            EditorState::Importing(editor) => {
                ui.heading("Import transactions");
                ui.label("1. Select file  2. Parse  3. Map CSV  4. Match & deduplicate  5. Review  6. Apply atomically");
                ui.label(format!("File: {}", if editor.source.as_os_str().is_empty() { "No file selected".into() } else { editor.source.display().to_string() }));
                if ui.button("Choose CSV or OFX file…").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Statements", &["csv", "ofx", "qfx"])
                        .pick_file()
                {
                    editor.source = path;
                    editor.metadata.dirty = true;
                }
                ui.small("Exact duplicates are excluded by default. Applied transactions appear in Inbox for approval and categorization.");
            }
            EditorState::Reconciling(editor) => {
                ui.heading("Reconcile account");
                ui.label("Statement date (YYYY-MM-DD)"); ui.text_edit_singleline(&mut editor.statement_date);
                ui.label("Statement balance"); ui.text_edit_singleline(&mut editor.statement_balance);
                let cleared = editor.account_id.and_then(|id| state.accounts.iter().find(|a| a.id == id)).map_or(crate::domain::Money::ZERO, |a| a.cleared_balance);
                ui.label(format!("Current cleared balance: {cleared}"));
                if let Ok(statement) = editor.statement_balance.parse::<crate::domain::Money>() { if let Ok(difference) = statement.checked_sub(cleared) { ui.label(format!("Difference: {difference}")); ui.small(if difference == crate::domain::Money::ZERO { "Ready to complete: difference is exactly zero." } else { "Completion requires an exact zero difference." }); } }
                ui.small("Transactions on or before the statement date may be cleared. Previously reconciled transactions are protected and require explicit confirmation to edit or delete.");
            }
            EditorState::CreatingTransaction(_) | EditorState::EditingTransaction(_) => { ui.heading("Transaction editor"); ui.label("Complete the transaction fields in the register."); }
            EditorState::ManagingCategory(_) | EditorState::ManagingAccountGroup(_) | EditorState::Idle => {}
        }
        if let Some(meta) = state.editor.metadata() {
            for error in &meta.validation_errors { ui.colored_label(egui::Color32::RED, error); }
            if meta.commit_state == CommitState::Submitting { ui.spinner(); ui.label("Submitting…"); }
            ui.horizontal(|ui| { if ui.add_enabled(meta.commit_state != CommitState::Submitting, egui::Button::new(if meta.commit_state == CommitState::Failed { "Retry" } else { "Commit" })).clicked() { commit = true; } if ui.button("Cancel").clicked() { cancel = true; } });
        }
    });
    if commit {
        actions.push(crate::app::command::AppCommand::Commit);
    }
    if cancel {
        actions.push(crate::app::command::AppCommand::Cancel);
    }
}

fn account_picker(
    ui: &mut egui::Ui,
    label: &str,
    selected: &mut Option<crate::domain::AccountId>,
    accounts: &[crate::app::state::AccountSummary],
) {
    egui::ComboBox::from_label(label)
        .selected_text(
            selected
                .and_then(|id| accounts.iter().find(|a| a.id == id).map(|a| a.name.clone()))
                .unwrap_or_else(|| "Choose…".into()),
        )
        .show_ui(ui, |ui| {
            for account in accounts.iter().filter(|a| !a.closed) {
                ui.selectable_value(selected, Some(account.id), &account.name);
            }
        });
}

fn show_budget_dialog(ctx: &egui::Context, state: &mut AppState, actions: &mut ActionCollector) {
    let Some(dialog) = state.dialog.as_ref().map(|dialog| dialog.dialog.clone()) else {
        return;
    };
    if matches!(dialog, crate::app::state::Dialog::Onboarding) {
        show_onboarding_wizard(ctx, state, actions);
        return;
    }
    let (title, guidance) = match dialog {
        crate::app::state::Dialog::Onboarding => unreachable!(),
        crate::app::state::Dialog::BudgetMaintenance => (
            "Budget settings",
            "Rename budget metadata, back up, restore, validate, repair, or reveal data and backup folders.",
        ),
        crate::app::state::Dialog::RepairBudget => (
            "Repair budget",
            "Review diagnostics and the validated backup before applying an explicit repair.",
        ),
        crate::app::state::Dialog::RecoveryChoice => (
            "Budget recovery required",
            "Opening was refused. Choose a backup or an explicit diagnostic/repair action; MNAB will not reset the database.",
        ),
        crate::app::state::Dialog::Reconcile(_)
        | crate::app::state::Dialog::Import(_)
        | crate::app::state::Dialog::Settings => return,
    };
    egui::Modal::new(egui::Id::new("budget-lifecycle-dialog")).show(ctx, |ui| {
        ui.heading(title);
        ui.label(guidance);
        if matches!(dialog, crate::app::state::Dialog::BudgetMaintenance) {
            ui.separator();
            ui.label("Budget name");
            ui.text_edit_singleline(&mut state.maintenance_budget_name);
            if ui.button("Save name").clicked() {
                actions.push(crate::app::command::ApplicationAction::Data(crate::app::command::DataAction::RenameBudget { name: state.maintenance_budget_name.clone() }));
            }
            ui.separator();
            ui.label("Repair preview: rebuild SQLite indexes on a private copy, validate it, create a safety backup, then replace the fixed database.");
            if ui.button("Review index repair…").clicked() {
                actions.push(crate::app::command::ApplicationAction::Data(crate::app::command::DataAction::Repair { request: crate::storage::repair::RepairRequest::Reindex, confirmed: false }));
            }
        }
        if matches!(dialog, crate::app::state::Dialog::RepairBudget | crate::app::state::Dialog::RecoveryChoice) {
            ui.separator();
            ui.colored_label(egui::Color32::YELLOW, "This operation replaces mnab-data/mnab.sqlite3. A validated safety backup is created first; no reset or deletion is available.");
            if ui.button("Confirm operation").clicked()
                && let Some(action) = state.pending_data_action.take()
            {
                let confirmed = match action {
                    crate::app::command::DataAction::RestoreBackup { metadata_path, .. } => crate::app::command::DataAction::RestoreBackup { metadata_path, confirmed: true },
                    crate::app::command::DataAction::Repair { request, .. } => crate::app::command::DataAction::Repair { request, confirmed: true },
                    other => other,
                };
                actions.push(crate::app::command::ApplicationAction::Data(confirmed));
            }
        }
        if ui.button("Cancel").clicked() {
            state.pending_data_action = None;
            state.dialog = None;
        }
    });
}

fn show_onboarding_wizard(
    ctx: &egui::Context,
    state: &mut AppState,
    actions: &mut ActionCollector,
) {
    egui::Modal::new(egui::Id::new("onboarding-wizard")).show(ctx, |ui| {
        let wizard = &mut state.onboarding;
        ui.heading(format!("Set up your budget — {} of 4", wizard.step));
        match wizard.step {
            1 => { ui.label("Budget identity"); ui.text_edit_singleline(&mut wizard.budget_name); }
            2 => {
                ui.label("First account");
                ui.label("Account name"); ui.text_edit_singleline(&mut wizard.account.name);
                egui::ComboBox::from_label("Account type").selected_text(format!("{:?}", wizard.account.account_type)).show_ui(ui, |ui| {
                    for kind in [crate::domain::AccountType::Checking, crate::domain::AccountType::Savings, crate::domain::AccountType::CreditCard, crate::domain::AccountType::Loan] { ui.selectable_value(&mut wizard.account.account_type, kind, format!("{kind:?}")); }
                });
                ui.label("Current balance"); ui.text_edit_singleline(&mut wizard.account.current_balance);
                if let Some(help) = wizard.debt_help() { ui.small(help); }
                ui.label("Balance date (YYYY-MM-DD)"); ui.text_edit_singleline(&mut wizard.account.balance_date);
                ui.label("Account group"); ui.text_edit_singleline(&mut wizard.account.group);
                ui.label("Optional note"); ui.text_edit_singleline(&mut wizard.account.note);
                ui.small("The account fields use the same positive-magnitude contract as AccountDialogForm.");
            }
            3 => {
                ui.label("Starter categories (all optional)");
                for name in crate::ui::onboarding::STARTER_CATEGORIES { let mut selected = wizard.selected_categories.contains(*name); if ui.checkbox(&mut selected, *name).changed() { if selected { wizard.selected_categories.insert((*name).into()); } else { wizard.selected_categories.remove(*name); } } }
            }
            _ => {
                ui.label("Review"); ui.label(format!("Budget: {}", wizard.budget_name));
                ui.label(format!("First account: {} ({:?})", wizard.account.name, wizard.account.account_type));
                match wizard.signed_opening_preview() { Ok(value) => { ui.label(format!("Opening ledger effect: {value}")); }, Err(error) => { ui.colored_label(egui::Color32::RED, error); } }
                ui.label(format!("Starter categories: {}", wizard.selected_categories.len()));
            }
        }
        ui.horizontal(|ui| {
            if wizard.step > 1 && ui.button("Back").clicked() { wizard.step -= 1; }
            if wizard.step < 4 {
                if ui.button("Next").clicked() { wizard.step += 1; }
            } else if ui.button("Finish setup").clicked() { actions.push(crate::app::command::AppCommand::CompleteOnboarding); }
            if ui.button("Cancel").clicked() { state.dialog = None; }
        });
    });
}

fn show_palette(ctx: &egui::Context, state: &mut AppState, actions: &mut ActionCollector) {
    if !state.palette.open {
        return;
    }
    let selected_reconciled_transaction = state
        .register_query
        .last_successful
        .as_ref()
        .is_some_and(|page| {
            page.rows
                .iter()
                .any(|row| row.reconciled && state.register_selection.contains(row.transaction_id))
        });
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
        selected_reconciled_transaction,
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
    #[test]
    fn every_account_toolbar_action_is_reachable() {
        use crate::app::command::{
            CommandAvailabilityContext, CommandWorkspace, command_availability,
        };
        let context = CommandAvailabilityContext {
            database_available: true,
            workspace: CommandWorkspace::AccountRegister,
            selected_account: true,
            ..Default::default()
        };
        for (label, command) in account_toolbar_actions() {
            assert!(!label.is_empty());
            assert!(command_availability(context, command).enabled, "{label}");
        }
    }
}
