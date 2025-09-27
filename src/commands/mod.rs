pub mod branch;
pub mod config;
pub mod diff;
pub mod graph;
pub mod init;
pub mod latest;
pub mod list;
pub mod reset;
pub mod restore;
pub mod snap;
pub mod switch;

// Helper function to combine --file and --files options
pub fn get_selective_files(
    files: Vec<std::path::PathBuf>,
    file: Option<std::path::PathBuf>,
) -> Option<Vec<std::path::PathBuf>> {
    let mut result = files;
    if let Some(single_file) = file {
        result.push(single_file);
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}
