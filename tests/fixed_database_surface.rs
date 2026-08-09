use mnab::{
    app::{
        command::{AccountCommand, AppCommand, MAJOR_WORKFLOW_COMMANDS},
        palette::{CommandContext, commands_for},
        portable_paths::PortablePaths,
        settings::Settings,
    },
    domain::AccountId,
};

const PROHIBITED_DESCRIPTOR_TERMS: &[&str] = &[
    "delete budget",
    "reset budget",
    "create empty database",
    "recent budget",
    "open budget file",
    "archive budget",
];

#[test]
fn production_command_registries_have_no_fixed_database_destructive_descriptors() {
    let descriptors = commands_for(CommandContext {
        database_available: true,
        account_register: true,
        categories_workspace: true,
        ..CommandContext::default()
    });
    for descriptor in descriptors {
        let searchable =
            format!("{} {}", descriptor.title, descriptor.keywords.join(" ")).to_ascii_lowercase();
        for prohibited in PROHIBITED_DESCRIPTOR_TERMS {
            assert!(
                !searchable.contains(prohibited),
                "prohibited command descriptor {prohibited:?} was registered by {:?}",
                descriptor.command
            );
        }
    }

    // The setup completion command is reachable only from the absence-driven onboarding modal,
    // not from the global palette or any shortcut descriptor.
    assert!(
        !commands_for(CommandContext::default())
            .iter()
            .any(|item| item.command == AppCommand::CompleteOnboarding)
    );
    assert!(MAJOR_WORKFLOW_COMMANDS.contains(&AppCommand::CompleteOnboarding));
}

#[test]
fn destructive_budget_lifecycle_api_names_are_absent_from_production_sources() {
    let command_api = include_str!("../src/app/command.rs");
    let runtime_api = include_str!("../src/app/runtime.rs");
    let dialogs = include_str!("../src/app/state.rs");
    let catalog = include_str!("../src/app/budget_catalog.rs");

    assert!(!command_api.contains("BudgetAction::Delete"));
    assert!(!runtime_api.contains("fn delete_budget"));
    assert!(!dialogs.contains("ConfirmDelete"));
    assert!(!catalog.contains("pub fn delete("));
    assert!(!catalog.contains("pub fn prepare_open"));
    assert!(!catalog.contains("pub fn recent("));
}

#[test]
fn portable_database_and_settings_cannot_persist_budget_file_selection() {
    let root = tempfile::tempdir().unwrap();
    let executable = root.path().join("mnab");
    let paths = PortablePaths::from_executable(&executable).unwrap();
    assert_eq!(paths.database, root.path().join("mnab-data/mnab.sqlite3"));

    let serialized = serde_json::to_string(&Settings::default()).unwrap();
    for prohibited_key in [
        "recent_budget",
        "budget_path",
        "database_path",
        "last_budget",
        "archive_state",
    ] {
        assert!(!serialized.contains(prohibited_key));
    }
}

#[test]
fn unused_account_deletion_remains_explicitly_scoped() {
    let command = AccountCommand::DeleteUnused(AccountId::new());
    assert!(matches!(command, AccountCommand::DeleteUnused(_)));
    let service = include_str!("../src/service/account_service.rs");
    assert!(service.contains("pub fn delete_if_unused"));
    assert!(service.contains("AccountServiceError::InUse"));
}
