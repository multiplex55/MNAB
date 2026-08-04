//! Deterministic command-palette matching, context gating, and focus lifecycle.
use super::command::AppCommand;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequiredContext {
    Always,
    BudgetOpen,
    AccountRegister,
    BudgetWorkspace,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CommandContext {
    pub budget_open: bool,
    pub account_register: bool,
    pub budget_workspace: bool,
    pub mutations_disabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandDescriptor {
    pub command: AppCommand,
    pub title: String,
    pub keywords: Vec<String>,
    pub shortcut: Option<String>,
    pub required_context: RequiredContext,
    pub enabled: bool,
    pub disabled_explanation: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecuteError {
    Disabled(String),
    Ambiguous,
    NotFound,
}

fn available(
    required: RequiredContext,
    context: CommandContext,
    command: AppCommand,
) -> (bool, Option<String>) {
    let mut explanation = match required {
        RequiredContext::Always => None,
        RequiredContext::BudgetOpen if !context.budget_open => Some("Open a budget first"),
        RequiredContext::AccountRegister if !context.account_register => {
            Some("Open an account register first")
        }
        RequiredContext::BudgetWorkspace if !context.budget_workspace => {
            Some("Open the budget workspace first")
        }
        _ => None,
    };
    if explanation.is_none() && context.mutations_disabled && mutation_command(command) {
        explanation = Some("Mutations are disabled while the budget is in recovery mode");
    }
    (explanation.is_none(), explanation.map(str::to_owned))
}

/// The allowlist intentionally contains only commands with a working dispatch path.
/// Backup, retry, and operation cancellation stay hidden until implemented end-to-end.
#[must_use]
pub fn commands_for(context: CommandContext) -> Vec<CommandDescriptor> {
    use AppCommand::*;
    use RequiredContext::*;
    [
        (
            ContextualNew,
            "New transaction",
            "new add transaction",
            AccountRegister,
        ),
        (AddAccount, "New account", "create add account", BudgetOpen),
        (
            PreviousMonth,
            "Previous month",
            "budget month back",
            BudgetWorkspace,
        ),
        (
            NextMonth,
            "Next month",
            "budget month forward",
            BudgetWorkspace,
        ),
        (
            Import,
            "Import transactions",
            "file csv ofx import",
            AccountRegister,
        ),
        (
            NavigateReports,
            "Open reports",
            "spending net worth reports",
            BudgetOpen,
        ),
        (Settings, "Open settings", "preferences appearance", Always),
        (
            NavigateAccounts,
            "Manage accounts",
            "accounts register",
            BudgetOpen,
        ),
        (
            NavigateBudget,
            "Manage categories",
            "categories budget",
            BudgetOpen,
        ),
        (
            ToggleInspector,
            "Toggle inspector",
            "details sidebar",
            Always,
        ),
    ]
    .into_iter()
    .map(|(command, title, keys, required_context)| {
        let (enabled, disabled_explanation) = available(required_context, context, command);
        CommandDescriptor {
            command,
            title: title.into(),
            keywords: keys.split(' ').map(str::to_owned).collect(),
            shortcut: shortcut(command).map(str::to_owned),
            required_context,
            enabled,
            disabled_explanation,
        }
    })
    .collect()
}

/// Descriptors for an application with an open budget. Prefer [`commands_for`]
/// when the current workspace is available.
#[must_use]
fn mutation_command(command: AppCommand) -> bool {
    matches!(
        command,
        AppCommand::ContextualNew
            | AppCommand::AddAccount
            | AppCommand::Import
            | AppCommand::Undo
            | AppCommand::Redo
            | AppCommand::Commit
            | AppCommand::Delete
            | AppCommand::Rename
    )
}

pub fn commands() -> Vec<CommandDescriptor> {
    commands_for(CommandContext {
        budget_open: true,
        account_register: false,
        budget_workspace: true,
        mutations_disabled: false,
    })
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
        PreviousMonth => Some("Ctrl+Left"),
        NextMonth => Some("Ctrl+Right"),
        Settings => Some("Ctrl+,"),
        ToggleInspector => Some("Ctrl+\\"),
        _ => None,
    }
}
fn score(q: &str, d: &CommandDescriptor) -> Option<usize> {
    let needle = q.to_lowercase();
    if needle.trim().is_empty() {
        return Some(0);
    }
    let hay = format!("{} {}", d.title, d.keywords.join(" ")).to_lowercase();
    let mut cursor = 0;
    let mut gaps = 0;
    for ch in needle.chars().filter(|c| !c.is_whitespace()) {
        let found = hay[cursor..].find(ch)?;
        gaps += found;
        cursor += found + ch.len_utf8();
    }
    // Prefer a title prefix, then compact subsequences.
    Some(gaps + usize::from(!d.title.to_lowercase().starts_with(needle.trim())) * 1000)
}
#[must_use]
pub fn fuzzy<'a>(query: &str, items: &'a [CommandDescriptor]) -> Vec<&'a CommandDescriptor> {
    let mut out: Vec<_> = items
        .iter()
        .filter_map(|d| score(query, d).map(|s| (s, d)))
        .collect();
    out.sort_by(|(a, x), (b, y)| a.cmp(b).then_with(|| x.title.cmp(&y.title)));
    out.into_iter().map(|(_, d)| d).collect()
}
pub fn execute(query: &str, items: &[CommandDescriptor]) -> Result<AppCommand, ExecuteError> {
    let found = fuzzy(query, items);
    let Some(first) = found.first() else {
        return Err(ExecuteError::NotFound);
    };
    if !first.enabled {
        return Err(ExecuteError::Disabled(
            first
                .disabled_explanation
                .clone()
                .unwrap_or_else(|| "Command is unavailable".into()),
        ));
    }
    if found
        .get(1)
        .is_some_and(|next| score(query, next) == score(query, first))
    {
        return Err(ExecuteError::Ambiguous);
    }
    Ok(first.command)
}

