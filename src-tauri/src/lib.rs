mod commands;

use tauri::Emitter;

use commands::config::{self, ConfigInfo, WriteResult};
use commands::deeplink;
use commands::scheme;
use commands::sso;
use commands::detect::{self, EnvStatus};
use commands::launch;
use commands::prepare;
use commands::test::{self, TestResult};

#[tauri::command]
async fn check_environment() -> EnvStatus {
    detect::check_environment()
}

/// 下载并解压 Codex 到我们自己的目录（不安装、不写 PATH）。
#[tauri::command]
async fn prepare_codex(app: tauri::AppHandle) -> Result<String, String> {
    prepare::prepare_codex(app).await
}

#[tauri::command]
async fn read_config() -> ConfigInfo {
    config::read_config()
}

/// 写我们自己的（隔离的）config.toml；`also_desktop` 为真时才额外写用户的 `~/.codex`（带备份、外科式）。
#[tauri::command]
async fn write_config(key: String, also_desktop: bool) -> Result<WriteResult, String> {
    let r = config::write_isolated(&key)?;
    if also_desktop {
        let d = config::write_desktop(&key)?;
        return Ok(WriteResult {
            path: r.path,
            backup_path: d.backup_path,
            changed: r.changed || d.changed,
        });
    }
    Ok(r)
}

#[tauri::command]
async fn run_test(key: Option<String>) -> TestResult {
    test::run_test(key).await
}

/// 内嵌 SSO：浏览器登录 → 回环回调 → 换 Key（失败时前端退回手动粘贴）。
#[tauri::command]
async fn sso_obtain_key(app: tauri::AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        use tauri_plugin_opener::OpenerExt;
        let opener = |url: &str| -> Result<(), String> {
            app.opener()
                .open_url(url, None::<&str>)
                .map_err(|e| format!("打开系统浏览器失败：{e}"))
        };
        sso::obtain_key(&opener, std::time::Duration::from_secs(180))
    })
    .await
    .map_err(|e| format!("登录线程异常：{e}"))?
}

#[tauri::command]
async fn launch_codex() -> Result<String, String> {
    launch::launch_codex()
}

#[tauri::command]
async fn launch_desktop() -> Result<String, String> {
    launch::launch_desktop()
}

#[tauri::command]
async fn cleanup(app: tauri::AppHandle) -> Result<String, String> {
    // 绿色版：卸载时把 scheme 注册也撤掉，不在系统里留东西。
    let _ = &app;
    if let Err(e) = scheme::unregister() {
        println!("[deeplink] 注销 scheme 失败：{e}");
    }
    launch::cleanup()
}

#[tauri::command]
async fn open_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

/// 冷启动时 deep link 在 webview 还没加载完就来了，事件会丢；
/// 所以同时留一份「待取回」的 Key，前端挂载后调 `take_pending_key` 取走。
#[derive(Default)]
struct PendingKey(std::sync::Mutex<Option<deeplink::DeepLinkKey>>);

#[tauri::command]
fn take_pending_key(state: tauri::State<'_, PendingKey>) -> Option<deeplink::DeepLinkKey> {
    state.0.lock().ok().and_then(|mut g| g.take())
}

