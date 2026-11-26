//! The supported agent clients and where each one keeps its MCP configuration.
//!
//! Sources (checked 2026-10):
//! - Claude Code: <https://code.claude.com/docs/en/mcp> (user scope: `mcpServers` in
//!   `~/.claude.json`, or `$CLAUDE_CONFIG_DIR/.claude.json`; project scope: `.mcp.json`).
//! - Claude Desktop: <https://modelcontextprotocol.io/docs/develop/connect-local-servers>
//!   (`mcpServers` in `claude_desktop_config.json`); the Windows MSIX package reads a
//!   package-private copy under `%LOCALAPPDATA%\Packages\Claude_pzs8sxrjxfjjc`.
//! - VS Code: <https://code.visualstudio.com/docs/copilot/customization/mcp-servers> (top-level
//!   `servers` in the user profile's `mcp.json`, or `.vscode/mcp.json`; `type: "stdio"`).
//! - Cursor: <https://cursor.com/docs/context/mcp> (`mcpServers` in `~/.cursor/mcp.json` or
//!   `.cursor/mcp.json`; `type: "stdio"`).
//! - Codex: <https://developers.openai.com/codex/mcp> (`[mcp_servers.<name>]` in
//!   `$CODEX_HOME/config.toml`, default `~/.codex/config.toml`; `.codex/config.toml` in trusted
//!   projects).

use std::path::{Path, PathBuf};

/// An agent client that `bicraft setup` can register the MCP server with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Client {
    ClaudeCode,
    ClaudeDesktop,
    VsCode,
    Cursor,
    Codex,
}

/// How a client's configuration file is written.
#[derive(Clone, Copy, Debug)]
pub enum Format {
    /// A JSON object holding a map of servers under `servers_key`.
    Json {
        servers_key: &'static str,
        /// New entries get `"type": "stdio"`.
        typed: bool,
    },
    /// Codex's `config.toml`, with a `[mcp_servers.<name>]` table per server.
    Toml,
}

