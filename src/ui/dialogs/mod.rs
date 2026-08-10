mod account;
mod import;
mod reconcile;
mod transfer;

use crate::app::{
    dispatcher::ActionCollector,
    state::{AppState, CommitState, EditorState, EditorSurface},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Presentation {
    pub title: &'static str,
    pub primary: &'static str,
}

pub fn presentation(editor: &EditorState) -> Option<Presentation> {
    let (title, primary) = match editor {
        EditorState::CreatingAccount(_) => ("New Account", "Create Account"),
        EditorState::EditingAccount(_) => ("Edit Account", "Save Changes"),
        EditorState::CreatingTransfer(_) => ("Transfer Money", "Transfer"),
        EditorState::Importing(_) => ("Import Transactions", "Import Transactions"),
        EditorState::Reconciling(_) => ("Reconcile Account", "Finish Reconciliation"),
        _ => return None,
    };
    Some(Presentation { title, primary })
}

pub fn show(ctx: &egui::Context, state: &mut AppState, actions: &mut ActionCollector) {
    if state.editor.surface() != EditorSurface::Modal {
        return;
    }
    let Some(mut p) = presentation(&state.editor) else {
        debug_assert!(false, "transaction editor reached modal rendering");
        return;
    };
    let account_name = match &state.editor {
        EditorState::Reconciling(e) => e
            .account_id
            .and_then(|id| state.accounts.iter().find(|a| a.id == id))
            .map(|a| a.name.as_str()),
        _ => None,
    };
    let reconcile_title = account_name.map(|name| format!("Reconcile {name}"));
    let title = reconcile_title.as_deref().unwrap_or(p.title);
    let mut commit = false;
    let mut cancel = false;
    let response = egui::Modal::new(egui::Id::new("editor-modal")).show(ctx, |ui| {
        ui.set_min_width(420.0);
        ui.heading(title);
        ui.separator();
        match &mut state.editor {
            EditorState::CreatingAccount(e) | EditorState::EditingAccount(e) => {
                account::show(ui, e, &state.account_groups)
            }
            EditorState::CreatingTransfer(e) => transfer::show(ui, e, &state.accounts),
            EditorState::Importing(e) => import::show(ui, e),
            EditorState::Reconciling(e) => reconcile::show(ui, e, &state.accounts),
            _ => unreachable!("non-modal editor reached modal rendering"),
        }
        if let Some(meta) = state.editor.metadata() {
            for error in &meta.validation_errors {
                ui.colored_label(egui::Color32::RED, error);
            }
            if meta.commit_state == CommitState::Submitting {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Submitting…");
                });
            }
            if meta.commit_state == CommitState::Failed {
                p.primary = "Retry";
            }
            ui.separator();
            ui.horizontal(|ui| {
                commit = ui
                    .add_enabled(
                        meta.commit_state != CommitState::Submitting,
                        egui::Button::new(p.primary),
                    )
                    .clicked();
                cancel = ui
                    .add_enabled(
                        meta.commit_state != CommitState::Submitting,
                        egui::Button::new("Cancel"),
                    )
                    .clicked();
            });
        }
    });
    // Backdrop and Escape use the same runtime cancellation policy as the button.
    cancel |= response.should_close();
    if commit {
        actions.push(crate::app::command::AppCommand::Commit);
    }
    if cancel {
        actions.push(crate::app::command::AppCommand::Cancel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transaction_editors_have_no_modal_presentation() {
        let e = crate::app::transaction_editor::TransactionEditorState::new(
            None,
            crate::app::state::EditorMetadata::new(egui::Id::new("x")),
        );
        assert_eq!(presentation(&EditorState::CreatingTransaction(e)), None);
    }
}
