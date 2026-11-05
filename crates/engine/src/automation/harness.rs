//! Local MCP harness configuration for LLM coding agents (`Antigravity`, `Claude Code`,
//! `Claude Desktop`, `Codex`).
//!
//! Reads, inspects, and updates each harness's local configuration file so `figcraft-cli mcp
//! --attach <port>` (or headless `figcraft-cli mcp`) is registered under `figcraft`.
//! Because these files live in the user's home/config directories (`~/.gemini/antigravity/…`,
//! `~/.claude.json`, `~/.codex/config.toml`), file operations here use `#[allow(clippy::disallowed_methods)]`
//! just like preferences and installed plugins.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Supported LLM harnesses that speak the Model Context Protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HarnessKind {
    Antigravity,
    ClaudeCode,
    ClaudeDesktop,
    Codex,
}

impl HarnessKind {
    pub const ALL: [HarnessKind; 4] = [
        HarnessKind::Antigravity,
        HarnessKind::ClaudeCode,
        HarnessKind::ClaudeDesktop,
        HarnessKind::Codex,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Antigravity => "antigravity",
            Self::ClaudeCode => "claude-code",
            Self::ClaudeDesktop => "claude-desktop",
            Self::Codex => "codex",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Antigravity => "Antigravity",
            Self::ClaudeCode => "Claude Code",
            Self::ClaudeDesktop => "Claude Desktop",
            Self::Codex => "Codex",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "antigravity" | "agy" => Some(Self::Antigravity),
            "claude" | "claude-code" | "claudecode" => Some(Self::ClaudeCode),
            "claude-desktop" | "claudedesktop" => Some(Self::ClaudeDesktop),
            "codex" | "openai-codex" => Some(Self::Codex),
            _ => None,
        }
    }
}

/// Inspection status of a single LLM harness on this machine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessStatus {
    pub id: String,
    pub label: String,
    pub config_path: PathBuf,
    pub configured: bool,
    pub attached_port: Option<u16>,
    pub snippet: String,
}

/// Home directory for the current user.
fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