/// 把 deep link 里的 Key 交给界面（只发事件，不落盘、不写日志原文）。
fn handle_deep_link(app: &tauri::AppHandle, raw: &str) {
    match deeplink::parse_set_key(raw) {
        Ok(found) => {
            // 日志里只允许出现掩码
            println!(
                "[deeplink] 收到 set-key：{}（group={}）",
                deeplink::mask(&found.key),
                found.group.as_deref().unwrap_or("-")
            );
            {
                use tauri::Manager;
                if let Some(state) = app.try_state::<PendingKey>() {
                    if let Ok(mut g) = state.0.lock() {
                        *g = Some(found.clone());
                    }
                }
            }
            let _ = app.emit(
                "deep-link-key",
                serde_json::json!({ "key": found.key, "group": found.group }),
            );
        }
        Err(msg) => {
            println!("[deeplink] 忽略：{msg}（原文不入日志）");
            let _ = app.emit("deep-link-error", serde_json::json!({ "message": msg }));
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // single-instance 必须是第一个插件（官方要求），否则「程序已在跑时又点官网按钮」收不到转发。
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // 程序已经在跑时又点了一次官网按钮：把新链接里的 Key 灌进已有窗口。
            for arg in argv.iter().filter(|a| a.starts_with("aiquick://")) {
                handle_deep_link(app, arg);
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .manage(PendingKey::default())
        .setup(|app| {
            use tauri_plugin_deep_link::DeepLinkExt;
            // 绿色版：自己注册 scheme（Windows 写 HKCU、Linux 写用户级 .desktop），不依赖插件的 GUI 注册。
            let handle = app.handle().clone();
            if !scheme::is_registered() {
                match scheme::register() {
                    Ok(msg) => println!("[deeplink] {msg}"),
                    Err(e) => println!("[deeplink] 注册 scheme 失败（不影响手动粘贴）：{e}"),
                }
            }
            // 冷启动：程序是被 deep link 拉起来的，链接在 current 里。
            if let Ok(Some(urls)) = handle.deep_link().get_current() {
                for u in urls {
                    handle_deep_link(&handle, u.as_str());
                }
            }
            // 运行中：再点官网按钮触发。
            let handle2 = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for u in event.urls() {
                    handle_deep_link(&handle2, u.as_str());
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            take_pending_key,
            sso_obtain_key,
            check_environment,
            prepare_codex,
            read_config,
            write_config,
            run_test,
            launch_codex,
            launch_desktop,
            cleanup,
            open_url
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// 无界面模式：与界面共用同一套函数，供自动化验收（假 HOME、真 Key、错 Key）使用。
/// 用法：`aiquick --headless <check|prepare|config|test|launch|cleanup> [--key sk-...] [--also-desktop]`
pub fn headless(args: &[String]) -> i32 {
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let key = args
        .iter()
        .position(|a| a == "--key")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let also_desktop = args.iter().any(|a| a == "--also-desktop");
    let out = match cmd {
        "check" => Ok(serde_json::to_string_pretty(&detect::check_environment()).unwrap()),
        "prepare" => prepare::prepare_sync().map(|v| {
            serde_json::json!({ "version": v }).to_string()
        }),
        "config" => match key {
            Some(k) => {
                let r = config::write_isolated(&k).and_then(|r| {
                    if also_desktop {
                        config::write_desktop(&k).map(|d| WriteResult {
                            path: r.path.clone(),
                            backup_path: d.backup_path,
                            changed: r.changed || d.changed,
                        })
                    } else {
                        Ok(r)
                    }
                });
                r.and_then(|r| serde_json::to_string_pretty(&r).map_err(|e| e.to_string()))
            }
            None => serde_json::to_string_pretty(&config::read_config()).map_err(|e| e.to_string()),
        },
        "test" => Ok(serde_json::to_string_pretty(&test::run_test_blocking(key)).unwrap()),
        // 给主页/排障用的一句话状态：Codex 就绪了吗、Key 配好了吗
        "status" => {
            let env = detect::check_environment();
            let cfg = config::read_config();
            let tool_status = commands::tools::primary().status();
            let codex_ready = env.codex.prepared;
            let key_configured = cfg.has_key;
            Ok(serde_json::json!({
                "codex_ready": codex_ready,
                "key_configured": key_configured,
                "ready": codex_ready && key_configured,
                "codex_version": env.codex.version,
                "model": cfg.model,
                "config_path": cfg.path,
                "data_root": env.data_root,
                "tool": tool_status,
            })
            .to_string())
        }
        "deeplink" => {
            let raw = args.get(1).cloned().unwrap_or_default();
            match deeplink::parse_set_key(&raw) {
                Ok(found) => Ok(serde_json::json!({
                    "ok": true,
                    "key_masked": deeplink::mask(&found.key),
                    "group": found.group,
                })
                .to_string()),
                Err(msg) => Ok(serde_json::json!({ "ok": false, "message": msg }).to_string()),
            }
        }
        "sso" => sso::debug_info().and_then(|v| serde_json::to_string_pretty(&v).map_err(|e| e.to_string())),
        "register-scheme" => scheme::register(),
        "unregister-scheme" => scheme::unregister(),
        "launch" => launch::launch_codex(),
        "launch-desktop" => launch::launch_desktop(),
        "cleanup" => launch::cleanup(),
        other => Err(format!("未知子命令：{other}")),
    };
    match out {
        Ok(s) => {
            println!("{s}");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}
