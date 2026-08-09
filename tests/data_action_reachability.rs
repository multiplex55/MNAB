use std::path::PathBuf;

use mnab::{
    app::command::{ApplicationAction, DataAction},
    storage::repair::RepairRequest,
};

#[test]
fn every_exposed_data_command_has_a_typed_reachable_action() {
    let actions = [
        DataAction::CreateBackup,
        DataAction::RestoreBackup {
            metadata_path: PathBuf::from("backup.json"),
            confirmed: false,
        },
        DataAction::Validate,
        DataAction::Repair {
            request: RepairRequest::Reindex,
            confirmed: false,
        },
        DataAction::RevealDataDirectory,
        DataAction::RevealBackupDirectory,
        DataAction::RenameBudget {
            name: "Home".into(),
        },
    ];
    assert_eq!(actions.len(), 7);
    assert!(
        actions
            .into_iter()
            .all(|action| matches!(ApplicationAction::Data(action), ApplicationAction::Data(_)))
    );

    let menu = include_str!("../src/ui/shell.rs");
    let runtime = include_str!("../src/app/runtime.rs");
    for variant in [
        "CreateBackup",
        "RestoreBackup",
        "Validate",
        "Repair",
        "RevealDataDirectory",
        "RevealBackupDirectory",
        "RenameBudget",
    ] {
        assert!(
            menu.contains(variant),
            "{variant} is not reachable from Data UI"
        );
        assert!(
            runtime.contains(&format!("DataAction::{variant}")),
            "{variant} is inert"
        );
    }
}

#[test]
fn maintenance_no_longer_uses_budget_or_recents_routing() {
    let command = include_str!("../src/app/command.rs");
    let shell = include_str!("../src/ui/shell.rs");
    assert!(!command.contains("enum BudgetAction"));
    assert!(!command.contains("ShowRecents"));
    assert!(!shell.contains("BudgetAction"));
}

#[test]
fn risky_actions_have_cancel_and_confirmation_paths() {
    let shell = include_str!("../src/ui/shell.rs");
    let runtime = include_str!("../src/app/runtime.rs");
    assert!(shell.contains("Confirm operation"));
    assert!(shell.contains("pending_data_action = None"));
    assert!(runtime.contains("confirmed: false"));
    assert!(runtime.contains("confirmed: true"));
    assert!(runtime.contains("Restore results"));
    assert!(runtime.contains("Repair results"));
}