/// Default configuration file path for `kind` under `home` (or the current user's home folder).
pub fn config_path_in(kind: HarnessKind, home: &Path) -> PathBuf {
    match kind {
        HarnessKind::Antigravity => home.join(".gemini").join("antigravity").join("mcp_config.json"),
        HarnessKind::ClaudeCode => home.join(".claude.json"),
        HarnessKind::ClaudeDesktop => {
            #[cfg(target_os = "macos")]
            {
                home.join("Library")
                    .join("Application Support")
                    .join("Claude")
                    .join("claude_desktop_config.json")
            }
            #[cfg(target_os = "windows")]
            {
                std::env::var_os("APPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join("AppData").join("Roaming"))
                    .join("Claude")
                    .join("claude_desktop_config.json")
            }
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            {
                home.join(".config").join("Claude").join("claude_desktop_config.json")
            }
        }
        HarnessKind::Codex => home.join(".codex").join("config.toml"),
    }
}

/// Default configuration file path for `kind` on this system.
pub fn config_path(kind: HarnessKind) -> Option<PathBuf> {
    home_dir().map(|h| config_path_in(kind, &h))
}

/// Locate the `figcraft-cli` binary to invoke from MCP harness configs.
/// Checks alongside the current executable first, then common workspace target directories,
/// and falls back to `"figcraft-cli"` on `PATH`.
#[allow(clippy::disallowed_methods)]
pub fn detect_cli_binary() -> String {
    let bin_name = if cfg!(windows) { "figcraft-cli.exe" } else { "figcraft-cli" };
    if let Ok(exe) = std::env::current_exe() {
        if exe.file_name().and_then(|n| n.to_str()) == Some(bin_name) && exe.is_file() {
            return exe.to_string_lossy().into_owned();
        }
        if let Some(dir) = exe.parent() {
            let sibling = dir.join(bin_name);
            if sibling.is_file() {
                return sibling.to_string_lossy().into_owned();
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        for rel in [
            Path::new("target/release").join(bin_name),
            Path::new("target/debug").join(bin_name),
            Path::new("figcraft-ihpnull/target/release").join(bin_name),
            Path::new("figcraft-ihpnull/target/debug").join(bin_name),
        ] {
            let candidate = cwd.join(rel);
            if candidate.is_file() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    bin_name.to_string()
}

/// Build the args array for `figcraft-cli mcp` (`Some(port)` attaches to the running desktop app;
/// `None` runs a standalone headless MCP session).
pub fn mcp_args(port: Option<u16>) -> Vec<String> {
    match port {
        Some(p) => vec!["mcp".into(), "--attach".into(), p.to_string()],
        None => vec!["mcp".into()],
    }
}

/// Ready-to-copy configuration snippet for `kind`.
pub fn snippet(kind: HarnessKind, cli_bin: &str, port: Option<u16>) -> String {
    let args = mcp_args(port);
    match kind {
        HarnessKind::Antigravity | HarnessKind::ClaudeCode | HarnessKind::ClaudeDesktop => {
            let doc = json!({
                "mcpServers": {
                    "figcraft": {
                        "command": cli_bin,
                        "args": args,
                    }
                }
            });
            serde_json::to_string_pretty(&doc).unwrap_or_default()
        }
        HarnessKind::Codex => {
            let quoted_args: Vec<String> = args.iter().map(|a| format!("{a:?}")).collect();
            format!(
                "[mcp_servers.figcraft]\ncommand = {:?}\nargs = [{}]\n",
                cli_bin,
                quoted_args.join(", ")
            )
        }
    }
}

/// Inspect a single harness's configuration at `path`.
#[allow(clippy::disallowed_methods)]
pub fn inspect_at(kind: HarnessKind, path: &Path, cli_bin: &str, port: Option<u16>) -> HarnessStatus {
    let snip = snippet(kind, cli_bin, port);
    let content = std::fs::read_to_string(path).ok();
    let (configured, attached_port) = match (kind, content.as_deref()) {
        (HarnessKind::Antigravity | HarnessKind::ClaudeCode | HarnessKind::ClaudeDesktop, Some(text)) => {
            inspect_json(text)
        }
        (HarnessKind::Codex, Some(text)) => inspect_codex_toml(text),
        (_, None) => (false, None),
    };
    HarnessStatus {
        id: kind.id().to_string(),
        label: kind.label().to_string(),
        config_path: path.to_path_buf(),
        configured,
        attached_port,
        snippet: snip,
    }
}

fn inspect_json(text: &str) -> (bool, Option<u16>) {
    let Ok(val) = serde_json::from_str::<Value>(text) else {
        return (false, None);
    };
    let Some(entry) = val.get("mcpServers").and_then(|s| s.get("figcraft")) else {
        return (false, None);
    };
    let has_cmd = entry.get("command").and_then(Value::as_str).is_some_and(|c| !c.is_empty());
    let mut attached_port = None;
    if let Some(args) = entry.get("args").and_then(Value::as_array) {
        let mut iter = args.iter().filter_map(Value::as_str);
        while let Some(arg) = iter.next() {
            if arg == "--attach"
                && let Some(p) = iter.next().and_then(|v| v.parse::<u16>().ok())
            {
                attached_port = Some(p);
            } else if let Some(rest) = arg.strip_prefix("--attach=")
                && let Ok(p) = rest.parse::<u16>()
            {
                attached_port = Some(p);
            }
        }
    }
    (has_cmd, attached_port)
}

fn inspect_codex_toml(text: &str) -> (bool, Option<u16>) {
    let mut in_section = false;
    let mut has_cmd = false;
    let mut attached_port = None;
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_section = line == "[mcp_servers.figcraft]";
            continue;
        }
        if !in_section || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            let v = v.trim();
            if k == "command" && v.len() >= 2 {
                has_cmd = true;
            } else if k == "args" {
                // Parse port after "--attach" if present.
                let tokens: Vec<&str> = v
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .split(',')
                    .map(|t| t.trim().trim_matches('"').trim_matches('\''))
                    .collect();
                for pair in tokens.windows(2) {
                    if pair[0] == "--attach"
                        && let Ok(p) = pair[1].parse::<u16>()
                    {
                        attached_port = Some(p);
                    }
                }
            }
        }
    }
    (has_cmd, attached_port)
}

/// Inspect all supported harnesses using the current user's home directory.
pub fn inspect_all(cli_bin: &str, port: Option<u16>) -> Vec<HarnessStatus> {
    let Some(home) = home_dir() else {
        return Vec::new();
    };
    HarnessKind::ALL
        .into_iter()
        .map(|k| inspect_at(k, &config_path_in(k, &home), cli_bin, port))
        .collect()
}

/// Install or update the `figcraft` MCP server entry in `path` for `kind`, preserving all other
/// settings in the file.
#[allow(clippy::disallowed_methods)]
pub fn install_at(kind: HarnessKind, path: &Path, cli_bin: &str, port: Option<u16>) -> Result<HarnessStatus, String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| format!("couldn't create {}: {e}", parent.display()))?;
    }
    let existing = if path.exists() {
        Some(std::fs::read_to_string(path).map_err(|e| format!("couldn't read {}: {e}", path.display()))?)
    } else {
        None
    };
    let updated = match kind {
        HarnessKind::Antigravity | HarnessKind::ClaudeCode | HarnessKind::ClaudeDesktop => {
            merge_json_config(existing.as_deref(), cli_bin, port)?
        }
        HarnessKind::Codex => merge_codex_toml(existing.as_deref(), cli_bin, port),
    };
    std::fs::write(path, updated).map_err(|e| format!("couldn't write {}: {e}", path.display()))?;
    Ok(inspect_at(kind, path, cli_bin, port))
}

