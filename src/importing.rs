//! Statement-import boundary. Parsed transactions remain pending until reviewed.

#[derive(Debug)]
pub struct PendingImport {
    pub source_name: String,
    pub row_count: usize,
}
