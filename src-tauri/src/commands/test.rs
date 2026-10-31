use serde::Serialize;
use serde_json::Value;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::config::{self};
use super::tools;
use super::detect;

#[derive(Serialize, Clone, Default)]
pub struct TestResult {
    pub ok: bool,
    /// ok / auth / no_channel / network / not_prepared / config / unknown
    pub category: String,
    pub message: String,
    pub model: String,
    pub elapsed_ms: u64,
    /// 实际执行的命令（可复制，便于排查）
    pub command: String,
    /// 原始输出（可复制错误文本）
    pub output: String,
    /// 实测回复内容
    pub reply: Option<String>,
}

/// 带超时地跑一个命令，返回 (退出码, 合并后的输出)。
fn run_with_timeout(cmd: &mut Command, timeout: Duration) -> Result<(i32, String), String> {
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().map_err(|e| format!("启动失败：{e}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => {
                let out = child.wait_with_output().map_err(|e| e.to_string())?;
                let mut text = String::from_utf8_lossy(&out.stdout).to_string();
                let err = String::from_utf8_lossy(&out.stderr).to_string();
                if !err.trim().is_empty() {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(&err);
                }
                return Ok((st.code().unwrap_or(-1), text));
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("超时（{} 秒无结果）", timeout.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(120));
            }
            Err(e) => return Err(format!("等待进程失败：{e}")),
        }
    }
}

fn classify(text: &str) -> Option<(&'static str, String)> {
    let t = text.to_lowercase();
    if t.contains("no available channel") {
        return Some((
            "no_channel",
            "模型在当前分组没有可用渠道（上游 503），请换一个模型".to_string(),
        ));
    }
    if t.contains("401")
        || t.contains("unauthorized")
        || t.contains("invalid_api_key")
        || t.contains("invalid api key")
        || t.contains("incorrect api key")
        || t.contains("invalid token")
    {
        return Some(("auth", "鉴权失败：Key 无效或已过期".to_string()));
    }
    if t.contains("429") || t.contains("quota") || t.contains("insufficient") {
        return Some(("auth", "额度不足或请求过于频繁（429）".to_string()));
    }
    if t.contains("404") || t.contains("model_not_found") || t.contains("does not exist") {
        return Some((
            "no_channel",
            "模型不存在，或该 Key 所属分组没有这个模型".to_string(),
        ));
    }
    if t.contains("timed out")
        || t.contains("timeout")
        || t.contains("dns")
        || t.contains("connection refused")
        || t.contains("connection reset")
        || t.contains("failed to connect")
        || t.contains("error sending request")
        || t.contains("network")
    {
        return Some(("network", "网络不通：连不上 new-api.hsfp.cn".to_string()));
    }
    None
}

/// 直接用 HTTP 打一次 `/v1/responses`（同一把 Key、同一个模型）。
/// 不引入额外 HTTP 依赖：curl 在 macOS / Linux / Win10 1803+ 都是自带的。
fn probe_http(model: &str, key: &str) -> Result<(u16, String, String), String> {
    let body = serde_json::json!({
        "model": model,
        "input": "reply with only the word OK",
        "max_output_tokens": 64
    })
    .to_string();
    let mut cmd = Command::new("curl");
    cmd.args([
        "-sS",
        "-m",
        "90",
        "-w",
        "\n%{http_code}",
        "-H",
        &format!("Authorization: Bearer {key}"),
        "-H",
        "Content-Type: application/json",
        "-X",
        "POST",
        &format!("{}/responses", config::BASE_URL),
        "-d",
        &body,
    ]);
    let command_line = format!(
        "curl -sS -m 90 -X POST {}/responses -H 'Authorization: Bearer ***' -d '{}'",
        config::BASE_URL,
        body
    );
    let (code, text) = run_with_timeout(&mut cmd, Duration::from_secs(120))?;
    if code != 0 {
        return Err(format!("curl 退出码 {code}：{}", short(&text, 200)));
    }
    let trimmed = text.trim_end();
    match trimmed.rsplit_once('\n') {
        Some((body, status)) => {
            let http_code: u16 = status.trim().parse().unwrap_or(0);
            Ok((http_code, body.to_string(), command_line))
        }
        None => Ok((0, trimmed.to_string(), command_line)),
    }
}

