//! Canonical, transient register selection semantics.
//!
//! Selection belongs to the application session, not preferences.  Every stored
//! position is a stable transaction identity; row offsets are accepted only while
//! interpreting an input event.

use crate::{
    app::view_model::{
        RegisterCursor, RegisterFilter, RegisterScope, RegisterSortDirection, RegisterSortField,
    },
    domain::TransactionId,
};
use std::collections::BTreeSet;

/// Immutable description of the semantic query selected by "select all".
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalQuery {
    pub scope: RegisterScope,
    pub filter: RegisterFilter,
    pub sort_field: RegisterSortField,
    pub sort_direction: RegisterSortDirection,
    /// Changes when the canonical interpretation of a query changes, not for a refresh.
    pub revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionSelection {
    Explicit {
        ids: BTreeSet<TransactionId>,
        anchor: Option<TransactionId>,
        cursor: Option<TransactionId>,
    },
    AllMatching {
        query: CanonicalQuery,
        exclusions: BTreeSet<TransactionId>,
        cursor: Option<TransactionId>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AllMatchingClick {
    /// A normal click abandons the query-wide selection and starts a new selection.
    StartExplicit,
    /// A command-modified click toggles the row's exclusion.
    ToggleExclusion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorMove {
    Moved(TransactionId),
    /// The caller must request the next page and retry after it arrives.
    RequestNextPage,
    AtBoundary,
}

impl Default for TransactionSelection {
    fn default() -> Self {
        Self::Explicit {
            ids: BTreeSet::new(),
            anchor: None,
            cursor: None,
        }
    }
}

impl TransactionSelection {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Explicit { ids, .. } if ids.is_empty())
    }
    #[must_use]
    pub fn contains(&self, id: TransactionId) -> bool {
        match self {
            Self::Explicit { ids, .. } => ids.contains(&id),
            Self::AllMatching { exclusions, .. } => !exclusions.contains(&id),
        }
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn select_only(&mut self, id: TransactionId) {
        *self = Self::Explicit {
            ids: BTreeSet::from([id]),
            anchor: Some(id),
            cursor: Some(id),
        };
    }
    pub fn toggle(&mut self, id: TransactionId, all_matching: AllMatchingClick) {
        match self {
            Self::Explicit {
                ids,
                anchor,
                cursor,
            } => {
                if !ids.remove(&id) {
                    ids.insert(id);
                }
                *cursor = Some(id);
                anchor.get_or_insert(id);
            }
            Self::AllMatching {
                exclusions, cursor, ..
            } if all_matching == AllMatchingClick::ToggleExclusion => {
                if !exclusions.remove(&id) {
                    exclusions.insert(id);
                }
                *cursor = Some(id);
            }
            Self::AllMatching { .. } => self.select_only(id),
        }
    }
    pub fn select_range(&mut self, target: TransactionId, ordered: &[TransactionId]) {
        let anchor = match self {
            Self::Explicit { anchor, .. } => anchor.unwrap_or(target),
            Self::AllMatching { .. } => {
                self.select_only(target);
                return;
            }
        };
        let (Some(a), Some(b)) = (
            ordered.iter().position(|id| *id == anchor),
            ordered.iter().position(|id| *id == target),
        ) else {
            self.select_only(target);
            return;
        };
        if let Self::Explicit { ids, cursor, .. } = self {
            ids.clear();
            ids.extend(ordered[a.min(b)..=a.max(b)].iter().copied());
            *cursor = Some(target);
        }
    }
    pub fn select_all(query: CanonicalQuery, cursor: Option<TransactionId>) -> Self {
        Self::AllMatching {
            query,
            exclusions: BTreeSet::new(),
            cursor,
        }
    }
    /// Same-query refresh retains valid explicit IDs. A semantic query change clears.
    pub fn apply_query_change(
        &mut self,
        previous: &CanonicalQuery,
        next: &CanonicalQuery,
        valid_ids: &BTreeSet<TransactionId>,
        restoring: bool,
    ) {
        if previous != next && !restoring {
            self.clear();
            return;
        }
        self.query_changed(next, valid_ids, restoring);
    }
    /// Reconciles a refresh when the caller has already established query identity.
    pub fn query_changed(
        &mut self,
        next: &CanonicalQuery,
        valid_ids: &BTreeSet<TransactionId>,
        restoring: bool,
    ) {
        let same = match self {
            Self::AllMatching { query, .. } => query == next,
            Self::Explicit { .. } => true,
        };
        if !same && !restoring {
            self.clear();
            return;
        }
        if let Self::Explicit {
            ids,
            anchor,
            cursor,
        } = self
        {
            ids.retain(|id| valid_ids.contains(id));
            if anchor.is_some_and(|id| !valid_ids.contains(&id)) {
                *anchor = None;
            }
            if cursor.is_some_and(|id| !valid_ids.contains(&id)) {
                *cursor = None;
            }
        }
    }
    pub fn move_cursor(
        &mut self,
        loaded: &[TransactionId],
        delta: isize,
        extend: bool,
        has_more: bool,
    ) -> CursorMove {
        if loaded.is_empty() {
            return if has_more {
                CursorMove::RequestNextPage
            } else {
                CursorMove::AtBoundary
            };
        }
        let current_id = match self {
            Self::Explicit { cursor, .. } | Self::AllMatching { cursor, .. } => *cursor,
        };
        let current = current_id
            .and_then(|id| loaded.iter().position(|x| *x == id))
            .unwrap_or(0);
        if delta > 0 && current == loaded.len() - 1 {
            return if has_more {
                CursorMove::RequestNextPage
            } else {
                CursorMove::AtBoundary
            };
        }
        let next = current.saturating_add_signed(delta).min(loaded.len() - 1);
        let id = loaded[next];
        if extend {
            self.select_range(id, loaded);
        } else {
            self.select_only(id);
        }
        CursorMove::Moved(id)
    }
    #[must_use]
    pub fn cursor(&self) -> Option<TransactionId> {
        match self {
            Self::Explicit { cursor, .. } | Self::AllMatching { cursor, .. } => *cursor,
        }
    }
}

/// A summary request can be evaluated by SQL from a query snapshot and exclusions,
/// without materialising every transaction identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionSummaryRequest {
    pub selection: TransactionSelection,
    pub page_cursor: Option<RegisterCursor>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AccountId;
    fn query(scope: RegisterScope, revision: u64) -> CanonicalQuery {
        CanonicalQuery {
            scope,
            filter: RegisterFilter::default(),
            sort_field: RegisterSortField::Date,
            sort_direction: RegisterSortDirection::Ascending,
            revision,
        }
    }
    #[test]
    fn ranges_and_exclusions_store_ids() {
        let ids: Vec<_> = (0..5).map(|_| TransactionId::new()).collect();
        let mut s = TransactionSelection::default();
        s.select_only(ids[1]);
        s.select_range(ids[3], &ids);
        assert!(ids[1..=3].iter().all(|id| s.contains(*id)));
        s = TransactionSelection::select_all(
            query(RegisterScope::AllTransactions, 1),
            Some(ids[0]),
        );
        s.toggle(ids[2], AllMatchingClick::ToggleExclusion);
        assert!(!s.contains(ids[2]));
        s.toggle(ids[2], AllMatchingClick::ToggleExclusion);
        assert!(s.contains(ids[2]));
    }
    #[test]
    fn refresh_preserves_but_query_change_clears() {
        let id = TransactionId::new();
        let q = query(RegisterScope::AllTransactions, 1);
        let mut s = TransactionSelection::select_all(q.clone(), Some(id));
        s.query_changed(&q, &BTreeSet::from([id]), false);
        assert!(s.contains(id));
        s.query_changed(
            &query(RegisterScope::Account(AccountId::new()), 2),
            &BTreeSet::new(),
            false,
        );
        assert!(s.is_empty());
        let mut explicit = TransactionSelection::default();
        explicit.select_only(id);
        explicit.apply_query_change(
            &q,
            &query(RegisterScope::Account(AccountId::new()), 2),
            &BTreeSet::from([id]),
            false,
        );
        assert!(explicit.is_empty());
    }
    #[test]
    fn page_boundary_requests_more_before_moving() {
        let id = TransactionId::new();
        let mut s = TransactionSelection::default();
        s.select_only(id);
        assert_eq!(
            s.move_cursor(&[id], 1, false, true),
            CursorMove::RequestNextPage
        );
        let next = TransactionId::new();
        assert_eq!(
            s.move_cursor(&[id, next], 1, false, false),
            CursorMove::Moved(next)
        );
    }
}
