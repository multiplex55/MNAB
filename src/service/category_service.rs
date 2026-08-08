//! Transaction-friendly category organization commands.

use crate::domain::{AccountId, CategoryGroupId, CategoryId};
use std::collections::HashMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Group {
    pub id: CategoryGroupId,
    pub name: String,
    pub position: usize,
    pub collapsed: bool,
    pub hidden: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Category {
    pub id: CategoryId,
    pub group_id: CategoryGroupId,
    pub name: String,
    pub position: usize,
    pub hidden: bool,
    pub archived: bool,
    pub historically_used: bool,
    pub usage: CategoryUsage,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CategoryUsage {
    pub transactions: bool,
    pub splits: bool,
    pub assignments: bool,
    pub targets: bool,
    pub schedules: bool,
    pub import_mappings: bool,
    pub reconciliation_history: bool,
}
impl CategoryUsage {
    #[must_use]
    pub const fn any(self) -> bool {
        self.transactions
            || self.splits
            || self.assignments
            || self.targets
            || self.schedules
            || self.import_mappings
            || self.reconciliation_history
    }

    /// Number of distinct historical/reference sources using this category.
    #[must_use]
    pub const fn source_count(self) -> usize {
        self.transactions as usize
            + self.splits as usize
            + self.assignments as usize
            + self.targets as usize
            + self.schedules as usize
            + self.import_mappings as usize
            + self.reconciliation_history as usize
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CategoryCatalog {
    pub groups: Vec<Group>,
    pub categories: Vec<Category>,
    /// Immutable account/category linkage for system-managed card payment categories.
    pub managed_payment_categories: HashMap<AccountId, CategoryId>,
}
#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum CategoryCommandError {
    #[error("item not found")]
    NotFound,
    #[error("name must not be empty")]
    EmptyName,
    #[error("category is historically used and was archived")]
    Archived,
    #[error("credit-card payment categories are managed with their account")]
    Managed,
}

impl CategoryCatalog {
    /// Archives `source` and returns the ID all persistent references must be
    /// retargeted to. Persistence performs those updates atomically; the source
    /// row remains so snapshots and audit history can still resolve it.
    pub fn merge(
        &mut self,
        source: CategoryId,
        destination: CategoryId,
    ) -> Result<CategoryId, CategoryCommandError> {
        if source == destination
            || !self
                .categories
                .iter()
                .any(|category| category.id == destination)
        {
            return Err(CategoryCommandError::NotFound);
        }
        if self.is_managed_payment_category(source) {
            return Err(CategoryCommandError::Managed);
        }
        self.archive(source)?;
        Ok(destination)
    }
    #[must_use]
    pub fn is_managed_payment_category(&self, id: CategoryId) -> bool {
        self.managed_payment_categories
            .values()
            .any(|managed| *managed == id)
    }

    pub fn relink_managed_payment_category(
        &mut self,
        account_id: AccountId,
        category_id: CategoryId,
    ) -> Result<(), CategoryCommandError> {
        if !self.categories.iter().any(|c| c.id == category_id) {
            return Err(CategoryCommandError::NotFound);
        }
        self.managed_payment_categories
            .insert(account_id, category_id);
        Ok(())
    }

    /// Resolves names independently of visibility so historical rows remain intelligible.
    #[must_use]
    pub fn resolve_name(&self, id: CategoryId) -> Option<&str> {
        self.categories
            .iter()
            .find(|category| category.id == id)
            .map(|category| category.name.as_str())
    }

    pub fn archive(&mut self, id: CategoryId) -> Result<(), CategoryCommandError> {
        let category = self
            .categories
            .iter_mut()
            .find(|category| category.id == id)
            .ok_or(CategoryCommandError::NotFound)?;
        category.archived = true;
        category.hidden = true;
        Ok(())
    }

    /// ID-based ordering for drag/drop and keyboard movement.
    pub fn reorder_group_before(
        &mut self,
        id: CategoryGroupId,
        before: Option<CategoryGroupId>,
    ) -> Result<(), CategoryCommandError> {
        let old = self
            .groups
            .iter()
            .position(|group| group.id == id)
            .ok_or(CategoryCommandError::NotFound)?;
        let item = self.groups.remove(old);
        let destination = before.map_or(self.groups.len(), |target| {
            self.groups
                .iter()
                .position(|group| group.id == target)
                .unwrap_or(self.groups.len())
        });
        self.groups.insert(destination, item);
        self.normalize();
        Ok(())
    }
    fn normalize(&mut self) {
        self.groups.sort_by_key(|g| g.position);
        for (p, g) in self.groups.iter_mut().enumerate() {
            g.position = p;
        }
        for group in &self.groups {
            let mut ids: Vec<_> = self
                .categories
                .iter()
                .enumerate()
                .filter(|(_, c)| c.group_id == group.id)
                .map(|(i, c)| (i, c.position))
                .collect();
            ids.sort_by_key(|v| v.1);
            for (p, (i, _)) in ids.into_iter().enumerate() {
                self.categories[i].position = p;
            }
        }
    }
    pub fn add_group(&mut self, name: &str) -> Result<CategoryGroupId, CategoryCommandError> {
        if name.trim().is_empty() {
            return Err(CategoryCommandError::EmptyName);
        }
        let id = CategoryGroupId::new();
        self.groups.push(Group {
            id,
            name: name.trim().into(),
            position: self.groups.len(),
            collapsed: false,
            hidden: false,
        });
        Ok(id)
    }
    pub fn rename_group(
        &mut self,
        id: CategoryGroupId,
        name: &str,
    ) -> Result<(), CategoryCommandError> {
        if name.trim().is_empty() {
            return Err(CategoryCommandError::EmptyName);
        }
        self.groups
            .iter_mut()
            .find(|g| g.id == id)
            .ok_or(CategoryCommandError::NotFound)?
            .name = name.trim().into();
        Ok(())
    }
    pub fn reorder_group(
        &mut self,
        id: CategoryGroupId,
        position: usize,
    ) -> Result<(), CategoryCommandError> {
        let old = self
            .groups
            .iter()
            .position(|g| g.id == id)
            .ok_or(CategoryCommandError::NotFound)?;
        let item = self.groups.remove(old);
        self.groups.insert(position.min(self.groups.len()), item);
        for (p, g) in self.groups.iter_mut().enumerate() {
            g.position = p;
        }
        Ok(())
    }
    pub fn set_collapsed(
        &mut self,
        id: CategoryGroupId,
        value: bool,
    ) -> Result<(), CategoryCommandError> {
        self.groups
            .iter_mut()
            .find(|g| g.id == id)
            .ok_or(CategoryCommandError::NotFound)?
            .collapsed = value;
        Ok(())
    }
    pub fn add_category(
        &mut self,
        group_id: CategoryGroupId,
        name: &str,
    ) -> Result<CategoryId, CategoryCommandError> {
        if name.trim().is_empty() {
            return Err(CategoryCommandError::EmptyName);
        }
        if !self.groups.iter().any(|g| g.id == group_id) {
            return Err(CategoryCommandError::NotFound);
        }
        let id = CategoryId::new();
        let position = self
            .categories
            .iter()
            .filter(|c| c.group_id == group_id)
            .count();
        self.categories.push(Category {
            id,
            group_id,
            name: name.trim().into(),
            position,
            hidden: false,
            archived: false,
            historically_used: false,
            usage: CategoryUsage::default(),
        });
        Ok(id)
    }
    pub fn rename_category(
        &mut self,
        id: CategoryId,
        name: &str,
    ) -> Result<(), CategoryCommandError> {
        if self.is_managed_payment_category(id) {
            return Err(CategoryCommandError::Managed);
        }
        if name.trim().is_empty() {
            return Err(CategoryCommandError::EmptyName);
        }
        self.categories
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or(CategoryCommandError::NotFound)?
            .name = name.trim().into();
        Ok(())
    }
    pub fn move_category(
        &mut self,
        id: CategoryId,
        group_id: CategoryGroupId,
        position: usize,
    ) -> Result<(), CategoryCommandError> {
        if self.is_managed_payment_category(id) {
            return Err(CategoryCommandError::Managed);
        }
        if !self.groups.iter().any(|g| g.id == group_id) {
            return Err(CategoryCommandError::NotFound);
        }
        let c = self
            .categories
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or(CategoryCommandError::NotFound)?;
        c.group_id = group_id;
        c.position = position;
        self.normalize();
        Ok(())
    }
    pub fn set_hidden(&mut self, id: CategoryId, value: bool) -> Result<(), CategoryCommandError> {
        if self.is_managed_payment_category(id) {
            return Err(CategoryCommandError::Managed);
        }
        self.categories
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or(CategoryCommandError::NotFound)?
            .hidden = value;
        Ok(())
    }
    pub fn delete_if_unused(&mut self, id: CategoryId) -> Result<(), CategoryCommandError> {
        if self.is_managed_payment_category(id) {
            return Err(CategoryCommandError::Managed);
        }
        let i = self
            .categories
            .iter()
            .position(|c| c.id == id)
            .ok_or(CategoryCommandError::NotFound)?;
        if self.categories[i].historically_used || self.categories[i].usage.any() {
            self.categories[i].archived = true;
            self.categories[i].hidden = true;
            return Err(CategoryCommandError::Archived);
        }
        self.categories.remove(i);
        self.normalize();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn moves_normalize_and_history_archives() {
        let mut catalog = CategoryCatalog::default();
        let first_group = catalog.add_group("A").unwrap();
        let second_group = catalog.add_group("B").unwrap();
        let unused = catalog.add_category(first_group, "x").unwrap();
        let historic = catalog.add_category(first_group, "y").unwrap();
        catalog.move_category(unused, second_group, 99).unwrap();
        assert_eq!(
            catalog
                .categories
                .iter()
                .find(|v| v.id == unused)
                .unwrap()
                .position,
            0
        );
        catalog
            .categories
            .iter_mut()
            .find(|v| v.id == historic)
            .unwrap()
            .historically_used = true;
        assert_eq!(
            catalog.delete_if_unused(historic),
            Err(CategoryCommandError::Archived)
        );
        assert!(
            catalog
                .categories
                .iter()
                .find(|v| v.id == historic)
                .unwrap()
                .archived
        );
        catalog.delete_if_unused(unused).unwrap();
        assert!(!catalog.categories.iter().any(|v| v.id == unused));
    }

    #[test]
    fn merge_archives_source_and_preserves_destination() {
        let mut catalog = CategoryCatalog::default();
        let group = catalog.add_group("Living").unwrap();
        let old = catalog.add_category(group, "Old rent").unwrap();
        let current = catalog.add_category(group, "Rent").unwrap();
        assert_eq!(catalog.merge(old, current), Ok(current));
        assert!(
            catalog
                .categories
                .iter()
                .find(|c| c.id == old)
                .unwrap()
                .archived
        );
        assert!(
            !catalog
                .categories
                .iter()
                .find(|c| c.id == current)
                .unwrap()
                .archived
        );
    }
}
