use crate::{
    app::state::AppState,
    domain::{CategoryId, PayeeId},
};

/// Framework-independent state shared by payee and category autocomplete controls.
/// `selected` is deliberately separate from the editable display query, so an
/// Escape can discard the popup interaction without corrupting a stable domain ID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PickerState<Id> {
    pub selected: Option<Id>,
    pub query: String,
    pub open: bool,
    pub active: usize,
    snapshot: Option<Id>,
}
impl<Id: Copy> PickerState<Id> {
    pub fn new(selected: Option<Id>, display: impl Into<String>) -> Self {
        Self {
            selected,
            query: display.into(),
            open: false,
            active: 0,
            snapshot: selected,
        }
    }
    pub fn open(&mut self) {
        self.snapshot = self.selected;
        self.open = true;
        self.active = 0;
    }
    pub fn move_active(&mut self, delta: isize, count: usize) {
        if count > 0 {
            self.active = self.active.saturating_add_signed(delta).min(count - 1);
        }
    }
    pub fn accept(&mut self, choices: &[PickerChoice<Id>]) -> bool {
        let Some(choice) = choices.get(self.active) else {
            return false;
        };
        self.selected = Some(choice.id);
        self.query = choice.text.clone();
        self.open = false;
        true
    }
    /// Returns true when Escape was consumed by this popup.
    pub fn escape(&mut self) -> bool {
        if !self.open {
            return false;
        }
        self.selected = self.snapshot;
        self.open = false;
        true
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PickerChoice<Id> {
    pub id: Id,
    pub text: String,
    pub group: Option<String>,
    /// Presentation metadata only: transfer construction remains register orchestration's job.
    pub transfer: bool,
    pub valid: bool,
}

#[must_use]
pub fn filter_choices<Id: Copy>(
    choices: &[PickerChoice<Id>],
    query: &str,
) -> Vec<PickerChoice<Id>> {
    let needle = query.trim().to_lowercase();
    choices
        .iter()
        .filter(|choice| {
            choice.valid
                && (needle.is_empty()
                    || choice.text.to_lowercase().contains(&needle)
                    || choice
                        .group
                        .as_ref()
                        .is_some_and(|group| group.to_lowercase().contains(&needle)))
        })
        .cloned()
        .collect()
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(id: PayeeId, text: &str, group: Option<&str>, valid: bool) -> PickerChoice<PayeeId> {
        PickerChoice {
            id,
            text: text.into(),
            group: group.map(str::to_owned),
            transfer: false,
            valid,
        }
    }

    #[test]
    fn filtering_preserves_source_order_group_context_and_validity() {
        let a = PayeeId::new();
        let b = PayeeId::new();
        let c = PayeeId::new();
        let choices = [
            choice(a, "Grocer", Some("Everyday"), true),
            choice(b, "Rent", Some("Bills"), true),
            choice(c, "Old rent", Some("Bills"), false),
        ];
        assert_eq!(
            filter_choices(&choices, "bills")
                .iter()
                .map(|x| x.id)
                .collect::<Vec<_>>(),
            vec![b]
        );
    }

    #[test]
    fn acceptance_uses_stable_id_and_escape_restores_selection() {
        let a = PayeeId::new();
        let b = PayeeId::new();
        let choices = [choice(a, "Same", None, true), choice(b, "Same", None, true)];
        let mut picker = PickerState::new(Some(a), "Same");
        picker.open();
        picker.move_active(1, choices.len());
        assert!(picker.accept(&choices));
        assert_eq!(picker.selected, Some(b));
        picker.open();
        picker.selected = Some(a);
        assert!(picker.escape());
        assert_eq!(picker.selected, Some(b));
    }
}
