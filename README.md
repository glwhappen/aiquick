# aiquick

工具无关的极简安装/配置器（当前只实现 Codex）。**绿色版：不安装、不写 PATH、不改你已有的环境。**

需求唯一来源：[`SPEC.md`](./SPEC.md)（§1.5「免安装原则」优先级最高）。

## 它做什么

> 名字里不带任何工具：目前只实现 **Codex**，加别的工具是「加一个 tool 定义」
> （`src-tauri/src/commands/tools/`），不改主流程。

打开后只有一条路，一屏四步：

1. **检查环境** —— 看本工具自己的目录里有没有备好的 Codex。
2. **准备 Codex** —— 从 npmmirror 镜像下载官方原生二进制，解压到
   `<数据目录>/aiquick/tools/codex/packages/<版本>/`，直接以该路径运行。
   **不装进系统目录、不建软链、不写 PATH、不碰 `~/.local/bin`。**
3. **填入 Key** —— 写进 `<数据目录>/aiquick/tools/codex/home/config.toml`。
   这个目录就是启动 Codex 时的 `CODEX_HOME`，**用户自己的 `~/.codex` 一个字节都不动**。
4. **测一下** —— 用准备好的 Codex 真的跑一次
   `codex exec --skip-git-repo-check "reply with only the word OK"`，
   成功显示 ✅ + 模型 + 耗时 + 真实回复；失败显示 ❌ + 具体原因（鉴权失败 / 模型无渠道 / 网络不通 / 还没准备）。

测通之后给「启动 Codex」按钮：在新终端窗口里带上 `CODEX_HOME` 启动，用户不用知道 PATH。
底部「高级 / 清理」里有「清理并退出」——删掉本工具产生的全部文件，等于卸载。

## 领 Key：内嵌 SSO（不再外跳）

点「登录并自动领取 Key」→ 系统浏览器打开 Authentik 登录页 → 回环回调 `http://127.0.0.1:17321/callback`
→ 程序拿 id_token 向 provisioning 端点换一把 Key → **直接写进隔离配置，用户不复制粘贴**。

- OIDC：Public client + PKCE(S256)，issuer `https://auth.hsfp.cn/application/o/aiquick/`，client_id `aiquick-desktop`
  （provider pk=63 / application slug=aiquick，`sub_mode=hashed_user_id`）。**没有 client_secret，仓库里没有任何凭据。**
- 服务端环节：桌面程序连不到 new-api 的 Postgres，所以「查/建用户 + 建 token」由 provisioning 端点做
  （默认 `POST https://aiquick.hsfp.cn/api/provision-key`，入参 `{id_token, group, name}`，出参 `{key:"sk-…"}`）。
  逻辑沿用 `/root/hsfp-code-site/app.py` 的 `ensure_user()` / `ensure_token()`（`group=半价`、`unlimited_quota=true`、
  `expired_time=-1`、`model_limits_enabled=false`、幂等复用同名 token）。
- 可用环境变量覆盖，便于自测/自托管：
  `AIQUICK_OIDC_ISSUER` / `AIQUICK_OIDC_CLIENT_ID` / `AIQUICK_REDIRECT_URI` / `AIQUICK_PROVISION_URL` / `AIQUICK_TOKEN_GROUP`。
- 自检：`aiquick --headless sso`（会真的拉一次 discovery，打印端点与 PKCE 参数）。
- 兜底：SSO 失败时仍可手动粘贴一把 Key；**任何提示里都不再出现别的产品域名**。

## deep link（可选旁路）：从网页回填 Key

程序注册了 URL scheme **`aiquick`**。网页端的 Key 卡片上点「填入 aiquick」会跳到：

```
aiquick://set-key?key=sk-xxxx&group=半价
```

本程序收到后**自动填入输入框**并提示「已从网页填入 Key，点『测一下』验证」，用户不用复制粘贴。

- 参数名固定：scheme=`aiquick`、host/path=`set-key`、参数 `key` / `group`（官网侧按这个对齐）。
- 幂等：重复点只覆盖同一个 Key；程序已在运行时会复用同一个窗口（`single-instance`）。
- 安全：**日志里只有「尾号 xxxx（长度 n）」，`rg 'sk-' 日志` 是空的**；非法/缺参数的链接忽略并给中性提示，不崩。
- 手动粘贴的兜底路径一直都在。

### 注册与撤销

scheme 由程序自己注册（不依赖安装器，绿色版也能用）：

- **Windows**：`HKCU\Software\Classes\aiquick`（当前用户，不需要管理员）
- **Linux**：`~/.local/share/applications/aiquick-handler.desktop` + `xdg-mime default`
- **macOS**：由 `.app` 的 `CFBundleURLTypes` 提供（打包时写入）

「清理并退出」会把它一起撤掉；也可以手动：
`aiquick --headless register-scheme` / `--headless unregister-scheme`。

