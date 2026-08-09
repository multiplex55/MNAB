//! Rendering and pure policy helpers for operation notifications.
use crate::app::{
    dispatcher::ActionCollector,
    state::{AppState, NotificationKind, NotificationRetryAction},
};

#[must_use]
pub const fn retry_command(action: NotificationRetryAction) -> crate::app::command::AppCommand {
    match action {
        NotificationRetryAction::RetryOperation => crate::app::command::AppCommand::RetryOperation,
    }
}

/// Removes technical material which must remain in diagnostics, not user-facing UI.
#[must_use]
pub fn sanitize_failure(message: &str) -> String {
    let lower = message.to_ascii_lowercase();
    if lower.contains("select ")
        || lower.contains("insert ")
        || lower.contains("update ")
        || lower.contains("delete from")
        || lower.contains("sqlite")
        || lower.contains(" at /")
    {
        "The storage operation failed without changing your data. Review storage access and try again.".into()
    } else {
        let safe = message.lines().next().unwrap_or_default().trim();
        if safe.is_empty() {
            "The operation could not be completed.".into()
        } else {
            safe.chars().take(240).collect()
        }
    }
}

pub fn show(ctx: &egui::Context, state: &mut AppState, actions: &mut ActionCollector) {
    let now = time::OffsetDateTime::now_utc();
    state
        .notifications
        .retain(|notice| !notice.is_expired_at(now));
    if state.notifications.is_empty() {
        return;
    }
    let mut dismiss = None;
    egui::TopBottomPanel::bottom("application-notifications").show(ctx, |ui| {
        for (index, notice) in state.notifications.iter().enumerate() {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    let (icon, color) = match notice.kind {
                        NotificationKind::Information => {
                            ("✓", egui::Color32::from_rgb(35, 150, 80))
                        }
                        NotificationKind::Warning => ("⚠", ui.visuals().warn_fg_color),
                        NotificationKind::Error => ("⛔", ui.visuals().error_fg_color),
                    };
                    ui.colored_label(color, format!("{icon} {}", notice.title));
                    if notice.persistent {
                        ui.collapsing("Details", |ui| {
                            ui.label(&notice.detail);
                        });
                    } else {
                        ui.label(&notice.detail);
                    }
                    if let Some(retry) = notice.retry_action
                        && ui.button("Retry").clicked()
                    {
                        actions.push(retry_command(retry));
                    }
                    if ui.button("Dismiss").clicked() {
                        dismiss = Some(index);
                    }
                })
            });
        }
    });
    if let Some(index) = dismiss {
        state.dismiss_notification(index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::Notification;
    #[test]
    fn transient_expires_but_critical_persists() {
        let success = Notification::success("Saved", "Done");
        let critical = Notification::actionable_error("Failed", "Safe detail");
        let later = success.created_at + time::Duration::seconds(9);
        assert!(success.is_expired_at(later));
        assert!(!critical.is_expired_at(later));
    }
    #[test]
    fn retry_maps_to_real_command() {
        assert_eq!(
            retry_command(NotificationRetryAction::RetryOperation),
            crate::app::command::AppCommand::RetryOperation
        );
    }
    #[test]
    fn technical_failures_are_sanitized() {
        let rendered =
            sanitize_failure("repository error: SELECT secret FROM x at /home/me/budget.sqlite");
        assert!(!rendered.contains("SELECT"));
        assert!(!rendered.contains("/home"));
    }
}
