use serde::Serialize;
use std::path::PathBuf;
use std::process::Command;

use super::{codex_dir, codex_home, config_path, data_root, exe_name, user_codex_home};

#[derive(Serialize, Clone, Default)]
pub struct CodexStatus {
    /// 我们已经解压好、可以直接运行的 Codex 版本目录
    pub prepared: bool,
    pub version: Option<String>,
    pub path: Option<String>,
    pub dir: Option<String>,
}

#[derive(Serialize, Clone, Default)]
pub struct DesktopStatus {
    /// 当前平台是否支持检测桌面版（Linux 不检测）
    pub supported: bool,
    pub found: bool,
    /// 界面上直接显示的一句话（例如「已检测到 Codex 桌面版 26.1027.3384.0」）
    pub detail: String,
    /// Appx 包名（Windows）/ .app 路径（macOS）
    pub package: Option<String>,
    /// 版本号（能从 Appx 读到时）
    pub version: Option<String>,
}

#[derive(Serialize, Clone, Default)]
pub struct SystemCodex {
    /// 用户系统里另外还装着的 Codex CLI（只读探测，不用它跑）
    pub cli_found: bool,
    pub cli_path: Option<String>,
    pub cli_version: Option<String>,
    /// 桌面版
    pub desktop_supported: bool,
    pub desktop_found: bool,
    pub desktop_detail: String,
    pub desktop_package: Option<String>,
    pub desktop_version: Option<String>,
}

#[derive(Serialize, Clone, Default)]
pub struct EnvStatus {
    pub codex: CodexStatus,
    pub system: SystemCodex,
    pub os: String,
    pub data_root: String,
    pub codex_home: String,
    pub config_path: String,
    pub config_exists: bool,
    pub user_codex_home: String,
}

/// 当前主工具是否已准备好（给 test / launch 用）。
pub fn prepared_version() -> Option<(String, PathBuf)> {
    super::tools::primary().detect()
}

fn version_of(exe: &PathBuf) -> Option<String> {
    let mut cmd = Command::new(exe);
    cmd.arg("--version");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().ok()?;
    let text = if out.stdout.is_empty() {
        String::from_utf8_lossy(&out.stderr).to_string()
    } else {
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|s| s.to_string())
}

fn check_codex() -> CodexStatus {
    match prepared_version() {
        Some((ver, exe)) => CodexStatus {
            prepared: true,
            version: version_of(&exe).or(Some(ver)),
            dir: exe.parent().and_then(|p| p.parent()).map(|p| p.display().to_string()),
            path: Some(exe.display().to_string()),
        },
        None => CodexStatus {
            dir: Some(codex_dir().display().to_string()),
            ..Default::default()
        },
    }
}

fn path_lookup(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let cand = dir.join(name);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

fn check_system() -> SystemCodex {
    let mut s = SystemCodex::default();
    // 只读探测：用户系统里可能另外装过 Codex，界面里只作提示，不用它运行。
    let mut candidates: Vec<PathBuf> = vec![
        user_codex_home()
            .join("packages")
            .join("standalone")
            .join("current")
            .join("bin")
            .join(exe_name()),
        super::home_dir().join(".local").join("bin").join(exe_name()),
    ];
    if cfg!(target_os = "windows") {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                PathBuf::from(local)
                    .join("Programs")
                    .join("OpenAI")
                    .join("Codex")
                    .join("bin")
                    .join(exe_name()),
            );
        }
    }
    if let Some(p) = candidates.into_iter().find(|p| p.is_file()) {
        s.cli_found = true;
        s.cli_version = version_of(&p);
        s.cli_path = Some(p.display().to_string());
    } else if let Some(p) = path_lookup(exe_name()) {
        s.cli_found = true;
        s.cli_version = version_of(&p);
        s.cli_path = Some(p.display().to_string());
    }

    let desktop = check_desktop();
    s.desktop_supported = desktop.supported;
    s.desktop_found = desktop.found;
    s.desktop_detail = desktop.detail;
    s.desktop_package = desktop.package;
    s.desktop_version = desktop.version;
    s
}

