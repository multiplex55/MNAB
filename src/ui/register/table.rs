use super::{RegisterColumn, RegisterScope, RowMenuAction};
use crate::app::{
    command::{AppCommand, ApplicationAction, RegisterAction},
    dispatcher::ActionCollector,
    state::{AppState, EditorSurface},
};

fn width(column: RegisterColumn) -> f32 {
    match column {
        RegisterColumn::Selection => 34.0,
        RegisterColumn::Memo => 160.0,
        RegisterColumn::PayeeTransfer | RegisterColumn::Category | RegisterColumn::Account => 120.0,
        _ => 82.0,
    }
}
pub fn show_header_preview(ui: &mut egui::Ui, columns: &[RegisterColumn]) {
    ui.horizontal(|ui| {
        for column in columns {
            ui.strong(column.label());
        }
    });
}
pub fn show(
    ui: &mut egui::Ui,
    state: &mut AppState,
    _empty_title: &str,
    commands: &mut ActionCollector,
) {
    let query = &state.register_query;
    if query.refresh_active && query.last_successful.is_none() {
        ui.spinner();
        ui.label("Loading transactions…");
        return;
    }
    if let Some(error) = &query.safe_failure {
        ui.colored_label(
            ui.visuals().error_fg_color,
            format!("Could not load transactions: {error}"),
        );
        if ui.button("Retry").clicked() {
            commands.push(AppCommand::RetryOperation);
        }
        if query.last_successful.is_none() {
            return;
        }
    }
    let page = query.last_successful.clone();
    let scope = if page.as_ref().is_some_and(|p| {
        matches!(
            p.scope,
            crate::app::view_model::RegisterScope::AllTransactions
        )
    }) {
        RegisterScope::AllTransactions
    } else {
        RegisterScope::Account
    };
    let columns = super::columns_for(scope);
    let editor_active = super::editor_visible(state.editor.surface());
    let editor_mode = super::transaction_editor_mode(&state.editor);
    let editor_id = match &state.editor {
        crate::app::state::EditorState::CreatingTransaction(e)
        | crate::app::state::EditorState::EditingTransaction(e) => e.transaction_id,
        _ => None,
    };
    debug_assert_eq!(
        editor_active,
        state.editor.surface() == EditorSurface::InlineRegister
    );
    if page.as_ref().is_none_or(|p| p.rows.is_empty()) && !editor_active {
        let model = crate::ui::empty_state::model(
            crate::ui::empty_state::EmptyState::EmptyRegister,
            state.active_budget.is_some(),
            state.selected_account.is_some(),
        );
        crate::ui::empty_state::show(ui, &model, commands);
        return;
    }
    if let Some(page) = &page {
        ui.label(format!("{} transactions", page.total_matches));
    }
    use egui_extras::{Column, TableBuilder};
    let creation_key = ui.id().with("register-creation-active");
    let was_creating = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(creation_key).unwrap_or(false));
    let creating = editor_mode == Some(super::TransactionEditorMode::Creating);
    ui.ctx()
        .data_mut(|data| data.insert_temp(creation_key, creating));
    let mut table = TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click());
    if creating && !was_creating {
        table = table.scroll_to_row(0, Some(egui::Align::TOP));
    }
    for c in columns {
        table = table.column(Column::initial(width(*c)).at_least(
            if *c == RegisterColumn::Selection {
                28.0
            } else {
                60.0
            },
        ));
    }
    table
        .header(24.0, |mut header| {
            for c in columns {
                header.col(|ui| {
                    ui.strong(c.label());
                });
            }
        })
        .body(|mut body| {
            let saved = page.as_ref().map(|p| p.rows.as_slice()).unwrap_or_default();
            let ids = saved
                .iter()
                .map(|row| row.transaction_id)
                .collect::<Vec<_>>();
            for placement in super::table_row_placements(&ids, editor_mode, editor_id) {
                match placement {
                    super::TableRowPlacement::Editor { replaces } => {
                        let missing = editor_mode == Some(super::TransactionEditorMode::Editing)
                            && replaces.is_none();
                        body.row(54.0, |mut row| {
                            // Editor rows deliberately have no row-level response handlers.
                            for c in columns {
                                row.col(|ui| {
                                    super::editor::show_cell(
                                        ui, state, commands, scope, *c, missing,
                                    )
                                });
                            }
                        });
                    }
                    super::TableRowPlacement::Saved(index) => {
                        let model = &saved[index];
                        body.row(26.0, |mut row| {
                            row.set_selected(
                                state.register_selection.contains(model.transaction_id),
                            );
                            for c in columns {
                                row.col(|ui| cell(ui, *c, model, state, commands));
                            }
                            let response = row.response();
                            if response.clicked() {
                                let m = response.ctx.input(|i| i.modifiers);
                                commands.push(ApplicationAction::Register(RegisterAction::Click {
                                    id: model.transaction_id,
                                    ctrl: m.command || m.ctrl,
                                    shift: m.shift,
                                }));
                            }
                            if response.double_clicked() {
                                commands.push(ApplicationAction::Register(
                                    RegisterAction::BeginEdit(model.transaction_id),
                                ));
                            }
                            response.context_menu(|ui| context_menu(ui, model, commands));
                        });
                    }
                }
            }
        });
}
fn cell(
    ui: &mut egui::Ui,
    c: RegisterColumn,
    m: &crate::app::view_model::RegisterRowView,
    state: &AppState,
    commands: &mut ActionCollector,
) {
    match c {
        RegisterColumn::Selection => {
            let mut selected = state.register_selection.contains(m.transaction_id);
            if ui.checkbox(&mut selected, "").clicked() {
                commands.push(ApplicationAction::Register(RegisterAction::Click {
                    id: m.transaction_id,
                    ctrl: true,
                    shift: false,
                }));
            }
        }
        RegisterColumn::Account => {
            ui.label(&m.account_name);
        }
        RegisterColumn::Date => {
            ui.label(crate::ui::format::register_date(m.date));
        }
        RegisterColumn::PayeeTransfer => {
            ui.label(&m.payee_name);
        }
        RegisterColumn::Category => {
            ui.label(&m.category_name);
        }
        RegisterColumn::Memo => {
            ui.label(m.memo.as_deref().unwrap_or(""));
        }
        RegisterColumn::Outflow => {
            crate::ui::format::money_cell(
                ui,
                crate::domain::Money::from_minor_units(m.outflow_cents),
            );
        }
        RegisterColumn::Inflow => {
            crate::ui::format::money_cell(
                ui,
                crate::domain::Money::from_minor_units(m.inflow_cents),
            );
        }
        RegisterColumn::Cleared => {
            ui.label(&m.cleared_state);
        }
        RegisterColumn::Approved => {
            ui.label(if m.approved {
                "Approved"
            } else {
                "Needs approval"
            });
        }
        RegisterColumn::RunningBalance => {
            if let Some(value) = m.running_balance_cents {
                crate::ui::format::money_cell(ui, crate::domain::Money::from_minor_units(value));
            } else {
                ui.label("—");
            }
        }
    }
}
fn context_menu(
    ui: &mut egui::Ui,
    row: &crate::app::view_model::RegisterRowView,
    commands: &mut ActionCollector,
) {
    for action in super::valid_row_actions(row) {
        let label = match action {
            RowMenuAction::Edit => "Edit",
            RowMenuAction::Clear => "Mark cleared",
            RowMenuAction::Uncleared => "Mark uncleared",
            RowMenuAction::Approve => "Approve",
            RowMenuAction::Delete => "Delete",
        };
        if ui.button(label).clicked() {
            commands.push(ApplicationAction::Register(match action {
                RowMenuAction::Edit => RegisterAction::BeginEdit(row.transaction_id),
                RowMenuAction::Clear => RegisterAction::SetClearance {
                    id: row.transaction_id,
                    clearance: crate::domain::Clearance::Cleared,
                },
                RowMenuAction::Uncleared => RegisterAction::SetClearance {
                    id: row.transaction_id,
                    clearance: crate::domain::Clearance::Uncleared,
                },
                RowMenuAction::Approve => RegisterAction::Approve(row.transaction_id),
                RowMenuAction::Delete => RegisterAction::Delete(row.transaction_id),
            }));
            ui.close();
        }
    }
}