fn extract_text(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    if let Some(arr) = v.get("output").and_then(|o| o.as_array()) {
        for item in arr {
            if let Some(contents) = item.get("content").and_then(|c| c.as_array()) {
                for c in contents {
                    if let Some(t) = c.get("text").and_then(|t| t.as_str()) {
                        return Some(t.trim().to_string());
                    }
                }
            }
        }
    }
    v.get("output_text")
        .and_then(|t| t.as_str())
        .map(|t| t.trim().to_string())
}

fn short(s: &str, n: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= n {
        t.to_string()
    } else {
        t.chars().take(n).collect::<String>() + "…"
    }
}

pub async fn run_test(key: Option<String>) -> TestResult {
    tauri::async_runtime::spawn_blocking(move || run_test_blocking(key))
        .await
        .unwrap_or_else(|e| TestResult {
            ok: false,
            category: "unknown".into(),
            message: format!("测试线程异常：{e}"),
            ..Default::default()
        })
}

/// 同步版：界面和无界面验收共用同一套逻辑。
pub fn run_test_blocking(key: Option<String>) -> TestResult {
    let start = Instant::now();
    let key = key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());

    // 传了 Key 就先按同一把 Key 落配置，保证「测的就是写进去的」。
    if let Some(k) = key.as_deref() {
        if let Err(e) = config::write_isolated(k) {
            return TestResult {
                ok: false,
                category: "config".into(),
                message: e,
                elapsed_ms: start.elapsed().as_millis() as u64,
                ..Default::default()
            };
        }
    }

    let tool = tools::primary();
    let info = config::read_config();
    let model = info
        .model
        .clone()
        .unwrap_or_else(|| tool.default_model().to_string());

    let Some((key, _)) = read_key() else {
        return TestResult {
            ok: false,
            category: "config".into(),
            message: "还没填 Key，请先在上面填入并点「保存配置」".to_string(),
            model,
            elapsed_ms: start.elapsed().as_millis() as u64,
            ..Default::default()
        };
    };

    // 主路径：用我们自己解压的 Codex、带 CODEX_HOME 真跑一次（端到端验证）
    if let Some((ver, exe)) = detect::prepared_version() {
        let (args, _) = tool.verify_args(&model);
        let mut cmd = Command::new(&exe);
        if let Some((var, home)) = tool.env_var() {
            cmd.env(var, &home);
        }
        cmd.current_dir(super::home_dir());
        // 必须显式 `-m`：Codex 会把「上次在 TUI 里选的模型」持久化在它自己的状态里、
        // 优先级高于 config.toml，不指定就可能拿着别的模型去打我们的网关（撞 503 no available channel）。
        cmd.args(&args);
        let env_prefix = tool
            .env_var()
            .map(|(v, p)| format!("{v}={} ", p.display()))
            .unwrap_or_default();
        let command_line = format!("{}{} {}", env_prefix, exe.display(), args.join(" "));
        match run_with_timeout(&mut cmd, Duration::from_secs(180)) {
            Ok((code, text)) => {
                let elapsed = start.elapsed().as_millis() as u64;
                if code == 0 && text.to_lowercase().contains("ok") {
                    return TestResult {
                        ok: true,
                        category: "ok".into(),
                        message: format!(
                            "实测通过：{} {ver} 用模型 {model} 真的回了一句",
                            tool.name()
                        ),
                        reply: Some(short(&meaningful(&text), 200)),
                        model,
                        elapsed_ms: elapsed,
                        command: command_line,
                        output: text,
                    };
                }
                let http = probe_http(&model, &key);
                let (cat, msg) = classify(&text).unwrap_or_else(|| match &http {
                    Ok((200, body, _)) => (
                        "unknown",
                        format!(
                            "配置本身是通的（HTTP 200：{}），但 codex 命令没跑通",
                            short(&extract_text(body).unwrap_or_default(), 40)
                        ),
                    ),
                    Ok((401, _, _)) | Ok((403, _, _)) => (
                        "auth",
                        "鉴权失败：Key 无效、过期或没有该分组权限".to_string(),
                    ),
                    Ok((c @ (502 | 503), _, _)) => (
                        "no_channel",
                        format!("模型 {model} 当前没有可用渠道（HTTP {c}）"),
                    ),
                    Ok((c, _, _)) => ("unknown", format!("上游返回 HTTP {c}")),
                    Err(e) => ("network", format!("网络不通：{e}")),
                });
                let detail = match &http {
                    Ok((_, body, _)) if !body.is_empty() => {
                        format!("\n上游返回：{}", short(body, 300))
                    }
                    Err(e) => format!("\nHTTP 探测失败：{e}"),
                    _ => String::new(),
                };
                return TestResult {
                    ok: false,
                    category: cat.to_string(),
                    message: format!("codex 命令失败（退出码 {code}）：{msg}{detail}"),
                    model,
                    elapsed_ms: elapsed,
                    command: command_line,
                    output: text,
                    reply: None,
                };
            }
            Err(e) => {
                let (cat, msg) = classify(&e).unwrap_or(("network", format!("执行失败：{e}")));
                return TestResult {
                    ok: false,
                    category: cat.to_string(),
                    message: msg,
                    model,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    command: command_line,
                    output: e,
                    reply: None,
                };
            }
        }
    }

    // 还没准备 Codex：至少用同一把 Key 验一次 Key 与模型通不通
    match probe_http(&model, &key) {
        Ok((200, body, cmdline)) => TestResult {
            ok: true,
            category: "ok".into(),
            message: "已用你的 Key 直连验证：Key 与模型可用（本机没装 CLI，桌面版会用同一个 Key）".to_string(),
            reply: Some(short(
                &extract_text(&body).unwrap_or_else(|| "OK".into()),
                200,
            )),
            model,
            elapsed_ms: start.elapsed().as_millis() as u64,
            command: cmdline,
            output: body,
        },
        Ok((code, body, cmdline)) => {
            let (cat, msg) = classify(&body)
                .map(|(c, m)| (c.to_string(), m))
                .unwrap_or_else(|| {
                    (
                        if code == 401 || code == 403 {
                            "auth".to_string()
                        } else {
                            "unknown".to_string()
                        },
                        format!("上游返回 HTTP {code}"),
                    )
                });
            TestResult {
                ok: false,
                category: cat,
                message: format!("{msg}\n上游返回：{}", short(&body, 300)),
                model,
                elapsed_ms: start.elapsed().as_millis() as u64,
                command: cmdline,
                output: body,
                reply: None,
            }
        }
        Err(e) => {
            let (cat, msg) = classify(&e).unwrap_or(("network", format!("网络不通：{e}")));
            TestResult {
                ok: false,
                category: cat.to_string(),
                message: msg,
                model,
                elapsed_ms: start.elapsed().as_millis() as u64,
                command: String::new(),
                output: e,
                reply: None,
            }
        }
    }
}

