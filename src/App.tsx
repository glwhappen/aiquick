import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type CodexStatus = {
  prepared: boolean;
  version: string | null;
  path: string | null;
  dir: string | null;
};

type SystemCodex = {
  cli_found: boolean;
  cli_path: string | null;
  cli_version: string | null;
  desktop_supported: boolean;
  desktop_found: boolean;
  desktop_detail: string;
  desktop_package: string | null;
  desktop_version: string | null;
};

type EnvStatus = {
  codex: CodexStatus;
  system: SystemCodex;
  os: string;
  data_root: string;
  codex_home: string;
  config_path: string;
  config_exists: boolean;
  user_codex_home: string;
};

type ConfigInfo = {
  path: string;
  exists: boolean;
  model: string | null;
  provider: string | null;
  base_url: string | null;
  has_key: boolean;
  key_masked: string | null;
  desktop_configured: boolean;
};

type TestResult = {
  ok: boolean;
  category: string;
  message: string;
  model: string;
  elapsed_ms: number;
  command: string;
  output: string;
  reply: string | null;
};

type StepState = "wait" | "active" | "done" | "fail";

const STORE_URL = "https://apps.microsoft.com/search?query=Codex";
const OFFICIAL_URL = "https://chatgpt.com/codex";

export default function App() {
  const [env, setEnv] = useState<EnvStatus | null>(null);
  const [config, setConfig] = useState<ConfigInfo | null>(null);
  const [key, setKey] = useState("");
  const [alsoDesktop, setAlsoDesktop] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [log, setLog] = useState<string[]>([]);
  const [prepareError, setPrepareError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saveInfo, setSaveInfo] = useState<string | null>(null);
  const [test, setTest] = useState<TestResult | null>(null);
  const [launchInfo, setLaunchInfo] = useState<string | null>(null);
  const [cleanInfo, setCleanInfo] = useState<string | null>(null);
  const [showDesktop, setShowDesktop] = useState(false);
  const [deepLinkInfo, setDeepLinkInfo] = useState<string | null>(null);
  const [loggingIn, setLoggingIn] = useState(false);
  const [ssoInfo, setSsoInfo] = useState<string | null>(null);
  const [ssoError, setSsoError] = useState<string | null>(null);
  /// 用户选择「跳过 CLI，用我已有的桌面版」
  const [skipCli, setSkipCli] = useState(false);
  const logRef = useRef<HTMLDivElement | null>(null);

  const refresh = useCallback(async () => {
    const [e, c] = await Promise.all([
      invoke<EnvStatus>("check_environment"),
      invoke<ConfigInfo>("read_config"),
    ]);
    setEnv(e);
    setConfig(c);
  }, []);

  useEffect(() => {
    void refresh();
    // 官网点「填入 aiquick」→ aiquick://set-key?key=...&group=...
    const unKey = listen<{ key: string; group: string | null }>("deep-link-key", (ev) => {
      const { key, group } = ev.payload;
      setKey(key);
      setSaveInfo(null);
      setSaveError(null);
      setTest(null);
      setDeepLinkInfo(
        `已从网页填入 Key（${key.slice(0, 8)}…${key.slice(-4)}${group ? ` · ${group}` : ""}），点「测一下」验证`,
      );
    });
    const unErr = listen<{ message: string }>("deep-link-error", (ev) => {
      setDeepLinkInfo(`网页传来的链接没派上用场：${ev.payload.message}`);
    });
    // 冷启动（程序是被 deep link 拉起来的）：事件在 webview 加载前就发过了，这里补取一次。
    void invoke<{ key: string; group: string | null } | null>("take_pending_key").then((pending) => {
      if (!pending) return;
      setKey(pending.key);
      setDeepLinkInfo(
        `已从网页填入 Key（${pending.key.slice(0, 8)}…${pending.key.slice(-4)}${
          pending.group ? ` · ${pending.group}` : ""
        }），点「测一下」验证`,
      );
    });
    const un = listen<{ level: string; line: string }>("prepare-log", (ev) => {
      if (ev.payload.level === "tick") {
        setLog((prev) => [...prev.filter((l) => !l.startsWith("已等待")), ev.payload.line]);
        return;
      }
      setLog((prev) => [...prev, ev.payload.line].slice(-5));
    });
    return () => {
      void un.then((f) => f());
      void unKey.then((f) => f());
      void unErr.then((f) => f());
    };
  }, [refresh]);

  useEffect(() => {
    if (logRef.current) logRef.current.scrollTop = logRef.current.scrollHeight;
  }, [log]);

  const open = (url: string) => {
    void invoke("open_url", { url });
  };

  const doPrepare = async () => {
    setPreparing(true);
    setPrepareError(null);
    setLog(["正在连接国内镜像…"]);
    try {
      const v = await invoke<string>("prepare_codex");
      setLog((prev) => [...prev, `✅ 准备好了：${v}`]);
      await refresh();
    } catch (e) {
      setPrepareError(String(e));
    } finally {
      setPreparing(false);
    }
  };

  const doSave = async () => {
    setSaving(true);
    setSaveError(null);
    setSaveInfo(null);
    try {
      // 选了「跳过，用我已有的桌面版」就自动写桌面版配置（用户不用再勾选任何东西）
      const wantDesktop = alsoDesktop || skipCli;
      const r = await invoke<{ path: string; backup_path: string | null; changed: boolean }>(
        "write_config",
        { key, alsoDesktop: wantDesktop },
      );
      setSaveInfo(
        wantDesktop
          ? `已写入 ${r.path}，并把同一把 Key 写进桌面版配置${r.backup_path ? `（原文件已备份）` : ""}`
          : `已写入 ${r.path}`,
      );
      await refresh();
      setTest(null);
    } catch (e) {
      setSaveError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const runTest = async (): Promise<TestResult | null> => {
    setTesting(true);
    setTest(null);
    try {
      const r = await invoke<TestResult>("run_test", { key: key.trim() ? key.trim() : null });
      setTest(r);
      await refresh();
      return r;
    } catch (e) {
      const r: TestResult = {
        ok: false,
        category: "unknown",
        message: String(e),
        model: "",
        elapsed_ms: 0,
        command: "",
        output: "",
        reply: null,
      };
      setTest(r);
      return r;
    } finally {
      setTesting(false);
    }
  };

  const doTest = async () => {
    await runTest();
  };

  // 「启动 Codex」是主路径的最后一步：没测过就当场真测一次，通了才启动。
  // （重开应用后不会记得上次测过，所以按钮不能是灰的。）
  /// 内嵌 SSO：浏览器登录 → 回环回调 → 服务端发 Key → 自动填入
  const doLogin = async () => {
    setLoggingIn(true);
    setSsoError(null);
    setSsoInfo(null);
    try {
      const key = await invoke<string>("sso_obtain_key");
      setKey(key);
      setDeepLinkInfo(null);
      setSsoInfo(`已登录并自动填入 Key（${key.slice(0, 8)}…${key.slice(-4)}），点「测一下」验证`);
    } catch (e) {
      setSsoError(String(e));
    } finally {
      setLoggingIn(false);
    }
  };

  const doLaunch = async () => {
    setLaunchInfo(null);
    if (!passed) {
      const r = await runTest();
      if (!r?.ok) return;
    }
    try {
      const msg = await invoke<string>(skipCli ? "launch_desktop" : "launch_codex");
      setLaunchInfo(msg);
    } catch (e) {
      setLaunchInfo(`启动失败：${e}`);
    }
  };

  const doCleanup = async () => {
    try {
      const msg = await invoke<string>("cleanup");
      setCleanInfo(msg);
      await refresh();
    } catch (e) {
      setCleanInfo(`清理失败：${e}`);
    }
  };

  const cliPrepared = !!env?.codex.prepared;
  const desktopFound = !!env?.system.desktop_found;
  const ready = cliPrepared || skipCli;
  const saved = !!config?.has_key;
  const passed = !!test?.ok;

  const steps = useMemo(() => {
    const list: { n: number; title: string; state: StepState; detail: string }[] = [];
    list.push({
      n: 1,
      title: "检查环境",
      state: env ? "done" : "active",
      detail: env
        ? env.system.desktop_supported
          ? env.system.desktop_found
            ? `${env.system.desktop_detail}${env.system.desktop_package ? ` · ${env.system.desktop_package}` : ""}`
            : `${env.system.desktop_detail}（没有也不影响）`
          : "当前系统没有 Codex 桌面版"
        : "正在检查…",
    });
    list.push({
      n: 2,
      title: "准备 Codex",
      state: ready ? "done" : preparing ? "active" : env ? "active" : "wait",
      detail: skipCli
        ? "已跳过：直接用你机器上装好的桌面版"
        : cliPrepared
          ? `已备好 Codex CLI ${env?.codex.version ?? ""}`.trim()
          : preparing
            ? "正在下载解压…"
            : desktopFound
              ? "可以直接跳过，用你已有的桌面版"
              : "点按钮从国内镜像直接下载（约 100MB），不安装、不写 PATH",
    });
    list.push({
      n: 3,
      title: "填入 Key",
      state: saved ? "done" : ready ? "active" : "wait",
      detail: saved
        ? `已保存（${config?.key_masked ?? ""}），模型 ${config?.model ?? ""}`
        : "把 sk- 开头的 Key 粘贴进来",
    });
    list.push({
      n: 4,
      title: "测一下能不能用",
      state: passed ? "done" : saved ? "active" : "wait",
      detail: passed
        ? "实测通过，可以开始用了"
        : saved
          ? "点「测一下」，会真的跑一次 Codex"
          : "保存 Key 之后就能测",
    });
    return list;
  }, [env, ready, preparing, saved, config, passed]);

  return (
    <div className="app">
      <header className="head">
        <h1>aiquick</h1>
        <p className="sub">绿色版，不安装、不改你的环境。配好就能用，用完删掉即可。</p>
      </header>

      <div className="steps">
        {steps.map((s) => (
          <div key={s.n} className={`step step-${s.state}`}>
            <div className="badge">{s.state === "done" ? "✓" : s.n}</div>
            <div className="step-body">
              <div className="step-title">{s.title}</div>
              <div className="step-detail">{s.detail}</div>
            </div>
          </div>
        ))}
      </div>

      <main className="panel">
        {!env && <div className="hint">正在检查环境…</div>}

        {env && !ready && (
          <>
            {desktopFound ? (
              <>
                <button className="btn primary big" onClick={() => setSkipCli(true)}>
                  跳过，用我已有的桌面版
                </button>
                <button className="btn secondary" disabled={preparing} onClick={doPrepare}>
                  {preparing
                    ? "正在下载，请稍候…"
                    : "也下载一份 Codex CLI（约 100MB，想在终端里用才需要）"}
                </button>
              </>
            ) : (
              <button className="btn primary big" disabled={preparing} onClick={doPrepare}>
                {preparing ? "正在下载，请稍候…" : "准备 Codex"}
              </button>
            )}
            <p className="hint small">
              从国内镜像下载（约 100MB），解压到本工具自己的目录，不写 PATH、不进系统目录。
            </p>
            {skipCli && (
              <button className="btn ghost small" onClick={() => setSkipCli(false)}>
                还是下载一份 CLI
              </button>
            )}
            {log.length > 0 && (
              <div className="log" ref={logRef}>
                {log.map((l, i) => (
                  <div key={i}>{l}</div>
                ))}
              </div>
            )}
            {prepareError && <ErrBox text={prepareError} />}
          </>
        )}

        {env && ready && (
          <>
            <label className="label" htmlFor="key">
              API Key
            </label>
            <input
              id="key"
              className="input"
              placeholder="sk-..."
              value={key}
              spellCheck={false}
              onChange={(e) => {
                setKey(e.target.value);
                setSaveInfo(null);
                setSaveError(null);
                setDeepLinkInfo(null);
              }}
            />
            <div className="row">
              <button
                className="btn secondary"
                disabled={loggingIn}
                onClick={doLogin}
              >
                {loggingIn ? "正在打开登录页…" : "登录并自动领取 Key"}
              </button>
              <p className="hint small">
                点上面的按钮会在浏览器里登录，回来 Key 就自动填好了（不复制粘贴）。
                也可以直接把已有的 sk- Key 粘贴到上面输入框。
              </p>
            </div>
            {ssoInfo && <p className="ok small">{ssoInfo}</p>}
            {ssoError && <ErrBox text={ssoError} />}
            {deepLinkInfo && <p className="ok small">{deepLinkInfo}</p>}

            {env.system.desktop_supported && (
              <div className={`sys ${env.system.desktop_found ? "sys-ok" : "sys-miss"}`}>
                {env.system.desktop_found ? (
                  <>
                    ✅ {env.system.desktop_detail}
                    <span className="muted"> · {env.system.desktop_package}</span>
                  </>
                ) : (
                  <>
                    ⚪ {env.system.desktop_detail}
                    <span className="muted">（没有也不影响，本工具自带一份 Codex）</span>
                  </>
                )}
              </div>
            )}

            <button
              className="btn primary"
              disabled={saving || key.trim().length === 0}
              onClick={doSave}
            >
              {saving ? "正在保存…" : "保存配置"}
            </button>
            {saveInfo && <p className="ok small">{saveInfo}</p>}
            {saveError && <ErrBox text={saveError} />}

            {skipCli ? (
              <p className="hint small">
                保存时会自动把这把 Key 写进桌面版配置（先备份，只改 model / model_provider /
                [model_providers.hsfp] 几行，其它不动）。
              </p>
            ) : (
              <label className="check small">
                <input
                  type="checkbox"
                  checked={alsoDesktop}
                  onChange={(e) => setAlsoDesktop(e.target.checked)}
                />
                桌面版也要用这个 Key（会备份并只改必要几行）
              </label>
            )}

            <button className="btn secondary" disabled={testing || (!saved && !key.trim())} onClick={doTest}>
              {testing ? "正在实测…" : "测一下"}
            </button>

            {test && (
              <div className={`result ${test.ok ? "result-ok" : "result-fail"}`}>
                <div className="result-title">
                  {test.ok ? "✅ 可以用" : "❌ 还不能用"}
                  {test.model ? <span className="muted"> · {test.model}</span> : null}
                  {test.elapsed_ms > 0 ? (
                    <span className="muted"> · {(test.elapsed_ms / 1000).toFixed(1)}s</span>
                  ) : null}
                </div>
                {test.reply && <div className="reply">回复：{test.reply}</div>}
                <div className="result-msg">{test.message}</div>
                <details>
                  <summary className="small muted">看详细输出</summary>
                  <pre className="pre">
                    {test.command ? `$ ${test.command}\n` : ""}
                    {test.output || "(无输出)"}
                  </pre>
                </details>
              </div>
            )}

            <button className="btn primary big" disabled={testing} onClick={doLaunch}>
              {testing
                ? "正在实测…"
                : passed
                  ? skipCli
                    ? "打开 Codex 桌面版"
                    : "启动 Codex"
                  : "测一下并启动"}
            </button>
            {!passed && !testing && (
              <p className="hint small">
                {skipCli
                  ? "会先用你的 Key 真跑一次（直连验证），确认能用再帮你打开桌面版。"
                  : "会先真跑一次确认能用，通了才打开终端（免得进去一脸懵）。"}
              </p>
            )}
            {!skipCli && (
              <p className="hint small">
                第一次进 Codex 它会问「是否信任当前文件夹」，按回车同意即可；之后直接聊天就能用。
              </p>
            )}
            {launchInfo && <p className="ok small">{launchInfo}</p>}
          </>
        )}
      </main>

      <footer className="foot">
        <button className="link" onClick={() => setShowDesktop((v) => !v)}>
          {showDesktop ? "收起桌面版说明" : "想用桌面版？"}
        </button>
        {showDesktop && (
          <div className="desktop">
            <p className="small">
              Codex 桌面版（含新版 ChatGPT 桌面应用）约 1GB，本工具不代下（太慢），请用下面任一方式自行安装。
              装好后本工具只需一步就能让它用上你的 Key。
            </p>
            <ol className="small">
              <li>
                Windows：Microsoft Store 搜 <code>Codex</code>（
                <a onClick={() => open(STORE_URL)}>打开商店搜索</a>）
              </li>
              <li>
                官方下载页：<a onClick={() => open(OFFICIAL_URL)}>{OFFICIAL_URL}</a>
              </li>
            </ol>
            <p className="small">
              这说的是 <b>Codex 桌面版（含新版 ChatGPT 桌面应用）</b>：装好后勾选上面的「桌面版也要用这个 Key」，
              再打开应用，左上角切到 <b>Codex 模式</b>（普通对话不走这里的 Key）。
            </p>
            <p className="small muted">
              {env?.system.desktop_supported
                ? env.system.desktop_detail
                : "当前系统没有 Codex 桌面版"}
              {config?.desktop_configured ? " · 桌面版配置已写入" : ""}
            </p>
          </div>
        )}

        <details className="tiny">
          <summary className="link">高级 / 清理</summary>
          <div className="desktop">
            <p className="small muted">
              本工具的数据目录：<code>{env?.data_root ?? ""}</code>
              <br />
              Codex 的隔离配置目录：<code>{env?.codex_home ?? ""}</code>
              <br />
              你自己的 <code>{env?.user_codex_home ?? "~/.codex"}</code> 不会被改动
              {config?.desktop_configured ? "（除了你勾选过桌面版那一次）" : ""}。
            </p>
            <button className="btn ghost small" onClick={doCleanup}>
              清理并退出（删掉本工具产生的全部文件）
            </button>
            {cleanInfo && <p className="small ok">{cleanInfo}</p>}
          </div>
        </details>

        <p className="tiny muted">命令行里也能用：把 CODEX_HOME 指向上面的隔离目录即可。</p>
      </footer>
    </div>
  );
}

function ErrBox({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="err">
      <div className="err-text">{text}</div>
      <button
        className="link small"
        onClick={() => {
          void navigator.clipboard.writeText(text).then(() => {
            setCopied(true);
            setTimeout(() => setCopied(false), 1500);
          });
        }}
      >
        {copied ? "已复制" : "复制错误"}
      </button>
    </div>
  );
}
