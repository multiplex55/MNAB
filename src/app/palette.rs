//! Deterministic command-palette matching and guarded dispatch.
use super::command::AppCommand;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandDescriptor {
    pub command: AppCommand,
    pub title: String,
    pub keywords: Vec<String>,
    pub shortcut: Option<String>,
    pub enabled: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecuteError {
    Disabled,
    Ambiguous,
    NotFound,
}
#[must_use]
pub fn commands() -> Vec<CommandDescriptor> {
    use AppCommand::*;
    [
        (ContextualNew, "New transaction", "new transaction"),
        (AddAccount, "New account", "create account"),
        (PreviousMonth, "Previous month", "month back"),
        (NextMonth, "Next month", "month forward"),
        (Import, "Import transactions", "file csv ofx"),
        (Backup, "Back up budget", "backup"),
        (NavigateReports, "Open reports", "spending net worth"),
        (Settings, "Open settings", "preferences"),
        (NavigateAccounts, "Manage accounts", "accounts"),
        (NavigateBudget, "Manage categories", "categories budget"),
        (Commit, "Reconcile account", "reconciliation"),
    ]
    .into_iter()
    .map(|(command, title, keys)| CommandDescriptor {
        command,
        title: title.into(),
        keywords: keys.split(' ').map(str::to_owned).collect(),
        shortcut: shortcut(command).map(str::to_owned),
        enabled: true,
    })
    .collect()
}
#[must_use]
pub const fn shortcut(c: AppCommand) -> Option<&'static str> {
    use AppCommand::*;
    match c {
        ContextualNew => Some("Ctrl+N"),
        Import => Some("Ctrl+I"),
        FocusSearch => Some("Ctrl+F"),
        Undo => Some("Ctrl+Z"),
        Redo => Some("Ctrl+Shift+Z"),
        Commit => Some("Enter"),
        Cancel => Some("Esc"),
        Backup => Some("Ctrl+Shift+B"),
        PreviousMonth => Some("Alt+Left"),
        NextMonth => Some("Alt+Right"),
        Settings => Some("Ctrl+,"),
        _ => None,
    }
}
fn score(q: &str, d: &CommandDescriptor) -> Option<usize> {
    if q.trim().is_empty() {
        return Some(0);
    }
    let needle = q.to_lowercase();
    let hay = format!("{} {}", d.title, d.keywords.join(" ")).to_lowercase();
    let mut pos = 0;
    let mut score = 0;
    for ch in needle.chars().filter(|c| !c.is_whitespace()) {
        let found = hay[pos..].find(ch)?;
        score += found;
        pos += found + ch.len_utf8()
    }
    Some(score)
}
#[must_use]
pub fn fuzzy<'a>(query: &str, items: &'a [CommandDescriptor]) -> Vec<&'a CommandDescriptor> {
    let mut out = items
        .iter()
        .filter_map(|d| score(query, d).map(|s| (s, d)))
        .collect::<Vec<_>>();
    out.sort_by(|(a, x), (b, y)| {
        a.cmp(b)
            .then_with(|| x.title.cmp(&y.title))
            .then_with(|| format!("{:?}", x.command).cmp(&format!("{:?}", y.command)))
    });
    out.into_iter().map(|x| x.1).collect()
}
pub fn execute(query: &str, items: &[CommandDescriptor]) -> Result<AppCommand, ExecuteError> {
    let found = fuzzy(query, items);
    let Some(first) = found.first() else {
        return Err(ExecuteError::NotFound);
    };
    if !first.enabled {
        return Err(ExecuteError::Disabled);
    }
    let first_score = score(query, first);
    if found.get(1).is_some_and(|x| score(query, x) == first_score) {
        return Err(ExecuteError::Ambiguous);
    }
    Ok(first.command)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RowSelection<Id: Ord> {
    pub selected: std::collections::BTreeSet<Id>,
    pub hidden: std::collections::BTreeSet<Id>,
}
impl<Id: Ord + Clone> RowSelection<Id> {
    /// Opening a context menu is deliberately a no-op: selection and native text selection survive.
    pub const fn open_context_menu(&self) {}
    pub fn apply_visible(&mut self, visible: impl IntoIterator<Item = Id>) {
        let v = visible
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        self.hidden = self
            .selected
            .iter()
            .filter(|x| !v.contains(*x))
            .cloned()
            .collect()
    }
    pub fn command_replace(&mut self, ids: impl IntoIterator<Item = Id>) {
        self.selected = ids.into_iter().collect();
        self.hidden.clear()
    }
}