命令行自检：`aiquick --headless deeplink 'aiquick://set-key?key=sk-xxx&group=半价'`

## 写进配置的东西

`<数据目录>/aiquick/tools/codex/home/config.toml`：

```toml
model = "deepseek-flash"
model_provider = "hsfp"

[model_providers.hsfp]
name = "HSFP"
base_url = "https://new-api.hsfp.cn/v1"
wire_api = "responses"

[model_providers.hsfp.http_headers]
Authorization = "Bearer sk-..."
```

用 `http_headers` 内联 Key，**不用 `env_key`**：桌面版 App 不继承 shell 环境变量，用 `env_key` 会静默失效。

### 桌面版（可选，默认不做）

桌面版读的是固定的 `~/.codex/config.toml`，没法用 `CODEX_HOME` 隔离，这是唯一例外。
只有勾选「桌面版也要用这个 Key」时才会写它：先备份 `config.toml.bak.<时间戳>`，
再按 TOML 段做外科式修改，只增改 `model` / `model_provider` / `[model_providers.hsfp]`，
用户原有的 `[desktop]`、`[projects.*]`、`[sandbox_workspace_write]` 等段逐字符保留。

## 数据目录在哪

| 平台 | 路径 |
|---|---|
| Windows | `%LOCALAPPDATA%\aiquick` |
| macOS | `~/Library/Application Support/aiquick` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/aiquick` |

可用环境变量 `AIQUICK_HOME` 覆盖（自动化验收就用它 + 假 `HOME`）。

## 下载即用（不用解压）

Release 资产命名：Windows 是裸 `aiquick.exe`；Linux 是裸 `aiquick` 与 `aiquick-<版本>-linux-x86_64.AppImage`；
macOS 因为系统限制只能打包，文件名是 `aiquick-<版本>-macos-aarch64.app.zip`。

| 你的机器 | 下载 | 怎么用 |
|---|---|---|
| Windows 10/11 | `aiquick.exe` | **双击即用**（单文件、免安装；需要系统自带 WebView2） |
| Linux x86_64（较新发行版） | `aiquick` | `chmod +x aiquick && ./aiquick` |
| Linux x86_64（老发行版 / glibc 太低） | `aiquick-<版本>-linux-x86_64.AppImage` | `chmod +x` 后双击或直接运行 |
| Apple 芯片 Mac | `aiquick-<版本>-macos-aarch64.app.zip` | 解压得到 `aiquick.app`，拖到任意目录；未签名，首次右键 → 打开 |
| Intel Mac | 暂未出（可在 CI 加 `macos-13` job） | — |

没有 `.deb` / `.dmg` / 安装向导——我们只出绿色版：下载即可运行，删掉即卸载（数据都在自己的目录里）。

- **Windows** 依赖系统自带 WebView2（Win11 与较新的 Win10 已内置）。
- **macOS** 未签名，首次打开被 Gatekeeper 拦一下：右键 → 打开，或 `xattr -dr com.apple.quarantine aiquick.app`。
- **Linux** 需要 webkit2gtk-4.1（多数桌面发行版已有）。

## 领 Key：内嵌 SSO（不再外跳）

点「登录并自动领取 Key」→ 系统浏览器打开 Authentik 登录页 → 回环回调 `http://127.0.0.1:17321/callback`
→ 程序拿 id_token 向 provisioning 端点换一把 Key → **直接写进隔离配置，用户不复制粘贴**。

- OIDC：Public client + PKCE(S256)，issuer `https://auth.hsfp.cn/application/o/aiquick/`，client_id `aiquick-desktop`
  （provider pk=63 / application slug=aiquick，`sub_mode=hashed_user_id`）。**没有 client_secret，仓库里没有任何凭据。**
- 服务端环节：桌面程序连不到 new-api 的 Postgres，所以「查/建用户 + 建 token」由 provisioning 端点做
  （默认 `POST https://aiquick.hsfp.cn/api/provision-key`，入参 `{id_token, group, name}`，出参 `{key:"sk-…"}`）。
  逻辑沿用 `/root/hsfp-code-site/app.py` 的 `ensure_user()` / `ensure_token()`（`group=半价`、`unlimited_quota=true`、
  `expired_time=-1`、`model_limits_enabled=false`、幂等复用同名 token）。
- 可用环境变量覆盖，便于自测/自托管：
  `AIQUICK_OIDC_ISSUER` / `AIQUICK_OIDC_CLIENT_ID` / `AIQUICK_REDIRECT_URI` / `AIQUICK_PROVISION_URL` / `AIQUICK_TOKEN_GROUP`。
- 自检：`aiquick --headless sso`（会真的拉一次 discovery，打印端点与 PKCE 参数）。
- 兜底：SSO 失败时仍可手动粘贴一把 Key；**任何提示里都不再出现别的产品域名**。

