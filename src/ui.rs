//! Immediate-mode UI. This layer emits typed commands and does not access persistence.

use crate::service::AppCommand;

pub fn shell(ctx: &egui::Context, commands: &mut Vec<AppCommand>) {
    egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
        ui.heading("MNAB — Multi Needs A Budget");
    });
    egui::SidePanel::left("navigation")
        .min_width(210.0)
        .show(ctx, |ui| {
            ui.heading("Budgets");
            if ui.button("New budget").clicked() {
                commands.push(AppCommand::CreateBudget {
                    name: "New budget".into(),
                });
            }
        });
    egui::SidePanel::right("inspector")
        .min_width(250.0)
        .show(ctx, |ui| {
            ui.heading("Inspector");
            ui.label("Select an item to see its details.");
        });
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.centered_and_justified(|ui| ui.label("Open or create a budget to begin."));
    });
}