#[derive(Clone, Debug)]
pub struct PaletteState {
    pub open: bool,
    pub query: String,
    pub selection: usize,
    initiating_widget: Option<egui::Id>,
}
impl Default for PaletteState {
    fn default() -> Self {
        Self {
            open: false,
            query: String::new(),
            selection: 0,
            initiating_widget: None,
        }
    }
}
impl PaletteState {
    pub fn open(&mut self, initiating_widget: Option<egui::Id>) {
        self.open = true;
        self.query.clear();
        self.selection = 0;
        self.initiating_widget = initiating_widget;
    }
    pub fn move_up(&mut self, count: usize) {
        if count > 0 {
            self.selection = self.selection.checked_sub(1).unwrap_or(count - 1);
        }
    }
    pub fn move_down(&mut self, count: usize) {
        if count > 0 {
            self.selection = (self.selection + 1) % count;
        }
    }
    pub fn close(&mut self, ctx: &egui::Context) {
        self.open = false;
        if let Some(id) = self.initiating_widget.take() {
            ctx.memory_mut(|memory| memory.request_focus(id));
        }
    }
    pub fn selected<'a>(&self, matches: &[&'a CommandDescriptor]) -> Option<&'a CommandDescriptor> {
        matches
            .get(self.selection.min(matches.len().saturating_sub(1)))
            .copied()
    }
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

#[cfg(test)]
mod command_tests {
    use super::*;

    #[test]
    fn unfinished_commands_are_excluded_and_context_is_explained() {
        let items = commands_for(CommandContext::default());
        assert!(!items.iter().any(|item| matches!(
            item.command,
            AppCommand::Backup | AppCommand::RetryOperation | AppCommand::CancelOperation
        )));
        let transaction = items
            .iter()
            .find(|item| item.command == AppCommand::ContextualNew)
            .unwrap();
        assert!(!transaction.enabled);
        assert_eq!(
            transaction.disabled_explanation.as_deref(),
            Some("Open an account register first")
        );
    }

    #[test]
    fn read_only_mode_disables_mutations_but_keeps_safe_reads_available() {
        let items = commands_for(CommandContext {
            budget_open: true,
            account_register: true,
            budget_workspace: true,
            mutations_disabled: true,
        });
        let new_transaction = items
            .iter()
            .find(|item| item.command == AppCommand::ContextualNew)
            .unwrap();
        assert!(!new_transaction.enabled);
        assert_eq!(
            new_transaction.disabled_explanation.as_deref(),
            Some("Mutations are disabled while the budget is in recovery mode")
        );
        let reports = items
            .iter()
            .find(|item| item.command == AppCommand::NavigateReports)
            .unwrap();
        assert!(reports.enabled);
    }

    #[test]
    fn fuzzy_prefers_title_prefix() {
        let items = commands_for(CommandContext {
            budget_open: true,
            account_register: true,
            budget_workspace: true,
            mutations_disabled: false,
        });
        assert_eq!(
            fuzzy("open rep", &items)[0].command,
            AppCommand::NavigateReports
        );
    }

    #[test]
    fn closing_restores_initiating_focus() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("initiator");
        let mut state = PaletteState::default();
        state.open(Some(id));
        state.close(&ctx);
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(id));
    }
}
