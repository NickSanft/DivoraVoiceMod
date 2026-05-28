//! Divora core — audio engine and DSP primitives.
//!
//! In Phase 0 this crate is a placeholder. Real audio capture/output,
//! the DSP graph, and effect implementations land in Phase 1+.

/// Returns the project name. Used as a smoke test for the workspace build.
#[must_use]
pub fn project_name() -> &'static str {
    "Divora"
}

#[cfg(test)]
mod tests {
    use super::project_name;

    #[test]
    fn project_name_is_divora() {
        assert_eq!(project_name(), "Divora");
    }
}
