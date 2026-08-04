use crate::domain::{AccountId, BudgetMonth};

/// Supplies the local calendar month without coupling navigation tests to wall time.
pub trait Clock {
    fn local_month(&self) -> BudgetMonth;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn local_month(&self) -> BudgetMonth {
        // A UTC fallback is preferable to preventing startup on platforms which do not expose
        // their local offset (for example, some sandboxed Unix processes).
        let now =
            time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        BudgetMonth::new(now.year(), u8::from(now.month())).expect("calendar month is valid")
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Workspace {
    #[default]
    Budget,
    Reports,
    AllAccounts,
    Inbox,
    Account(AccountId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Navigation {
    pub workspace: Workspace,
    pub month: BudgetMonth,
}
impl Default for Navigation {
    fn default() -> Self {
        Self::with_clock(&SystemClock)
    }
}
impl Navigation {
    #[must_use]
    pub fn with_clock(clock: &impl Clock) -> Self {
        Self {
            workspace: Workspace::Budget,
            month: clock.local_month(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FixedClock(BudgetMonth);
    impl Clock for FixedClock {
        fn local_month(&self) -> BudgetMonth {
            self.0
        }
    }
    #[test]
    fn default_month_is_injected() {
        let expected = BudgetMonth::new(2042, 9).unwrap();
        assert_eq!(
            Navigation::with_clock(&FixedClock(expected)).month,
            expected
        );
    }
}
