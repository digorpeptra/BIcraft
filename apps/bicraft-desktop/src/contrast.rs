//! The operating system's high-contrast setting, read once at start: when it is on, a window
//! without a theme preference starts in the high-contrast theme with the report pages in high
//! contrast ([`bicraft_ui::shell::Shell::system_high_contrast`]; docs/desktop.md, "High contrast").
//!
//! Windows keeps the setting of Settings › Accessibility › Contrast themes in the per-user value
//! `HKCU\Control Panel\Accessibility\HighContrast` `Flags`, a decimal number whose bit 0 is the
//! "on" flag of the `HIGHCONTRAST` structure (`HCF_HIGHCONTRASTON`, Win32 documentation); it is
//! read with the safe API of `windows-registry`. Linux: winit and eframe expose light and dark
//! only, so the desktop does not follow a desktop environment's contrast preference there.

/// The registry key and value that hold the Windows high-contrast flags.
#[cfg(windows)]
const KEY: &str = r"Control Panel\Accessibility\HighContrast";
#[cfg(windows)]
const VALUE: &str = "Flags";

/// Whether Windows high contrast is on (false when the value is missing or unreadable, and on
/// other systems).
pub fn system_high_contrast() -> bool {
    #[cfg(windows)]
    {
        windows_registry::CURRENT_USER
            .open(KEY)
            .and_then(|k| k.get_string(VALUE))
            .is_ok_and(|flags| flags_on(&flags))
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Whether the `Flags` text (a decimal number) has the high-contrast "on" bit; anything that is
/// not a number reads as off.
pub fn flags_on(flags: &str) -> bool {
    flags.trim().parse::<u32>().is_ok_and(|f| f & 1 == 1)
}

#[cfg(test)]
mod tests {
    use super::flags_on;

    /// Only the "on" bit counts (126, the value read on Windows 11 with high contrast off, is off;
    /// the same with bit 0 set is on); a missing or garbled value never turns it on.
    #[test]
    fn the_on_bit_turns_high_contrast_on() {
        assert!(flags_on("127") && flags_on(" 1 "));
        assert!(!flags_on("126") && !flags_on("0"));
        assert!(!flags_on("") && !flags_on("on") && !flags_on("-1") && !flags_on("99999999999"));
    }
}
