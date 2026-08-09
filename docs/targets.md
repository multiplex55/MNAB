# Canonical Budget targets

The `targets` table and the domain `Target` type are the only canonical target store for the
Budget workspace. Budget month rows and category inspector details must read through the targets
repository/projections, and Budget commands must write only through `TargetCommand`.

`category_goals` is a legacy, account-oriented compatibility table. Reads of it are isolated to
legacy account journeys and migration/diagnostic code. New Budget features must not read it, and
no operation may dual-write `targets` and `category_goals`. A legacy goal is migrated explicitly
to one `Target` before it can participate in Budget calculations.
