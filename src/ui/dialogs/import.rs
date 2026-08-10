use crate::app::state::ImportEditorState;

pub fn show(ui: &mut egui::Ui, editor: &mut ImportEditorState) {
    ui.label("Select file → Parse → Map CSV → Match & deduplicate → Review → Apply atomically");
    ui.label(format!(
        "File: {}",
        if editor.source.as_os_str().is_empty() {
            "No file selected".into()
        } else {
            editor.source.display().to_string()
        }
    ));
    if ui.button("Choose CSV or OFX file…").clicked()
        && let Some(path) = rfd::FileDialog::new()
            .add_filter("Statements", &["csv", "ofx", "qfx"])
            .pick_file()
    {
        editor.source = path;
        editor.metadata.dirty = true;
    }
    ui.small("Exact duplicates are excluded by default. Review is applied atomically; imported transactions appear in Inbox.");
}
