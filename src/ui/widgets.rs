use std::collections::BTreeSet;

/// ID-based selection state. Visible positions are accepted only as transient input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionModel<Id: Ord + Copy> {
    selected: BTreeSet<Id>,
    pub focused: Option<Id>,
    pub editing: Option<Id>,
    pub range_anchor: Option<Id>,
    pub pending_batch_action: Option<String>,
}
impl<Id: Ord + Copy> Default for SelectionModel<Id> {
    fn default() -> Self {
        Self {
            selected: BTreeSet::new(),
            focused: None,
            editing: None,
            range_anchor: None,
            pending_batch_action: None,
        }
    }
}
impl<Id: Ord + Copy> SelectionModel<Id> {
    pub fn selected(&self) -> &BTreeSet<Id> {
        &self.selected
    }
    pub fn select_only(&mut self, id: Id) {
        self.selected.clear();
        self.selected.insert(id);
        self.focused = Some(id);
        self.range_anchor = Some(id);
    }
    pub fn toggle(&mut self, id: Id) {
        if !self.selected.remove(&id) {
            self.selected.insert(id);
        }
        self.focused = Some(id);
        self.range_anchor.get_or_insert(id);
    }
    pub fn select_range(&mut self, to: Id, visible: &[Id]) {
        let Some(anchor) = self.range_anchor else {
            self.select_only(to);
            return;
        };
        let (Some(a), Some(b)) = (
            visible.iter().position(|x| *x == anchor),
            visible.iter().position(|x| *x == to),
        ) else {
            self.select_only(to);
            return;
        };
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        self.selected.extend(visible[start..=end].iter().copied());
        self.focused = Some(to);
    }
    pub fn reconcile(&mut self, existing: &BTreeSet<Id>, visible: &[Id]) {
        self.selected.retain(|id| existing.contains(id));
        if self.focused.is_some_and(|id| !existing.contains(&id)) {
            self.focused = visible.iter().copied().find(|id| existing.contains(id));
        }
        if self.editing.is_some_and(|id| !existing.contains(&id)) {
            self.editing = None;
        }
        if self.range_anchor.is_some_and(|id| !existing.contains(&id)) {
            self.range_anchor = self.focused;
        }
        if self.selected.is_empty() {
            self.pending_batch_action = None;
        }
    }
}

/// Deterministically chooses the initiating widget, then its valid container.
pub fn restoration_target<Id: Copy>(
    initiating: Id,
    container: Id,
    exists: impl Fn(Id) -> bool,
) -> Id {
    if exists(initiating) {
        initiating
    } else {
        container
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_across_view_changes() {
        let mut s = SelectionModel::default();
        s.select_only(2);
        s.toggle(4);
        let all = BTreeSet::from([1, 2, 3, 4, 5]);
        s.reconcile(&all, &[5, 4, 3, 2, 1]);
        assert_eq!(s.selected(), &BTreeSet::from([2, 4]));
        s.reconcile(&all, &[1, 3, 5]);
        assert_eq!(s.selected(), &BTreeSet::from([2, 4]));
        let inserted = BTreeSet::from([0, 1, 2, 3, 4, 5]);
        s.reconcile(&inserted, &[0, 1, 2, 3, 4, 5]);
        assert_eq!(s.selected(), &BTreeSet::from([2, 4]));
        let deleted = BTreeSet::from([0, 1, 3, 5]);
        s.reconcile(&deleted, &[0, 1, 3, 5]);
        assert!(s.selected().is_empty());
    }
    #[test]
    fn ranges_use_visible_order_and_store_ids() {
        let mut s = SelectionModel::default();
        s.select_only(40);
        s.select_range(10, &[50, 40, 30, 10, 20]);
        assert_eq!(s.selected(), &BTreeSet::from([10, 30, 40]));
    }
    #[test]
    fn focus_restoration_falls_back() {
        assert_eq!(restoration_target(1, 9, |x| x == 1), 1);
        assert_eq!(restoration_target(1, 9, |_| false), 9);
    }
}
use crate::app::{
    command::{AppCommand, CommandAvailabilityContext},
    dispatcher::ActionCollector,
};

/// A semantic action button whose enabled state and explanation always come from the shared
/// command policy.
pub fn action_button(
    ui: &mut egui::Ui,
    label: &str,
    command: AppCommand,
    context: CommandAvailabilityContext,
    actions: &mut ActionCollector,
) -> egui::Response {
    let descriptor = crate::app::command::action_descriptor(context, command);
    let response = ui.add_enabled(descriptor.enabled, egui::Button::new(label));
    let response = if let Some(reason) = descriptor.disabled_reason {
        response.on_disabled_hover_text(reason)
    } else {
        response
    };
    if response.clicked() {
        actions.push(command);
    }
    response
}

/// The shared transaction-style date field. Parsing and validation belong to the
/// owning form; this widget only provides a consistent, accessible editor.
pub fn date_picker(ui: &mut egui::Ui, text: &mut String) -> egui::Response {
    ui.horizontal(|ui| {
        let response = ui.add(
            egui::TextEdit::singleline(text)
                .hint_text("MM/DD/YYYY")
                .desired_width(100.0),
        );
        if ui
            .small_button("📅")
            .on_hover_text("Open date picker (MM/DD/YYYY)")
            .clicked()
        {
            response.request_focus();
        }
        response
    })
    .inner
}
