# Category workspace operation inventory

The category workspace is a projection over the persisted category, target, assignment, and
register models. It does not own accounting state.

## Safely supported operations

* `CategoryCatalog` already supports group/category creation, rename, group and category reorder,
  moving a category, hiding, archive-on-delete for historically used categories, and merge with a
  persistent source row. Managed credit-card payment categories are protected by the service.
* Persisted `FinancialCommand::Category::Update/Delete/CreateGroup` supports create, rename, move,
  hide, archive, and unused deletion. The richer reorder and merge variants are typed intents but
  are rejected by persistence until their existing service semantics can be performed atomically.
* `Target::new/edit/recommend`, `TargetCommand::Save/Delete`, and `AssignmentCommand::Set` support
  goal validation, integer-minor-unit recommendations, persistence, deletion, and category funding.
* The canonical register supports an exact `RegisterFilter::category_ids` filter, used by **Open
  transactions**. Credit-card payoff targets may enter the existing account transfer editor.

## Deliberately not claimed by UI copy

There is no safe backend implementation for an arbitrary “goal account” transfer for ordinary
category targets: those targets have no account association. Likewise, merge/reorder UI must not
pretend success until the repository can atomically retarget every reference and persist ordering.
The controls may open a typed editor, but no egui callback mutates those records directly.

Category mutations target catalog/detail, materialized register labels, reports, targets,
inspectors, lookup/search data, and saved-view diagnostics. They do not trigger a universal
application reload.
