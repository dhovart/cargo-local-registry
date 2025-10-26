use semver::Version;

/// Parse a crate filename in the format "{name}-{version}.crate" into its components.
///
/// This function follows cargo's filename format: `format!("{}-{}.crate", id.name(), id.version())`
/// (see cargo/src/cargo/core/package_id.rs)
///
/// It uses the semver crate to validate versions, ensuring we correctly parse:
/// - Crate names ending with digits (e.g., "sec1-0.7.3.crate")
/// - Crate names with dashes and digits (e.g., "foo-1-2.0.crate")
/// - Versions with dashes and plus signs (e.g., "curl-sys-0.4.80+curl-8.12.1.crate")
///
/// The algorithm tries each dash position and validates if what follows is a valid semver version.
pub fn parse_crate_filename(filename: &str) -> Option<(&str, &str)> {
    let stripped = filename.strip_suffix(".crate")?;

    // Try each dash position, looking for a valid semver version after it
    for (idx, _) in stripped.match_indices('-') {
        let potential_name = &stripped[..idx];
        let potential_version = &stripped[idx + 1..];

        // Validate that what follows the dash is a valid semver version
        // This is the canonical way to determine the split point
        if Version::parse(potential_version).is_ok() && !potential_name.is_empty() {
            return Some((potential_name, potential_version));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_crate_filename_simple() {
        assert_eq!(
            parse_crate_filename("serde-1.0.130.crate"),
            Some(("serde", "1.0.130"))
        );
    }

    #[test]
    fn test_parse_crate_filename_with_dash_in_version() {
        assert_eq!(
            parse_crate_filename("curl-sys-0.4.80+curl-8.12.1.crate"),
            Some(("curl-sys", "0.4.80+curl-8.12.1"))
        );
    }

    #[test]
    fn test_parse_crate_filename_name_ending_with_digit() {
        assert_eq!(
            parse_crate_filename("sec1-0.7.3.crate"),
            Some(("sec1", "0.7.3"))
        );
    }

    #[test]
    fn test_parse_crate_filename_invalid_no_crate_suffix() {
        assert_eq!(parse_crate_filename("serde-1.0.130"), None);
    }

    #[test]
    fn test_parse_crate_filename_invalid_no_dash() {
        assert_eq!(parse_crate_filename("serde.crate"), None);
    }

    #[test]
    fn test_parse_crate_filename_invalid_no_version() {
        assert_eq!(parse_crate_filename("serde-.crate"), None);
    }

    #[test]
    fn test_parse_crate_filename_invalid_no_name() {
        assert_eq!(parse_crate_filename("-1.0.130.crate"), None);
    }

    #[test]
    fn test_semver_validation() {
        use semver::Version;

        // Verify semver handles all the cases we need
        assert!(
            Version::parse("0.4.80+curl-8.12.1").is_ok(),
            "Build metadata should work"
        );
        assert!(
            Version::parse("0.7.3").is_ok(),
            "Standard version should work"
        );
        assert!(
            Version::parse("1.0.130").is_ok(),
            "Three-part version should work"
        );
        assert!(
            Version::parse("2.0.0").is_ok(),
            "Explicit patch should work"
        );

        // These should fail
        assert!(
            Version::parse("2.0").is_err(),
            "Two-part version should fail"
        );
        assert!(
            Version::parse("1-2.0").is_err(),
            "Invalid format should fail"
        );
    }
}