/// Install or update the `figcraft` MCP server entry in `kind`'s default config path.
pub fn install(kind: HarnessKind, cli_bin: &str, port: Option<u16>) -> Result<HarnessStatus, String> {
    let path = config_path(kind).ok_or_else(|| "couldn't determine your home directory".to_string())?;
    install_at(kind, &path, cli_bin, port)
}

fn merge_json_config(existing: Option<&str>, cli_bin: &str, port: Option<u16>) -> Result<String, String> {
    let mut root = match existing {
        Some(s) if !s.trim().is_empty() => serde_json::from_str::<Value>(s)
            .map_err(|e| format!("existing config is not valid JSON: {e}"))?,
        _ => json!({}),
    };
    let obj = root
        .as_object_mut()
        .ok_or_else(|| "existing config root is not a JSON object".to_string())?;
    let servers = obj
        .entry("mcpServers")
        .or_insert_with(|| json!({}));
    let servers_obj = servers
        .as_object_mut()
        .ok_or_else(|| "`mcpServers` in config is not a JSON object".to_string())?;
    servers_obj.insert(
        "figcraft".to_string(),
        json!({
            "command": cli_bin,
            "args": mcp_args(port),
        }),
    );
    let mut out = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    out.push('\n');
    Ok(out)
}

fn merge_codex_toml(existing: Option<&str>, cli_bin: &str, port: Option<u16>) -> String {
    let block = snippet(HarnessKind::Codex, cli_bin, port);
    let Some(text) = existing.filter(|s| !s.trim().is_empty()) else {
        return block;
    };
    let mut out_lines: Vec<&str> = Vec::new();
    let mut in_figcraft = false;
    let mut replaced = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if trimmed == "[mcp_servers.figcraft]" {
                in_figcraft = true;
                if !replaced {
                    for bl in block.trim_end().lines() {
                        out_lines.push(bl);
                    }
                    replaced = true;
                }
                continue;
            } else {
                in_figcraft = false;
            }
        }
        if !in_figcraft {
            out_lines.push(line);
        }
    }
    let mut result = out_lines.join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    if !replaced {
        if !result.ends_with("\n\n") {
            result.push('\n');
        }
        result.push_str(&block);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_json_preserving_other_servers_and_keys() {
        let existing = r#"{
  "theme": "dark",
  "mcpServers": {
    "other": {
      "command": "npx",
      "args": ["-y", "other-mcp"]
    }
  }
}"#;
        let merged = merge_json_config(Some(existing), "/usr/local/bin/figcraft-cli", Some(47800)).unwrap();
        let val: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(val["theme"], "dark");
        assert_eq!(val["mcpServers"]["other"]["command"], "npx");
        assert_eq!(val["mcpServers"]["figcraft"]["command"], "/usr/local/bin/figcraft-cli");
        assert_eq!(val["mcpServers"]["figcraft"]["args"], json!(["mcp", "--attach", "47800"]));
        let (ok, port) = inspect_json(&merged);
        assert!(ok);
        assert_eq!(port, Some(47800));
    }

    #[test]
    fn merges_codex_toml_preserving_existing_sections() {
        let existing = r#"model = "o3"

[mcp_servers.other]
command = "other"
args = ["serve"]
"#;
        let merged = merge_codex_toml(Some(existing), "/bin/figcraft-cli", Some(47800));
        assert!(merged.contains("model = \"o3\""));
        assert!(merged.contains("[mcp_servers.other]"));
        assert!(merged.contains("[mcp_servers.figcraft]"));
        let (ok, port) = inspect_codex_toml(&merged);
        assert!(ok);
        assert_eq!(port, Some(47800));

        // Updating again replaces the section in place without duplicating it.
        let updated = merge_codex_toml(Some(&merged), "/bin/figcraft-cli", Some(47999));
        assert_eq!(updated.matches("[mcp_servers.figcraft]").count(), 1);
        let (ok2, port2) = inspect_codex_toml(&updated);
        assert!(ok2);
        assert_eq!(port2, Some(47999));
    }
}
