use std::path::Path;

/// Remove all prior versions of a crate from the registry, keeping only the specified version.
///
/// This is used in "clean" mode to ensure only one version of each crate is stored locally.
pub fn remove_prior_versions(registry_path: &Path, crate_name: &str, keep_version: &str) {
    use std::fs;

    if let Ok(entries) = fs::read_dir(registry_path) {
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy();

            if file_name_str.ends_with(".crate")
                && let Some(stripped) = file_name_str.strip_suffix(".crate")
                && let Some(dash_pos) = stripped.rfind('-')
            {
                let file_crate_name = &stripped[..dash_pos];
                let file_version = &stripped[dash_pos + 1..];

                if file_crate_name == crate_name && file_version != keep_version {
                    if let Err(e) = fs::remove_file(entry.path()) {
                        tracing::warn!("Failed to remove old crate file {}: {}", file_name_str, e);
                    } else {
                        tracing::info!(
                            "Removed old crate file: {} (keeping {})",
                            file_name_str,
                            keep_version
                        );
                    }
                }
            }
        }
    }
}
