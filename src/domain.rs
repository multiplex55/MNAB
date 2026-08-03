//! Core budgeting types. This module must remain independent of persistence and UI crates.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// An amount in integer USD cents.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Money(pub i64);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Budget {
    pub id: Uuid,
    pub name: String,
}
