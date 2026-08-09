//! The only global shortcut router. Widgets may emit local edits, but never execute these commands.
use crate::app::command::AppCommand;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Modifiers {
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    P,
    K,
    N,
    A,
    I,
    F,
    Z,
    Enter,
    Escape,
    E,
    Delete,
    F2,
    Digit1,
    Digit2,
    Digit3,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Tab,
    Space,
    Comma,
    B,
    Backslash,
}

#[must_use]
pub fn matches_shortcut(stroke: KeyStroke, shortcut: &str) -> bool {
    let normalized = shortcut.trim().replace(' ', "").to_ascii_lowercase();
    let key = match stroke.key {
        Key::P => "p",
        Key::K => "k",
        Key::N => "n",
        Key::A => "a",
        Key::I => "i",
        Key::F => "f",
        Key::Z => "z",
        Key::E => "e",
        Key::Comma => ",",
        Key::Backslash => "\\",
        _ => return false,
    };
    let mut expected = String::from(if stroke.modifiers.command {
        "ctrl+"
    } else {
        ""
    });
    if stroke.modifiers.shift {
        expected.push_str("shift+");
    }
    if stroke.modifiers.alt {
        expected.push_str("alt+");
    }
    expected.push_str(key);
    normalized == expected
}
#[derive(Clone, Copy, Debug)]
pub struct KeyStroke {
    pub key: Key,
    pub modifiers: Modifiers,
    pub repeat: bool,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Scope {
    pub modal: bool,
    pub text_editor: bool,
    pub command_enabled: bool,
}

/// Canonical, non-configurable global bindings. Preferences are validated against
/// this table so the keyboard router and settings cannot silently drift apart.
pub const FIXED_BINDINGS: &[&str] = &[
    "Ctrl+N",
    "Ctrl+Shift+A",
    "Ctrl+I",
    "Ctrl+F",
    "Ctrl+Z",
    "Ctrl+Shift+Z",
    "Ctrl+E",
    "Ctrl+1",
    "Ctrl+2",
    "Ctrl+3",
    "Ctrl+Left",
    "Ctrl+Right",
    "Ctrl+,",
    "Ctrl+\\",
];

#[must_use]
pub fn conflicts_with_fixed(shortcut: &str) -> bool {
    let candidate = shortcut.trim().replace(' ', "");
    FIXED_BINDINGS
        .iter()
        .any(|fixed| fixed.eq_ignore_ascii_case(&candidate))
}

/// Maps platform-normalized Command (Ctrl on Windows/Linux, Cmd on macOS) input.
pub fn map(stroke: KeyStroke, scope: Scope) -> Option<AppCommand> {
    use AppCommand as C;
    use Key as K;
    if stroke.repeat && !matches!(stroke.key, K::ArrowLeft | K::ArrowRight) {
        return None;
    }
    if scope.modal {
        return match stroke.key {
            K::Escape => Some(C::Cancel),
            K::Enter => Some(C::Commit),
            _ => None,
        };
    }
    if scope.text_editor {
        return match (stroke.key, stroke.modifiers.command, stroke.modifiers.shift) {
            // Find is an application search command even while another text edit owns input.
            (K::F, true, false) => Some(C::FocusSearch),
            (K::Escape, _, _) => Some(C::Cancel),
            (K::Enter, false, _) => Some(C::Commit),
            (K::Z, true, false) => Some(C::Undo),
            (K::Z, true, true) => Some(C::Redo),
            _ => None,
        };
    }
    let m = stroke.modifiers;
    let command = match (stroke.key, m.command, m.shift, m.alt) {
        (K::N, true, false, false) => C::ContextualNew,
        (K::A, true, false, false) => C::SelectAllTransactions,
        (K::A, true, true, false) => C::AddAccount,
        (K::I, true, false, false) => C::Import,
        (K::F, true, false, false) => C::FocusSearch,
        (K::Z, true, false, false) => C::Undo,
        (K::Z, true, true, false) => C::Redo,
        (K::Enter, false, _, _) => C::Commit,
        (K::Escape, false, _, _) => C::Cancel,
        (K::E, true, false, false) => C::Edit,
        (K::Delete, false, _, _) => C::Delete,
        (K::ArrowUp, false, _, _) => C::MoveUp,
        (K::ArrowDown, false, _, _) => C::MoveDown,
        (K::Tab, false, false, _) => C::NextField,
        (K::Tab, false, true, _) => C::PreviousField,
        (K::Space, false, _, _) => C::ToggleSelection,
        (K::F2, false, _, _) => C::Edit,
        (K::Digit1, true, false, false) => C::NavigateCategories,
        (K::Digit2, true, false, false) => C::NavigateReports,
        (K::Digit3, true, false, false) => C::NavigateAllTransactions,
        (K::ArrowLeft, true, false, false) => C::PreviousMonth,
        (K::ArrowRight, true, false, false) => C::NextMonth,
        (K::Comma, true, false, false) => C::Settings,
        (K::B, true, true, false) => C::Backup,
        (K::Backslash, true, false, false) => C::ToggleInspector,
        _ => return None,
    };
    if scope.command_enabled {
        Some(command)
    } else {
        None
    }
}

/// Reads raw egui events once per frame and emits semantic commands.
pub fn route(ctx: &egui::Context, scope: Scope, out: &mut Vec<AppCommand>) {
    ctx.input(|input| {
        for event in &input.events {
            if let egui::Event::Key {
                key,
                pressed: true,
                repeat,
                modifiers,
                physical_key: _,
            } = event
            {
                let key = match key {
                    egui::Key::P => Key::P,
                    egui::Key::K => Key::K,
                    egui::Key::N => Key::N,
                    egui::Key::A => Key::A,
                    egui::Key::I => Key::I,
                    egui::Key::F => Key::F,
                    egui::Key::Z => Key::Z,
                    egui::Key::Enter => Key::Enter,
                    egui::Key::Escape => Key::Escape,
                    egui::Key::E => Key::E,
                    egui::Key::Delete => Key::Delete,
                    egui::Key::F2 => Key::F2,
                    egui::Key::Num1 => Key::Digit1,
                    egui::Key::Num2 => Key::Digit2,
                    egui::Key::Num3 => Key::Digit3,
                    egui::Key::ArrowLeft => Key::ArrowLeft,
                    egui::Key::ArrowRight => Key::ArrowRight,
                    egui::Key::ArrowUp => Key::ArrowUp,
                    egui::Key::ArrowDown => Key::ArrowDown,
                    egui::Key::Tab => Key::Tab,
                    egui::Key::Space => Key::Space,
                    egui::Key::Comma => Key::Comma,
                    egui::Key::B => Key::B,
                    egui::Key::Backslash => Key::Backslash,
                    _ => continue,
                };
                let modifiers = Modifiers {
                    command: modifiers.command,
                    shift: modifiers.shift,
                    alt: modifiers.alt,
                };
                if let Some(command) = map(
                    KeyStroke {
                        key,
                        modifiers,
                        repeat: *repeat,
                    },
                    scope,
                ) {
                    out.push(command);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(key: Key, command: bool, shift: bool) -> KeyStroke {
        KeyStroke {
            key,
            modifiers: Modifiers {
                command,
                shift,
                alt: false,
            },
            repeat: false,
        }
    }

    #[test]
    fn find_reaches_search_while_modal_escape_keeps_precedence() {
        assert_eq!(
            map(
                key(Key::F, true, false),
                Scope {
                    text_editor: true,
                    command_enabled: true,
                    ..Scope::default()
                }
            ),
            Some(AppCommand::FocusSearch)
        );
        assert_eq!(
            map(
                key(Key::Escape, false, false),
                Scope {
                    modal: true,
                    text_editor: true,
                    command_enabled: true
                }
            ),
            Some(AppCommand::Cancel)
        );
    }
    #[test]
    fn shortcuts_and_conflicts() {
        let s = Scope {
            command_enabled: true,
            ..Scope::default()
        };
        assert_eq!(
            map(key(Key::N, true, false), s),
            Some(AppCommand::ContextualNew)
        );
        assert_eq!(map(key(Key::N, true, true), s), None);
        assert_eq!(map(key(Key::Z, true, true), s), Some(AppCommand::Redo));
        assert_eq!(
            map(key(Key::Backslash, true, false), s),
            Some(AppCommand::ToggleInspector)
        );
    }
    #[test]
    fn modal_consumes_commit_cancel() {
        let s = Scope {
            modal: true,
            command_enabled: true,
            text_editor: false,
        };
        assert_eq!(
            map(key(Key::Escape, false, false), s),
            Some(AppCommand::Cancel)
        );
        assert_eq!(
            map(key(Key::Enter, false, false), s),
            Some(AppCommand::Commit)
        );
        assert_eq!(map(key(Key::Delete, false, false), s), None);
    }
    #[test]
    fn editors_block_destructive_globals() {
        let s = Scope {
            text_editor: true,
            command_enabled: true,
            modal: false,
        };
        assert_eq!(map(key(Key::Delete, false, false), s), None);
        assert_eq!(map(key(Key::A, true, false), s), None);
        assert_eq!(
            map(
                key(Key::A, true, false),
                Scope {
                    text_editor: false,
                    ..s
                }
            ),
            Some(AppCommand::SelectAllTransactions)
        );
        assert_eq!(map(key(Key::N, true, false), s), None);
        assert_eq!(map(key(Key::Z, true, false), s), Some(AppCommand::Undo));
    }
}
