//! BIcraft's environment variables, read through one place.
//!
//! Every `BICRAFT_*` variable is read with [`var`] or [`var_os`], so the settings stay easy to
//! find (`git grep bicraft_env::`).
//!
//! A foundation crate: no workspace dependency, usable by every layer (`cargo xtask layers`).

use std::env::VarError;
use std::ffi::OsString;

/// Prefix of every BIcraft environment variable.
pub const PREFIX: &str = "BICRAFT_";

/// `std::env::var_os(name)`.
pub fn var_os(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// `std::env::var(name)`.
pub fn var(name: &str) -> Result<String, VarError> {
    match var_os(name) {
        None => Err(VarError::NotPresent),
        Some(v) => v.into_string().map_err(VarError::NotUnicode),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_environment_unset() {
        assert!(var_os("BICRAFT_ENV_TEST_SURELY_UNSET_7F3A").is_none());
        assert!(matches!(
            var("BICRAFT_ENV_TEST_SURELY_UNSET_7F3A"),
            Err(VarError::NotPresent)
        ));
    }
}
