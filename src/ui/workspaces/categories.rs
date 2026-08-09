use crate::app::{
    command::{
        ApplicationAction, CategoryAction, CategoryCommand, FinancialCommand, TargetCommand,
    },
    dispatcher::ActionCollector,
    state::{AppState, CategoryEditorMode, EditorState},
    view_model::DisplayMoney,
};
use crate::domain::{CategoryId, TargetAssociation};

/// Category management is month independent; values shown here come from the correlated worker
/// projections and remain visible during refresh failures.
pub fn show(ui: &mut egui::Ui, state: &mut AppState, actions: &mut ActionCollector) {
    ui.heading("Categories");
    ui.horizontal(|ui| {
        if ui.button("New group").clicked() {
            actions.push(ApplicationAction::Category(CategoryAction::NewGroup));
        }
        let first_group = state
            .category_catalog
            .last_successful
            .as_ref()
            .and_then(|v| v.groups.first())
            .map(|v| v.id);
        if ui
            .add_enabled(first_group.is_some(), egui::Button::new("New category"))
            .clicked()
        {
            actions.push(ApplicationAction::Category(CategoryAction::NewCategory(
                first_group.expect("enabled"),
            )));
        }
        let mut archived = state.show_archived_categories;
        if ui.checkbox(&mut archived, "Show archived").changed() {
            actions.push(ApplicationAction::Category(CategoryAction::ToggleArchived(
                archived,
            )));
        }
        if ui.button("Refresh").clicked() {
            actions.push(ApplicationAction::Category(CategoryAction::RefreshCatalog));
        }
    });
    if state.category_catalog.refresh_active {
        ui.spinner();
    }
    if let Some(error) = &state.category_catalog.safe_failure {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    ui.separator();
    ui.columns(2, |columns| {
        columns[0].vertical(|ui| show_catalog(ui, state, actions));
        columns[1].vertical(|ui| show_detail(ui, state, actions));
    });
    show_editor(ui, state, actions);
}

fn show_catalog(ui: &mut egui::Ui, state: &AppState, actions: &mut ActionCollector) {
    let Some(catalog) = &state.category_catalog.last_successful else {
        ui.label("Loading category catalog…");
        return;
    };
    for group in &catalog.groups {
        ui.strong(&group.name);
        for category in &group.categories {
            let suffix = if category.archived {
                " (archived)"
            } else if category.hidden {
                " (hidden)"
            } else {
                ""
            };
            if ui
                .selectable_label(
                    state.selected_category == Some(category.id),
                    format!("{}{}", category.name, suffix),
                )
                .clicked()
            {
                actions.push(ApplicationAction::Category(CategoryAction::Select(
                    category.id,
                )));
            }
        }
    }
}
fn show_detail(ui: &mut egui::Ui, state: &AppState, actions: &mut ActionCollector) {
    ui.strong("Category details");
    if state.category_detail.refresh_active {
        ui.spinner();
    }
    if let Some(error) = &state.category_detail.safe_failure {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    let Some(detail) = &state.category_detail.last_successful else {
        ui.label("Select a category.");
        return;
    };
    ui.heading(&detail.category.name);
    ui.label(format!("Group: {}", detail.group_name));
    ui.horizontal_wrapped(|ui| {
        if ui.button("Rename").clicked() {
            actions.push(ApplicationAction::Category(CategoryAction::Edit(
                detail.category.id,
            )));
        }
        if ui.button("Move / reorder").clicked() {
            actions.push(ApplicationAction::Category(CategoryAction::Edit(
                detail.category.id,
            )));
        }
        if ui
            .button(if detail.category.hidden {
                "Unhide"
            } else {
                "Hide"
            })
            .clicked()
        {
            let mut c = crate::domain::Category {
                id: detail.category.id,
                group_id: detail.category.group_id,
                name: detail.category.name.clone(),
                hidden: !detail.category.hidden,
                archived: detail.category.archived,
            };
            actions.push(ApplicationAction::Financial(FinancialCommand::Category(
                CategoryCommand::Update(c.clone()),
            )));
        }
        let archive = ui.add_enabled(!detail.category.protected, egui::Button::new("Archive"));
        if archive.clicked() {
            let mut c = crate::domain::Category {
                id: detail.category.id,
                group_id: detail.category.group_id,
                name: detail.category.name.clone(),
                hidden: true,
                archived: true,
            };
            c.archive();
            actions.push(ApplicationAction::Financial(FinancialCommand::Category(
                CategoryCommand::Update(c),
            )));
        }
        if ui
            .add_enabled(!detail.category.protected, egui::Button::new("Merge"))
            .clicked()
        {
            actions.push(ApplicationAction::Category(CategoryAction::Edit(
                detail.category.id,
            )));
        }
    });
    ui.separator();
    ui.strong("Goal");
    egui::Grid::new("category-goal-summary").show(ui, |ui| {
        ui.label("Target amount");
        ui.label(
            detail
                .target_cents
                .map_or_else(|| "No goal".into(), |v| DisplayMoney::usd(v).text),
        );
        ui.end_row();
        ui.label("Target date");
        ui.label(
            detail
                .due_date
                .map_or_else(|| "No date".into(), crate::ui::format::date),
        );
        ui.end_row();
        ui.label("Current / remaining");
        ui.label(format!(
            "{} / {}",
            DisplayMoney::usd(detail.current_cents).text,
            detail
                .remaining_cents
                .map_or_else(|| "—".into(), |v| DisplayMoney::usd(v).text)
        ));
        ui.end_row();
    });
    ui.horizontal_wrapped(|ui| {
        if ui
            .button(if detail.target.is_some() {
                "Edit goal"
            } else {
                "Create goal"
            })
            .clicked()
        {
            actions.push(ApplicationAction::Category(CategoryAction::BeginGoal(
                detail.category.id,
            )));
        }
        let remove = ui.add_enabled(detail.target.is_some(), egui::Button::new("Remove goal"));
        if remove.clicked() {
            actions.push(ApplicationAction::Financial(FinancialCommand::Target(
                TargetCommand::Delete(detail.target.as_ref().expect("enabled").id),
            )));
        }
        if ui.button("View activity").clicked() {
            actions.push(ApplicationAction::Category(CategoryAction::OpenActivity(
                detail.category.id,
            )));
        }
        if ui.button("Open transactions").clicked() {
            actions.push(ApplicationAction::Category(
                CategoryAction::OpenTransactions(detail.category.id),
            ));
        }
        if ui
            .add_enabled(
                matches!(
                    detail.target.as_ref().map(|t| t.association),
                    Some(TargetAssociation::CreditCard { .. })
                ),
                egui::Button::new("Transfer to goal account"),
            )
            .clicked()
        {
            actions.push(ApplicationAction::Category(
                CategoryAction::BeginGoalTransfer(detail.category.id),
            ));
        }
    });
}
fn show_editor(ui: &mut egui::Ui, state: &mut AppState, actions: &mut ActionCollector) {
    let EditorState::ManagingCategory(editor) = &mut state.editor else {
        return;
    };
    ui.separator();
    ui.strong(match editor.mode {
        CategoryEditorMode::Group => "Category group",
        CategoryEditorMode::Category => "Category",
        CategoryEditorMode::Goal => "Goal",
    });
    let response = ui.add(
        egui::TextEdit::singleline(&mut editor.name).id(egui::Id::new("category-editor-name")),
    );
    for error in &editor.metadata.validation_errors {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    if ui.button("Save").clicked() {
        if editor.name.trim().is_empty() {
            editor.metadata.validation_errors = vec!["Name is required".into()];
            response.request_focus();
        } else {
            actions.push(crate::app::command::AppCommand::Commit);
        }
    }
    if ui.button("Cancel").clicked() {
        actions.push(crate::app::command::AppCommand::Cancel);
    }
}

#[must_use]
pub fn canonical_category_filter(
    budget_id: crate::domain::BudgetId,
    category_id: CategoryId,
) -> crate::app::view_model::RegisterRequest {
    crate::app::view_model::RegisterRequest {
        budget_id,
        scope: crate::app::view_model::RegisterScope::AllTransactions,
        filter: crate::app::view_model::RegisterFilter {
            category_ids: vec![category_id],
            ..Default::default()
        },
        sort_field: crate::app::view_model::RegisterSortField::Date,
        sort_direction: crate::app::view_model::RegisterSortDirection::Descending,
        page_size: 100,
        cursor: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn open_transactions_is_the_exact_canonical_category_filter() {
        let budget = crate::domain::BudgetId::new();
        let category = CategoryId::new();
        let request = canonical_category_filter(budget, category);
        assert_eq!(
            request.scope,
            crate::app::view_model::RegisterScope::AllTransactions
        );
        assert_eq!(request.filter.category_ids, vec![category]);
        assert!(request.filter.search.is_empty());
        assert!(request.filter.payee_ids.is_empty());
    }
    #[test]
    fn category_detail_refresh_rejects_stale_and_retains_success_on_failure() {
        use crate::storage::worker::Generation;
        let mut query = crate::app::state::ViewQueryState::default();
        let generation = Generation { budget: 1, view: 2 };
        query.begin(10, generation, None);
        assert!(query.accept(10, generation, "detail"));
        query.begin(11, generation, None);
        assert!(!query.accept(10, generation, "stale"));
        assert!(query.fail(11, generation, "safe failure"));
        assert_eq!(query.last_successful, Some("detail"));
    }
}
