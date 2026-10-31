use std::path::PathBuf;
use std::process::Command;

use super::detect;
use super::tools;

/// 生成一个启动脚本，再交给系统终端打开：
/// 这样不用在命令里拼一堆引号，Windows 的 cmd 与 macOS/Linux 的 shell 行为都可控。
fn write_launcher(exe: &PathBuf, codex_home: &PathBuf, extra_args: &[String]) -> Result<PathBuf, String> {
    let extra = extra_args.join(" ");
    let extra_win = extra_args.join(" ");
    let dir = super::data_root();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败：{e}"))?;

    #[cfg(target_os = "windows")]
    {
        let path = dir.join("launch-codex.cmd");
        // 有参数时按参数跑（便于自检 --version），否则显式带上 -m：
        // Codex 的 TUI **不认 config.toml 里的 model**（未知模型会退回它内置默认 gpt-5.5，
        // 导致 our gateway 报 503 no available channel），只有 `codex exec` 认。所以启动时必须显式指定。
        let script = format!(
            "@echo off\r\nset \"CODEX_HOME={}\"\r\ncd /d \"%USERPROFILE%\"\r\nif \"%~1\"==\"\" ( \"{exe}\" {extra_win} ) else ( \"{exe}\" %* )\r\n",
            codex_home.display(),
            exe = exe.display(),
            extra_win = extra_win,
        );
        std::fs::write(&path, script).map_err(|e| format!("写入启动脚本失败：{e}"))?;
        Ok(path)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let path = dir.join("launch-codex.sh");
        // 带参数时按参数跑（便于自检 `--version`）；否则显式给 `-m`：
        // Codex 的 TUI 不认 config.toml 里的 model（未知模型会退回它内置的默认模型，
        // 撞出 503 no available channel），`codex exec` 才认。启动这一步必须显式指定。
        let script = format!(
            "#!/bin/sh\n# 由「aiquick」生成\nexport CODEX_HOME='{}'\ncd \"$HOME\"\nif [ \"$#\" -gt 0 ]; then\n  exec '{}' \"$@\"\nfi\nexec '{}' {}\n",
            codex_home.display(),
            exe.display(),
            exe.display(),
            extra
        );
        std::fs::write(&path, script).map_err(|e| format!("写入启动脚本失败：{e}"))?;
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
        }
        Ok(path)
    }
}

/// 配置里写的模型（没有就退回默认），启动时显式传给 Codex。
fn configured_model() -> String {
    let path = super::config_path();
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| t.parse::<toml_edit::DocumentMut>().ok())
        .and_then(|d| d.get("model").and_then(|i| i.as_str()).map(|s| s.to_string()))
        .unwrap_or_else(|| tools::primary().default_model().to_string())
}

/// 「启动 Codex」：带上我们的 CODEX_HOME，在用户主目录里开一个终端窗口跑 Codex。
pub fn launch_codex() -> Result<String, String> {
    let Some((ver, exe)) = detect::prepared_version() else {
        return Err("Codex 还没准备好，请先点上面的准备按钮".to_string());
    };
    let home = super::codex_home();
    let model = configured_model();
    // 额外参数由 tool 定义（Codex 必须显式 `-m`）
    let extra = tools::primary().launch_args(&model);
    let launcher = write_launcher(&exe, &home, &extra)?;

    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("cmd");
        cmd.args(["/c", "start", "", &launcher.display().to_string()]);
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        cmd.spawn().map_err(|e| format!("打开终端失败：{e}"))?;
        return Ok(format!("已在新窗口启动 Codex {ver}"));
    }

    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("open");
        cmd.args(["-a", "Terminal", &launcher.display().to_string()]);
        cmd.spawn().map_err(|e| format!("打开终端失败：{e}"))?;
        return Ok(format!("已在终端窗口启动 Codex {ver}"));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let p = launcher.display().to_string();
        let candidates: Vec<(&str, Vec<&str>)> = vec![
            ("x-terminal-emulator", vec!["-e", &p]),
            ("gnome-terminal", vec!["--", &p]),
            ("konsole", vec!["-e", &p]),
            ("xfce4-terminal", vec!["-x", &p]),
            ("mate-terminal", vec!["-x", &p]),
            ("lxterminal", vec!["-e", &p]),
            ("xterm", vec!["-e", &p]),
        ];
        let mut tried: Vec<String> = Vec::new();
        for (bin, args) in candidates {
            let mut cmd = Command::new(bin);
            cmd.args(args);
            match cmd.spawn() {
                Ok(mut child) => {
                    // 终端程序存在但立刻失败（没 DISPLAY、缺参数）时继续试下一个，
                    // 别谎报「已启动」。
                    std::thread::sleep(std::time::Duration::from_millis(700));
                    match child.try_wait() {
                        Ok(Some(st)) if !st.success() => {
                            tried.push(format!("{bin}(退出码{})", st.code().unwrap_or(-1)));
                        }
                        _ => return Ok(format!("已在 {bin} 窗口启动 Codex {ver}")),
                    }
                }
                Err(e) => tried.push(format!("{bin}({e})")),
            }
        }
        Err(format!(
            "没找到可用的终端程序（试过：{}）。你也可以自己开终端，把 CODEX_HOME 指到 {} 后运行 {}",
            tried.join(" / "),
            home.display(),
            exe.display()
        ))
    }
}

/// 「打开 Codex 桌面版」：走 Appx 的 shell:AppsFolder（MSIX 应用不支持直接 Start-Process exe）。
#[cfg(target_os = "windows")]
pub fn launch_desktop() -> Result<String, String> {
    let script = r#"$ErrorActionPreference='Stop'
$p = Get-AppxPackage | Where-Object { $_.Name -like 'OpenAI.Codex*' -or $_.Name -like '*ChatGPT*' } | Select-Object -First 1
if (-not $p) { throw 'no-desktop-app' }
$id = (Get-AppxPackageManifest $p).Package.Applications.Application.Id
if ($id -is [array]) { $id = $id[0] }
Start-Process explorer.exe ('shell:AppsFolder\' + $p.PackageFamilyName + '!' + $id)
Write-Output $p.Name"#;
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script,
    ]);
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|e| format!("调用 PowerShell 失败：{e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stdout).to_string()
            + &String::from_utf8_lossy(&out.stderr);
        if err.contains("no-desktop-app") {
            return Err("没找到已安装的 Codex 桌面版（含新版 ChatGPT 桌面应用）".to_string());
        }
        return Err(format!("打开桌面版失败：{}", err.trim()));
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(format!("已打开 {}", if name.is_empty() { "Codex 桌面版".to_string() } else { name }))
}

#[cfg(not(target_os = "windows"))]
pub fn launch_desktop() -> Result<String, String> {
    Err("当前系统没有 Codex 桌面版应用可启动".to_string())
}

/// 「清理并退出」：删掉我们自己产生的全部文件。用户自己的 `~/.codex` 不碰。
pub fn cleanup() -> Result<String, String> {
    super::prepare::cleanup()
}