fn read_key() -> Option<(String, String)> {
    let path = super::config_path();
    let text = std::fs::read_to_string(&path).ok()?;
    let doc: toml_edit::DocumentMut = text.parse().ok()?;
    let auth = doc
        .get("model_providers")?
        .as_table_like()?
        .get(config::PROVIDER_ID)?
        .as_table_like()?
        .get("http_headers")?
        .get("Authorization")?
        .as_str()?
        .trim_start_matches("Bearer ")
        .trim()
        .to_string();
    if auth.is_empty() {
        None
    } else {
        Some((auth, path.display().to_string()))
    }
}

fn meaningful(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    // `codex exec` 的输出：正文在最后一段 `codex` 标记之后、`tokens used` 之前。
    if let Some(idx) = lines.iter().rposition(|l| *l == "codex") {
        let body: Vec<&str> = lines[idx + 1..]
            .iter()
            .copied()
            .take_while(|l| *l != "tokens used")
            .filter(|l| !l.is_empty())
            .collect();
        if !body.is_empty() {
            return body.join(" ");
        }
    }
    // 兜底：滤掉噪音行
    lines
        .iter()
        .copied()
        .filter(|l| !l.is_empty())
        .filter(|l| {
            let low = l.to_lowercase();
            !low.starts_with("warning")
                && !low.starts_with("reading")
                && !low.starts_with("openai codex")
                && !low.starts_with("workdir")
                && !low.starts_with("model:")
                && !low.starts_with("provider:")
                && !low.starts_with("approval")
                && !low.starts_with("sandbox")
                && !low.starts_with("reasoning")
                && !low.starts_with("session id")
                && *l != "--------"
                && *l != "user"
        })
        .take(3)
        .collect::<Vec<_>>()
        .join(" ")
}
