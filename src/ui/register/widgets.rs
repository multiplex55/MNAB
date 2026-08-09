use crate::{
    app::state::AppState,
    domain::{CategoryId, PayeeId},
};

pub fn payee_name(state: &AppState, id: Option<PayeeId>) -> String {
    id.and_then(|id| {
        state
            .register_query
            .last_successful
            .as_ref()?
            .rows
            .iter()
            .find(|row| row.payee_id == Some(id))
            .map(|row| row.payee_name.clone())
    })
    .unwrap_or_else(|| "Choose a payee".into())
}
pub fn category_name(state: &AppState, id: Option<CategoryId>) -> String {
    id.and_then(|id| {
        state
            .category_catalog
            .last_successful
            .as_ref()?
            .groups
            .iter()
            .flat_map(|group| &group.categories)
            .find(|category| category.id == id)
            .map(|category| category.name.clone())
    })
    .or_else(|| {
        id.and_then(|id| {
            state
                .register_query
                .last_successful
                .as_ref()?
                .rows
                .iter()
                .find(|row| row.category_id == Some(id))
                .map(|row| row.category_name.clone())
        })
    })
    .unwrap_or_else(|| "Choose a category".into())
}
