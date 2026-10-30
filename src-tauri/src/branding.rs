//! The application's display name.
//!
//! One stored setting decides every place the product name is shown: the window
//! title, the top bar, the About panel, and the title of each Android app window
//! WINAPK opens. The default is compiled in, so a missing, empty or no-longer
//! valid value falls back to it instead of leaving a blank title bar.
//!
//! This is display branding only. The executable name, the installer's product
//! name and the bundle identifier are build-time values and are deliberately
//! left alone: renaming them at runtime would break shortcuts and updates.

use crate::domain::{DwError, DwResult, ErrorCode};
use crate::store::Store;

/// The setting key. Absent on a fresh install, which is what makes the default
/// the answer rather than a migrated row.
pub const SETTING_KEY: &str = "app.display_name";

/// What WINAPK ships as, and what a reset returns to.
pub const DEFAULT_NAME: &str = "WINAPK";

/// Long enough for a real product name, short enough to stay a usable title.
pub const MAX_CHARS: usize = 40;

/// Trim, then refuse what a window title cannot show.
pub fn validate(input: &str) -> DwResult<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(DwError::new(
            ErrorCode::InvalidConfig,
            "The application name cannot be empty.",
        )
        .detail(format!(
            "Use 1 to {MAX_CHARS} characters, or reset the name to {DEFAULT_NAME}."
        )));
    }
    let length = trimmed.chars().count();
    if length > MAX_CHARS {
        return Err(DwError::new(
            ErrorCode::InvalidConfig,
            format!("The application name is limited to {MAX_CHARS} characters."),
        )
        .detail(format!("That name is {length} characters long.")));
    }
    // A newline or tab in a title is not a title: Windows renders part of it and
    // the rest becomes a control character. Refusing it here is kinder than
    // saving something the OS silently mangles.
    if trimmed.chars().any(char::is_control) {
        return Err(DwError::new(
            ErrorCode::InvalidConfig,
            "The application name cannot contain line breaks or control characters.",
        )
        .detail("Use a single line of text."));
    }
    Ok(trimmed.to_string())
}

/// The name to show. A stored value that no longer validates (written by an
/// older build, or edited in the database by hand) is ignored rather than
/// rendered broken.
pub fn display_name(store: &Store) -> String {
    store
        .setting(SETTING_KEY)
        .and_then(|value| validate(&value).ok())
        .unwrap_or_else(|| DEFAULT_NAME.to_string())
}

/// Persist a name, returning the value that is now in effect.
pub fn set_display_name(store: &Store, input: &str) -> DwResult<String> {
    let name = validate(input)?;
    store.set_setting(SETTING_KEY, &name)?;
    Ok(name)
}

/// Forget the stored name so the default applies again.
pub fn reset_display_name(store: &Store) -> DwResult<String> {
    store.set_setting(SETTING_KEY, "")?;
    Ok(DEFAULT_NAME.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open_in_memory().expect("an in-memory store")
    }

    #[test]
    fn a_fresh_install_shows_the_default_name() {
        assert_eq!(display_name(&store()), DEFAULT_NAME);
    }

    #[test]
    fn a_saved_name_is_what_every_surface_reads() {
        let store = store();
        let saved = set_display_name(&store, "  Windroid  ").expect("valid");
        assert_eq!(saved, "Windroid");
        assert_eq!(display_name(&store), "Windroid");
    }

    #[test]
    fn the_saved_name_survives_reopening_the_database() {
        let dir = std::env::temp_dir().join(format!("droidwin-name-{}", std::process::id()));
        let path = dir.join("name.db");
        let _ = std::fs::remove_dir_all(&dir);
        {
            let store = Store::open(&path).expect("first open");
            set_display_name(&store, "Windroid").unwrap();
        }
        let reopened = Store::open(&path).expect("second open");
        assert_eq!(display_name(&reopened), "Windroid");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_or_whitespace_name_is_refused() {
        for input in ["", "   ", "\t\n"] {
            let error = validate(input).expect_err("should be refused");
            assert_eq!(error.code, ErrorCode::InvalidConfig);
        }
    }

    #[test]
    fn a_name_longer_than_the_limit_is_refused_and_the_old_one_kept() {
        let store = store();
        set_display_name(&store, "Kept").unwrap();
        let too_long = "x".repeat(MAX_CHARS + 1);
        assert!(validate(&too_long).is_err());
        // A refused change must not clear what was already saved.
        assert!(set_display_name(&store, &too_long).is_err());
        assert_eq!(display_name(&store), "Kept");
        // Exactly at the limit is allowed.
        assert_eq!(validate(&"y".repeat(MAX_CHARS)).unwrap().len(), MAX_CHARS);
    }

    #[test]
    fn a_multi_line_name_is_refused_because_a_title_is_one_line() {
        assert!(validate("Win\ndroid").is_err());
        assert!(validate("Win\tdroid").is_err());
    }

    #[test]
    fn a_non_ascii_name_is_kept_as_written() {
        let store = store();
        assert_eq!(set_display_name(&store, "安卓窗口").unwrap(), "安卓窗口");
        assert_eq!(display_name(&store), "安卓窗口");
    }

    #[test]
    fn a_stored_value_that_no_longer_validates_falls_back_to_the_default() {
        let store = store();
        store.set_setting(SETTING_KEY, "line\nbreak").unwrap();
        assert_eq!(display_name(&store), DEFAULT_NAME);
    }

    #[test]
    fn resetting_returns_the_default_without_touching_other_settings() {
        let store = store();
        set_display_name(&store, "Windroid").unwrap();
        store.set_setting("runtime.stop_on_exit", "0").unwrap();
        assert_eq!(reset_display_name(&store).unwrap(), DEFAULT_NAME);
        assert_eq!(display_name(&store), DEFAULT_NAME);
        assert_eq!(store.setting("runtime.stop_on_exit").as_deref(), Some("0"));
    }
}
