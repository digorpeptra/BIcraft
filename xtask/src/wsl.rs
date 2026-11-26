//! Helpers for running xtask inside WSL from a Windows checkout.

/// Translate a Windows path (`C:\x\y`, optionally `\\?\`-prefixed) to its
/// WSL mount path (`/mnt/c/x/y`).
pub fn to_wsl_path(win: &str) -> Result<String, String> {
    let p = win.strip_prefix(r"\\?\").unwrap_or(win);
    let mut chars = p.chars();
    let (Some(drive), Some(':')) = (chars.next(), chars.next()) else {
        return Err(format!("wsl: `{win}` is not a drive-letter path (C:\\...)"));
    };
    if !drive.is_ascii_alphabetic() {
        return Err(format!("wsl: `{win}` is not a drive-letter path (C:\\...)"));
    }
    let rest = chars.as_str().replace('\\', "/");
    let rest = rest.trim_end_matches('/');
    let rest = if rest.is_empty() || rest.starts_with('/') {
        rest.to_owned()
    } else {
        format!("/{rest}")
    };
    Ok(format!("/mnt/{}{rest}", drive.to_ascii_lowercase()))
}

/// Quote `s` as a single bash word.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The bash script running `cargo xtask ARGS...` inside WSL. The Linux-side target dir is
/// `$BICRAFT_WSL_TARGET_DIR` read on the Windows side (a leading `~/` means the WSL home),
/// default `~/target/bicraft`.
pub fn script_args(repo: &str, args: &[String]) -> String {
    let target = bicraft_env::var("BICRAFT_WSL_TARGET_DIR").ok();
    script_with_target(repo, args, target.as_deref())
}

fn script_with_target(repo: &str, args: &[String], target: Option<&str>) -> String {
    let target = match target.map(str::trim).filter(|t| !t.is_empty()) {
        None => "\"$HOME/target/bicraft\"".to_owned(),
        Some(t) => match t.strip_prefix("~/") {
            Some(rest) => format!("\"$HOME\"/{}", sh_quote(rest)),
            None => sh_quote(t),
        },
    };
    let args: Vec<String> = args.iter().map(|a| sh_quote(a)).collect();
    format!(
        "cd {} && CARGO_TARGET_DIR={target} cargo xtask {}",
        sh_quote(repo),
        args.join(" ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_paths_map_to_mnt() {
        assert_eq!(to_wsl_path(r"C:\x\y").unwrap(), "/mnt/c/x/y");
        assert_eq!(to_wsl_path(r"D:\a b\c\").unwrap(), "/mnt/d/a b/c");
        assert_eq!(to_wsl_path(r"\\?\C:\Users\me").unwrap(), "/mnt/c/Users/me");
        assert_eq!(to_wsl_path(r"C:\").unwrap(), "/mnt/c");
        assert_eq!(to_wsl_path("C:/x/y").unwrap(), "/mnt/c/x/y");
    }

    #[test]
    fn non_drive_paths_rejected() {
        assert!(to_wsl_path("/home/me").is_err());
        assert!(to_wsl_path(r"\\server\share").is_err());
        assert!(to_wsl_path("").is_err());
        assert!(to_wsl_path("1:\\x").is_err());
    }

    #[test]
    fn script_quotes_path() {
        assert_eq!(
            script_with_target("/mnt/c/it's here", &["ci".to_owned()], None),
            "cd '/mnt/c/it'\\''s here' && CARGO_TARGET_DIR=\"$HOME/target/bicraft\" cargo xtask 'ci'"
        );
    }

    #[test]
    fn script_target_dir_and_args() {
        let args = ["dist".to_owned(), "--out".to_owned(), "/mnt/c/a b".to_owned()];
        assert_eq!(
            script_with_target("/r", &args, Some("~/target/pkg")),
            "cd '/r' && CARGO_TARGET_DIR=\"$HOME\"/'target/pkg' cargo xtask 'dist' '--out' '/mnt/c/a b'"
        );
        assert_eq!(
            script_with_target("/r", &args[..1], Some("/tmp/t")),
            "cd '/r' && CARGO_TARGET_DIR='/tmp/t' cargo xtask 'dist'"
        );
    }
}