## deep link（可选旁路）：从网页回填 Key

程序注册了 URL scheme **`aiquick`**。网页端的 Key 卡片上点「填入 aiquick」会跳到：

```
aiquick://set-key?key=sk-xxxx&group=半价
```

本程序收到后**自动填入输入框**并提示「已从网页填入 Key，点『测一下』验证」，用户不用复制粘贴。

- 参数名固定：scheme=`aiquick`、host/path=`set-key`、参数 `key` / `group`（官网侧按这个对齐）。
- 幂等：重复点只覆盖同一个 Key；程序已在运行时会复用同一个窗口（`single-instance`）。
- 安全：**日志里只有「尾号 xxxx（长度 n）」，`rg 'sk-' 日志` 是空的**；非法/缺参数的链接忽略并给中性提示，不崩。
- 手动粘贴的兜底路径一直都在。

### 注册与撤销

scheme 由程序自己注册（不依赖安装器，绿色版也能用）：

- **Windows**：`HKCU\Software\Classes\aiquick`（当前用户，不需要管理员）
- **Linux**：`~/.local/share/applications/aiquick-handler.desktop` + `xdg-mime default`
- **macOS**：由 `.app` 的 `CFBundleURLTypes` 提供（打包时写入）

「清理并退出」会把它一起撤掉；也可以手动：
`aiquick --headless register-scheme` / `--headless unregister-scheme`。

命令行自检：`aiquick --headless deeplink 'aiquick://set-key?key=sk-xxx&group=半价'`

## 写进配置的东西

`<数据目录>/aiquick/tools/codex/home/config.toml`：

```toml
model = "deepseek-flash"
model_provider = "hsfp"

[model_providers.hsfp]
name = "HSFP"
base_url = "https://new-api.hsfp.cn/v1"
wire_api = "responses"

[model_providers.hsfp.http_headers]
Authorization = "Bearer sk-..."
```

用 `http_headers` 内联 Key，**不用 `env_key`**：桌面版 App 不继承 shell 环境变量，用 `env_key` 会静默失效。

### 桌面版（可选，默认不做）

桌面版读的是固定的 `~/.codex/config.toml`，没法用 `CODEX_HOME` 隔离，这是唯一例外。
只有勾选「桌面版也要用这个 Key」时才会写它：先备份 `config.toml.bak.<时间戳>`，
再按 TOML 段做外科式修改，只增改 `model` / `model_provider` / `[model_providers.hsfp]`，
用户原有的 `[desktop]`、`[projects.*]`、`[sandbox_workspace_write]` 等段逐字符保留。

## 数据目录在哪

| 平台 | 路径 |
|---|---|
| Windows | `%LOCALAPPDATA%\aiquick` |
| macOS | `~/Library/Application Support/aiquick` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/aiquick` |

可用环境变量 `AIQUICK_HOME` 覆盖（自动化验收就用它 + 假 `HOME`）。

## 开发 / 构建

```bash
pnpm install
pnpm dev                       # 前端
pnpm tauri dev                 # 整个应用（开发）
pnpm tauri build               # 出安装包（本项目主推绿色版，见下）
./scripts/package-portable.sh  # 出绿色版 zip（不打包安装向导）
cargo test --manifest-path src-tauri/Cargo.toml --lib   # 配置外科式修改的单元测试
```

### 无界面模式（自动化验收用）

```
aiquick --headless check
aiquick --headless prepare
aiquick --headless config --key sk-... [--also-desktop]
aiquick --headless test --key sk-...
aiquick --headless launch
aiquick --headless cleanup
```

与界面调用的是同一套函数，输出 JSON。

## 平台说明

- **Windows**：绿色版就是那个 `aiquick.exe`，双击即跑。
  依赖系统的 **WebView2 Runtime**（Win10 2004+/Win11 一般已自带）。
- **macOS**：`.app` 拖走即用（不装 LaunchDaemon、不写 /usr/local）。
- **Linux**：单文件可执行，需要 webkit2gtk-4.1。

## 不做的事

不做供应商列表、不做路由/故障转移、不做 MCP/Skills/看板、不做多工具管理、不做第二个选择分支。

## 实测踩到的两个 Codex 行为（已在代码里规避）

1. **`model` 写进 `config.toml` 不等于 Codex 会用**：Codex 会把「上次在 TUI 里选的模型」持久化在它自己的状态里，优先级高于 `config.toml`，
   连 `codex exec` 都跟着走，于是拿着别的模型去打网关 → `503 No available channel`。所以「测一下」和「启动 Codex」都**显式带 `-m <模型>`**。
2. **第一次进 TUI 会问「是否信任当前文件夹」**：这是 Codex 自己的安全确认，按回车同意即可（我们没有替你自动设 trusted）。