impl Client {
    pub const ALL: [Client; 5] = [
        Client::ClaudeCode,
        Client::ClaudeDesktop,
        Client::VsCode,
        Client::Cursor,
        Client::Codex,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Client::ClaudeCode => "claude_code",
            Client::ClaudeDesktop => "claude_desktop",
            Client::VsCode => "vscode",
            Client::Cursor => "cursor",
            Client::Codex => "codex",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Client::ClaudeCode => "Claude Code",
            Client::ClaudeDesktop => "Claude Desktop",
            Client::VsCode => "VS Code",
            Client::Cursor => "Cursor",
            Client::Codex => "Codex",
        }
    }

    /// Accepts the id, with `-` for `_` and any case (`claude-code`, `VSCode`).
    pub fn parse(s: &str) -> Option<Client> {
        let norm = s.trim().to_ascii_lowercase().replace('-', "_");
        let alias = match norm.as_str() {
            "claude" => "claude_code",
            "vs_code" | "code" => "vscode",
            other => other,
        };
        Client::ALL.into_iter().find(|c| c.id() == alias)
    }

    pub fn format(self) -> Format {
        match self {
            Client::ClaudeCode => Format::Json {
                servers_key: "mcpServers",
                typed: true,
            },
            Client::ClaudeDesktop => Format::Json {
                servers_key: "mcpServers",
                typed: false,
            },
            Client::VsCode => Format::Json {
                servers_key: "servers",
                typed: true,
            },
            Client::Cursor => Format::Json {
                servers_key: "mcpServers",
                typed: true,
            },
            Client::Codex => Format::Toml,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Linux,
    MacOs,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(windows) {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::MacOs
        } else {
            Platform::Linux
        }
    }

    pub fn parse(s: &str) -> Option<Platform> {
        match s.to_ascii_lowercase().as_str() {
            "windows" => Some(Platform::Windows),
            "linux" => Some(Platform::Linux),
            "macos" | "mac" | "darwin" => Some(Platform::MacOs),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Platform::Windows => "windows",
            Platform::Linux => "linux",
            Platform::MacOs => "macos",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    User,
    Project,
}

impl Scope {
    pub fn id(self) -> &'static str {
        match self {
            Scope::User => "user",
            Scope::Project => "project",
        }
    }
}

/// The directories client paths are resolved against.
pub struct Ctx {
    pub platform: Platform,
    home: PathBuf,
    appdata: PathBuf,
    local_appdata: PathBuf,
    xdg_config: PathBuf,
    claude_config_dir: Option<PathBuf>,
    codex_home: Option<PathBuf>,
    pub project_dir: PathBuf,
    /// Set when every path lives under a sandbox root (`--config-root`, `BICRAFT_SETUP_HOME`).
    pub root: Option<PathBuf>,
}

/// Where a client's configuration lives.
pub struct Location {
    pub path: PathBuf,
    /// The client looks installed (its configuration file or directory exists).
    pub detected: bool,
}

/// The Claude Desktop MSIX package family on Windows.
const CLAUDE_MSIX_FAMILY: &str = "Claude_pzs8sxrjxfjjc";

fn env_dir(name: &str) -> Option<PathBuf> {
    let v = std::env::var_os(name)?;
    let p = PathBuf::from(v);
    (!p.as_os_str().is_empty() && p.is_absolute()).then_some(p)
}

impl Ctx {
    /// A context under `root` (sandboxed: no environment variable is consulted), or from the real
    /// environment.
    pub fn new(root: Option<PathBuf>, platform: Platform, project_dir: PathBuf) -> Result<Ctx, String> {
        if let Some(root) = root {
            return Ok(Ctx {
                platform,
                appdata: root.join("AppData").join("Roaming"),
                local_appdata: root.join("AppData").join("Local"),
                xdg_config: root.join(".config"),
                home: root.clone(),
                claude_config_dir: None,
                codex_home: None,
                project_dir,
                root: Some(root),
            });
        }
        let home = if platform == Platform::Windows {
            env_dir("USERPROFILE").or_else(|| env_dir("HOME"))
        } else {
            env_dir("HOME")
        }
        .ok_or("cannot determine the home directory (set HOME, or USERPROFILE on Windows)")?;
        Ok(Ctx {
            platform,
            appdata: env_dir("APPDATA").unwrap_or_else(|| home.join("AppData").join("Roaming")),
            local_appdata: env_dir("LOCALAPPDATA").unwrap_or_else(|| home.join("AppData").join("Local")),
            xdg_config: env_dir("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config")),
            claude_config_dir: env_dir("CLAUDE_CONFIG_DIR"),
            codex_home: env_dir("CODEX_HOME"),
            home,
            project_dir,
            root: None,
        })
    }

    /// The per-user application data directory (`%APPDATA%`, `~/Library/Application Support`,
    /// `$XDG_CONFIG_HOME`).
    fn app_config(&self) -> PathBuf {
        match self.platform {
            Platform::Windows => self.appdata.clone(),
            Platform::MacOs => self.home.join("Library").join("Application Support"),
            Platform::Linux => self.xdg_config.clone(),
        }
    }

    /// The client's configuration file for `scope`, or `None` if it has no such scope.
    pub fn locate(&self, client: Client, scope: Scope) -> Option<Location> {
        if scope == Scope::Project {
            let rel: &[&str] = match client {
                Client::ClaudeCode => &[".mcp.json"],
                Client::ClaudeDesktop => return None,
                Client::VsCode => &[".vscode", "mcp.json"],
                Client::Cursor => &[".cursor", "mcp.json"],
                Client::Codex => &[".codex", "config.toml"],
            };
            let path = rel.iter().fold(self.project_dir.clone(), |p, c| p.join(c));
            let detected = path.is_file();
            return Some(Location { path, detected });
        }
        let loc = match client {
            Client::ClaudeCode => {
                let dir = self.claude_config_dir.clone();
                let path = dir.as_deref().unwrap_or(&self.home).join(".claude.json");
                let detected = path.is_file()
                    || dir.as_deref().is_some_and(Path::is_dir)
                    || self.home.join(".claude").is_dir();
                Location { path, detected }
            }
            Client::ClaudeDesktop => self.claude_desktop(),
            Client::VsCode => {
                let dir = self.app_config().join("Code").join("User");
                Location {
                    detected: dir.is_dir(),
                    path: dir.join("mcp.json"),
                }
            }
            Client::Cursor => {
                let dir = self.home.join(".cursor");
                Location {
                    detected: dir.is_dir(),
                    path: dir.join("mcp.json"),
                }
            }
            Client::Codex => {
                let dir = self
                    .codex_home
                    .clone()
                    .unwrap_or_else(|| self.home.join(".codex"));
                Location {
                    detected: dir.is_dir(),
                    path: dir.join("config.toml"),
                }
            }
        };
        Some(loc)
    }

    /// Whether this process sees Claude Desktop's MSIX package storage at the `%APPDATA%` paths:
    /// a process started by the packaged app (such as a terminal in its Code tab) has its
    /// `%APPDATA%` and `%LOCALAPPDATA%` writes redirected into the package, where other apps never
    /// read them. Detected by the package's private configuration and the
    /// `%APPDATA%` one being the same file (same bytes and modification time).
    pub fn msix_redirected(&self) -> bool {
        if self.platform != Platform::Windows {
            return false;
        }
        let plain = self.appdata.join("Claude").join("claude_desktop_config.json");
        let private = self
            .local_appdata
            .join("Packages")
            .join(CLAUDE_MSIX_FAMILY)
            .join("LocalCache")
            .join("Roaming")
            .join("Claude")
            .join("claude_desktop_config.json");
        let (Ok(a), Ok(b)) = (std::fs::metadata(&plain), std::fs::metadata(&private)) else {
            return false;
        };
        let same_time = matches!((a.modified(), b.modified()), (Ok(x), Ok(y)) if x == y);
        same_time
            && a.len() == b.len()
            && matches!((std::fs::read(&plain), std::fs::read(&private)), (Ok(x), Ok(y)) if x == y)
    }

    /// Whether `path` lies under `%APPDATA%` or `%LOCALAPPDATA%` (Windows).
    pub fn in_app_data(&self, path: &Path) -> bool {
        self.platform == Platform::Windows
            && (path.starts_with(&self.appdata) || path.starts_with(&self.local_appdata))
    }

    fn claude_desktop(&self) -> Location {
        let dir = self.app_config().join("Claude");
        let plain = dir.join("claude_desktop_config.json");
        if self.platform != Platform::Windows {
            return Location {
                detected: dir.is_dir(),
                path: plain,
            };
        }
        // The MSIX build virtualizes AppData: once it has saved its configuration it reads the
        // package-private copy and ignores the %APPDATA% file.
        let package = self.local_appdata.join("Packages").join(CLAUDE_MSIX_FAMILY);
        let private = package
            .join("LocalCache")
            .join("Roaming")
            .join("Claude")
            .join("claude_desktop_config.json");
        let detected = dir.is_dir() || package.is_dir();
        let path = if private.is_file() {
            private
        } else if plain.is_file() || !package.is_dir() {
            plain
        } else {
            private
        };
        Location { path, detected }
    }
}