/// 解析 `Get-AppxPackage | ForEach-Object { "$($_.Name)|$($_.Version)" }` 的输出。
/// 也兼容 PowerShell 默认表格格式（`Name : xxx` / `Version : yyy` 多行）。
/// 抽成纯函数是为了能单测——远程验收机器上 agent 以 SYSTEM 运行，看不到用户装的 Appx。
fn parse_appx_output(raw: &str) -> DesktopStatus {
    let raw = raw.trim();
    if raw.is_empty() {
        return DesktopStatus {
            supported: true,
            found: false,
            detail: "没检测到 Codex 桌面版，可以到 Microsoft Store 搜 Codex 安装".to_string(),
            package: None,
            version: None,
        };
    }
    let (name, ver) = if raw.contains('|') {
        let mut it = raw.split('|');
        (
            it.next().unwrap_or("").trim().to_string(),
            it.next().unwrap_or("").trim().to_string(),
        )
    } else {
        let mut name = String::new();
        let mut ver = String::new();
        for line in raw.lines() {
            let l = line.trim();
            if name.is_empty() {
                if let Some((_, v)) = l.strip_prefix("Name").and_then(|r| r.split_once(':')) {
                    name = v.trim().to_string();
                }
            }
            if ver.is_empty() {
                if let Some((_, v)) = l.strip_prefix("Version").and_then(|r| r.split_once(':')) {
                    ver = v.trim().to_string();
                }
            }
        }
        (name, ver)
    };
    DesktopStatus {
        supported: true,
        found: true,
        detail: format!(
            "已检测到 Codex 桌面版{}",
            if ver.is_empty() { String::new() } else { format!(" {ver}") }
        ),
        package: if name.is_empty() { None } else { Some(name) },
        version: if ver.is_empty() { None } else { Some(ver) },
    }
}

#[cfg(target_os = "windows")]
fn check_desktop() -> DesktopStatus {
    // 覆盖两种正式名字：早期独立 Codex 版（OpenAI.Codex）与新版合并的 ChatGPT 桌面版（*ChatGPT*）。
    // 输出 `Name|Version`，界面就能显示「已检测到 … 26.1027.3384.0」。
    let script = "Get-AppxPackage | Where-Object { $_.Name -like 'OpenAI.Codex*' -or $_.Name -like '*ChatGPT*' } | Select-Object -First 1 | ForEach-Object { \"$($_.Name)|$($_.Version)\" }";
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script,
    ]);
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let raw = cmd
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    parse_appx_output(&raw)
}

#[cfg(target_os = "macos")]
fn check_desktop() -> DesktopStatus {
    for p in ["/Applications/Codex.app", "/Applications/ChatGPT.app"] {
        if std::path::Path::new(p).exists() {
            return DesktopStatus {
                supported: true,
                found: true,
                detail: "已检测到 Codex 桌面版".to_string(),
                package: Some(p.to_string()),
                version: None,
            };
        }
    }
    DesktopStatus {
        supported: true,
        found: false,
        detail: "没检测到 Codex 桌面版，可以从官网下载安装".to_string(),
        package: None,
        version: None,
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn check_desktop() -> DesktopStatus {
    DesktopStatus {
        supported: false,
        found: false,
        detail: "当前系统没有 Codex 桌面版".to_string(),
        package: None,
        version: None,
    }
}

pub fn check_environment() -> EnvStatus {
    EnvStatus {
        codex: check_codex(),
        system: check_system(),
        os: std::env::consts::OS.to_string(),
        data_root: data_root().display().to_string(),
        codex_home: codex_home().display().to_string(),
        config_path: config_path().display().to_string(),
        config_exists: config_path().is_file(),
        user_codex_home: user_codex_home().display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_appx_pipe_format_with_version() {
        let st = parse_appx_output("OpenAI.Codex|26.1027.3384.0");
        assert!(st.found);
        assert_eq!(st.version.as_deref(), Some("26.1027.3384.0"));
        assert_eq!(st.package.as_deref(), Some("OpenAI.Codex"));
        assert_eq!(st.detail, "已检测到 Codex 桌面版 26.1027.3384.0");
    }

    #[test]
    fn parses_appx_table_format_from_real_machine() {
        // 郭老师 Win10 真机上 `Get-AppxPackage` 的原始输出（已确认能命中）
        let raw = "Name            : OpenAI.Codex\nVersion         : 26.1027.3384.0\nPackageFullName : OpenAI.Codex_26.1027.3384.0_x64__2p2nqsd0c76g0";
        let st = parse_appx_output(raw);
        assert!(st.found);
        assert_eq!(st.package.as_deref(), Some("OpenAI.Codex"));
        assert_eq!(st.version.as_deref(), Some("26.1027.3384.0"));
        assert_eq!(st.detail, "已检测到 Codex 桌面版 26.1027.3384.0");
    }

    #[test]
    fn parses_new_chatgpt_desktop_name() {
        let st = parse_appx_output("OpenAI.ChatGPT-Desktop|1.2026.1.0");
        assert!(st.found);
        assert_eq!(st.package.as_deref(), Some("OpenAI.ChatGPT-Desktop"));
        assert_eq!(st.detail, "已检测到 Codex 桌面版 1.2026.1.0");
    }

    #[test]
    fn missing_desktop_is_not_silent() {
        let st = parse_appx_output("");
        assert!(!st.found);
        assert!(st.detail.contains("没检测到"), "{}", st.detail);
        assert!(st.detail.contains("Store"), "文案里要告诉用户去哪装：{}", st.detail);
        assert!(st.version.is_none());
    }

    #[test]
    fn name_without_version_still_ok() {
        let st = parse_appx_output("OpenAI.Codex|");
        assert!(st.found);
        assert_eq!(st.detail, "已检测到 Codex 桌面版");
        assert!(st.version.is_none());
    }
}
