use std::path::{Path, PathBuf};

/// Get the local filesystem path for a crate's index file based on cargo's naming convention.
///
/// Cargo uses specific path patterns based on crate name length:
/// - 1 char: index/1/{name}
/// - 2 char: index/2/{name}
/// - 3 char: index/3/{first-char}/{name}
/// - 4+ char: index/{first-2-chars}/{chars-3-4}/{name}
pub fn get_index_path(registry_path: &Path, crate_name: &str) -> PathBuf {
    match crate_name.len() {
        1 => registry_path.join("index").join("1").join(crate_name),
        2 => registry_path.join("index").join("2").join(crate_name),
        3 => registry_path
            .join("index")
            .join("3")
            .join(&crate_name[..1])
            .join(crate_name),
        _ => registry_path
            .join("index")
            .join(&crate_name[..2])
            .join(&crate_name[2..4])
            .join(crate_name),
    }
}

/// Get the crates.io URL for a crate's index file.
pub fn get_crates_io_index_url(crate_name: &str) -> String {
    match crate_name.len() {
        1 => format!("https://index.crates.io/1/{}", crate_name),
        2 => format!("https://index.crates.io/2/{}", crate_name),
        3 => format!(
            "https://index.crates.io/3/{}/{}",
            &crate_name[..1],
            crate_name
        ),
        _ => format!(
            "https://index.crates.io/{}/{}/{}",
            &crate_name[..2],
            &crate_name[2..4],
            crate_name
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_get_index_path_1_char() {
        let registry = PathBuf::from("/tmp/registry");
        assert_eq!(
            get_index_path(&registry, "a"),
            PathBuf::from("/tmp/registry/index/1/a")
        );
    }

    #[test]
    fn test_get_index_path_2_char() {
        let registry = PathBuf::from("/tmp/registry");
        assert_eq!(
            get_index_path(&registry, "ab"),
            PathBuf::from("/tmp/registry/index/2/ab")
        );
    }

    #[test]
    fn test_get_index_path_3_char() {
        let registry = PathBuf::from("/tmp/registry");
        assert_eq!(
            get_index_path(&registry, "abc"),
            PathBuf::from("/tmp/registry/index/3/a/abc")
        );
    }

    #[test]
    fn test_get_index_path_4plus_char() {
        let registry = PathBuf::from("/tmp/registry");
        assert_eq!(
            get_index_path(&registry, "serde"),
            PathBuf::from("/tmp/registry/index/se/rd/serde")
        );
    }

    #[test]
    fn test_get_crates_io_index_url_1_char() {
        assert_eq!(get_crates_io_index_url("a"), "https://index.crates.io/1/a");
    }

    #[test]
    fn test_get_crates_io_index_url_2_char() {
        assert_eq!(
            get_crates_io_index_url("ab"),
            "https://index.crates.io/2/ab"
        );
    }

    #[test]
    fn test_get_crates_io_index_url_3_char() {
        assert_eq!(
            get_crates_io_index_url("abc"),
            "https://index.crates.io/3/a/abc"
        );
    }

    #[test]
    fn test_get_crates_io_index_url_4plus_char() {
        assert_eq!(
            get_crates_io_index_url("serde"),
            "https://index.crates.io/se/rd/serde"
        );
    }
}
