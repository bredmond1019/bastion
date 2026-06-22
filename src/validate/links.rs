// Link validation — relative link existence check.
// Implemented fully in Task 3; this stub compiles so Task 1 can wire the shell.

use crate::validate::ValidationError;
use std::path::Path;

/// Validate relative links found in a markdown file.
/// Returns an empty vec until Task 3 fills the implementation.
pub fn validate_links(_content: &str, _file: &Path) -> Vec<ValidationError> {
    vec![]
}
