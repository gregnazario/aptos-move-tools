//! Move file discovery and collection.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

/// Collect all `.move` files from the given paths.
/// - For directories: recursively walks and collects all `.move` files
/// - For files: includes the path if it has a `.move` extension
pub fn collect_move_files(paths: &[impl AsRef<Path>]) -> Vec<PathBuf> {
    paths
        .iter()
        .flat_map(|p| {
            let path = p.as_ref();
            if path.is_dir() {
                WalkDir::new(path)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().is_some_and(|ext| ext == "move"))
                    .map(|e| e.path().to_path_buf())
                    .collect::<Vec<_>>()
            } else if path.extension().is_some_and(|ext| ext == "move") {
                vec![path.to_path_buf()]
            } else {
                vec![]
            }
        })
        .collect()
}
