use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! ids {
    ($($name:ident),+ $(,)?) => {$ (
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            #[must_use] pub fn new() -> Self { Self(Uuid::new_v4()) }
            #[must_use] pub const fn from_uuid(value: Uuid) -> Self { Self(value) }
            #[must_use] pub const fn as_uuid(self) -> Uuid { self.0 }
        }
        impl Default for $name { fn default() -> Self { Self::new() } }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.0.fmt(f) }
        }
        impl std::str::FromStr for $name {
            type Err = uuid::Error;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                value.parse().map(Self)
            }
        }
    )+};
}

ids!(
    BudgetId,
    AccountId,
    CategoryGroupId,
    CategoryId,
    PayeeId,
    TransactionId,
    TransferId,
    ReconciliationId,
    ImportBatchId,
    TargetId,
    ScheduledTransactionId,
    ScheduledOccurrenceId
);
