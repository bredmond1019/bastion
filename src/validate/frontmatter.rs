// Frontmatter validation — OKF required fields (`type`, `title`, `description`).
// Implemented fully in Task 2; this stub compiles so Task 1 can wire the shell.

use crate::validate::ValidationError;
use std::path::Path;

/// Validate the OKF frontmatter of a markdown file.
/// Returns an empty vec until Task 2 fills the implementation.
pub fn validate_frontmatter(_content: &str, _file: &Path) -> Vec<ValidationError> {
    vec![]
}
