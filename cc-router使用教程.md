# cc-router 配置与使用教程（Windows）

> 基于源码 v6.5.0 整理。cc-router 是本地运行的大模型聚合网关：多个订阅的 API Key  
> 聚合为一个 Anthropic / OpenAI 兼容端点，供 Claude Code / Codex / Trae / Open WebUI 等工具接入。  
> 支持经典四槽范式与能力分类池（pool）范式，两套共存、可随时切换门禁。

---

## 一、它是什么（30 秒理解）

```
你的工具 (Claude Code / Codex / 浏览器...)
        │  入口：127.0.0.1:23456  (Anthropic/OpenAI 兼容协议)
        ▼
    cc-router  ← 你在这配置「虚拟模型 → 多家订阅」的调度
        │  出口：DeepSeek / GLM / Kimi / Anthropic / OpenAI / Gemini ...
        ▼
    各家大模型厂商
```

- 入口：`/v1/messages`（Anthropic）、`/v1/responses`（OpenAI）、`/v1/chat/completions`（OpenAI）
- 出口：24 家厂商预设，或任意兼容端点
- 调度：顺序 / 轮询 / 会话亲和，自动故障转移

---

## 二、安装与启动

### 方式 A：直接下载安装包（推荐，无需开发环境）

从官方 Release 下载对应平台安装包并安装：

| 系统      | 架构        | 安装包                                 |
| ------- | --------- | ----------------------------------- |
| Windows | x64       | `cc-router_windows-x64-setup.exe`   |
| Windows | x64 (MSI) | `cc-router_windows-x64.msi`         |
| Windows | arm64     | `cc-router_windows-arm64-setup.exe` |

下载地址（两地址内容相同，大陆用「中国下载」更快）：

- 全球：`https://github.com/finch-xu/cc-router/releases/latest`
- 中国：`https://d.cc-router.catonthe.top/latest/`

安装后直接启动 cc-router 桌面 app（首次启动进入 onboarding 引导）。

### 方式 B：从源码开发运行（需要 Node + Rust）

```bash
# 环境要求：Node.js ≥ 20、pnpm、Rust ≥ 1.90 (MSVC toolchain on Windows)
cd E:\cc-router-main
pnpm install
pnpm tauri dev      # 开发模式：前端 + Rust 后端 + 代理同时跑
```

> Windows 注意：必须 MSVC toolchain（`rustup show` 应带 `-msvc` 后缀），  
> 且需要 Visual Studio Build Tools（勾选「使用 C++ 的桌面开发」）。  
> dev 端口固定 1420，残留进程会导致 `Port 1420 is already in use`。

---

## 三、配置数据存在哪（重要）

| 内容       | 位置                                              |
| -------- | ----------------------------------------------- |
| 全局设置     | `%APPDATA%\com.cc-router.desktop\settings.json` |
| 订阅/日志数据库 | `%APPDATA%\com.cc-router.desktop\` 下的 SQLite 文件 |
| 证书/运行时   | 同目录下 tls 证书、runtime 文件                          |

> 提示：`%APPDATA%` 一般指 `C:\Users\<你的用户名>\AppData\Roaming`。  
> 备份 = 把这个 `com.cc-router.desktop` 文件夹整个复制走即可。

### settings.json 关键字段（默认值）

| 字段                   | 默认         | 说明                                  |
| -------------------- | ---------- | ----------------------------------- |
| `proxy_port`         | `23456`    | HTTP 代理端口                           |
| `proxy_mode`         | `"http"`   | `http` / `https` / `both`           |
| `https_port`         | `23457`    | HTTPS 端口（mode 含 https 时用）           |
| `listen_all`         | `false`    | `false`=仅本机 127.0.0.1；`true`=局域网可访问 |
| `auth_enabled`       | `true`     | 是否校验 token                          |
| `auth_token`         | 自动生成       | 32 位 hex，客户端用它鉴权                    |
| `cors_enabled`       | `true`     | 响应附加 CORS 头                         |
| `preferred_language` | `"system"` | `system` / `zh` / `zh-Hant` / `en` / `ja` |
| `routing_mode`       | `"slots"`  | `slots`=经典四槽；`pool`=能力分类池（详见第十三节）     |
| `log_retention_days` | `30`       | 请求日志保留天数（0=永久）；到期自动清理            |
| `web_ui_enabled`     | `false`    | 是否在代理端口提供网页管理界面 `/ui`               |
| `autostart`          | `false`    | 开机自启                                |
| `debug_mode`         | `false`    | 调试模式（出站请求体落盘）                       |

> 改动 settings.json 需要重启 app（或点 app 内「重启代理服务」）才生效。  
> 建议：正常使用一律在 app 界面里改，不要手改 json（除非做迁移/备份）。

---

## 四、首次使用（onboarding，照做即可）

1. **启动 app** → 进入引导页
2. **添加订阅**：
   - 选厂商（DeepSeek / GLM / Kimi / MiniMax / Anthropic / OpenAI ...）
   - 选接入点（Anthropic / OpenAI / Gemini 协议，按你的 Key 类型）
   - 填 API Key
   - 自动抓取模型列表（可手动改）
3. **绑定虚拟模型**：把订阅里的真实模型拖进 `model-fable / model-opus / model-sonnet / model-haiku` 四个槽位，每个槽位可绑多家 + 选调度模式
4. **复制接入配置**：app 内生成 Claude Code 的环境变量配置，粘到你的 `~/.claude/settings.json`

---

## 五、接入 Claude Code（最常用）

在 `~/.claude/settings.json` 里加入：

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://127.0.0.1:23456",
    "ANTHROPIC_AUTH_TOKEN": "你的token（app设置页里可见）",
    "API_TIMEOUT_MS": "3000000",
    "ANTHROPIC_MODEL": "model-fable",
    "ANTHROPIC_DEFAULT_FABLE_MODEL": "model-fable",
    "ANTHROPIC_DEFAULT_OPUS_MODEL": "model-opus",
    "ANTHROPIC_DEFAULT_SONNET_MODEL": "model-sonnet",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL": "model-haiku",
    "CLAUDE_CODE_SUBAGENT_MODEL": "model-opus",
    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
    "CLAUDE_CODE_DISABLE_NONSTREAMING_FALLBACK": "1",
    "CLAUDE_CODE_ATTRIBUTION_HEADER": "0",
    "CLAUDE_CODE_EFFORT_LEVEL": "max"
  }
}
```

然后正常使用 `claude` 即可。请求会走 cc-router 聚合的多家订阅。

> `model-opus` 若支持 1m 上下文可写 `model-opus[1m]`。  
> 也兼容 `anthropic/model-opus` 这种带前缀写法。

### 虚拟模型与别名

| 虚拟模型           | 别名（都归一）                                                 |
| -------------- | ------------------------------------------------------- |
| `model-fable`  | `anthropic/model-fable`、`claude-fable*`、`gpt-*-astra`   |
| `model-opus`   | `anthropic/model-opus`、`claude-opus*`、`gpt-*-sol`       |
| `model-sonnet` | `anthropic/model-sonnet`、`claude-sonnet*`、`gpt-*-terra` |
| `model-haiku`  | `anthropic/model-haiku`、`claude-haiku*`、`gpt-*-luna`    |
| `model-jev`    | 仅 `/v1/systemone` 入口可用                                  |

---

## 六、接入 OpenAI Codex CLI

`~/.codex/config.toml`：

```toml
[model_providers.cc-router]
name = "cc-router"
base_url = "http://127.0.0.1:23456/v1"
wire_api = "responses"
env_key = "OPENAI_API_KEY"

[profiles.cc-router]
model_provider = "cc-router"
model = "model-sonnet"
```

然后 `codex -p cc-router` 启动。

---

## 七、接入 Open WebUI / Cherry Studio / Cline（OpenAI Chat Completions）

- Base URL：`http://127.0.0.1:23456/v1`
- API Key：cc-router 设置页的 token
- 模型名：`model-fable` 等四槽名，或 `model-llm` / `model-multimodal` 等**能力池名**（见第十三节）；`gpt-6-sol` 等档位别名也支持
- 模型列表可从 `GET /v1/models` 拉取（该端点免鉴权）

---

## 八、网页界面（可选）

设置 → 网页界面 → 启用后，浏览器打开 `http://127.0.0.1:23456/ui/`，  
得到与桌面完全相同的管理界面。局域网访问需把监听切到 `0.0.0.0`。

---

## 九、终端界面 TUI（可选）

设置 → 安全与访问 → 终端界面 → 启用后：

- 复制启动路径；点「添加到 PATH」后，任何终端输入 `cc-router-tui` 即可
- 支持总览/订阅管理/虚拟模型/实时路由/请求日志，按 `?` 看键位
- 终端至少 80×24

---

## 十、验证是否跑通

```bash
# 检查代理是否监听
netstat -ano | findstr 23456

# 直接调一次入口（把 <token> 换成你的）
curl http://127.0.0.1:23456/v1/models -H "x-api-key: <token>"

# 用 Claude Code 发一句话测试
claude "你好，简单回一句"
```

---

## 十一、常见问题排查

| 现象                      | 原因与处理                                                        |
| ----------------------- | ------------------------------------------------------------ |
| 连不上 127.0.0.1:23456     | app 没启动 / 代理服务没起；看设置页「重启代理服务」                                |
| 401 鉴权失败                | token 不对；确认 `auth_enabled` 与 `auth_token`，或用 app 设置页里的 token |
| 工具报「模型不存在」              | 虚拟模型槽位没绑定订阅，或模型名不在别名表；回 app 检查绑定                             |
| 全部请求失败                  | 所有订阅都不可用；逐个「测试连接」看哪家挂                                        |
| 想局域网用                   | 设置 `listen_all=true`，客户端指向你的局域网 IP                           |
| 端口被占                    | 23456 被别的程序占用；改 `proxy_port` 后重启代理                           |
| 提示 Port 1420 in use（开发） | `Get-NetTCPConnection -LocalPort 1420` 找到进程杀掉                |
| 503「模型不存在」（池模式）     | 模型 ID 填了真实模型名（如 `qwen3.8-max`）；池模式只能填能力名 `model-llm` 等 |
| 400 `invalid_request_error` | pool 模式下发了四槽旧名（`model-opus` 等）；改用 `model-llm` 或切回 slots 模式 |
| 请求偶发变慢 / 限流         | 上游返回 `Retry-After` 时，cc-router 会按 30s~30min 冷却后自动重试，属正常      |

---

## 十二、更新

- 安装版：app 内自动更新（GitHub / 中国源可切）
- 源码版：`git pull` + `pnpm install` + 重新 build

---

## 十三、能力分类池模式（pool）与 Trae 接入

cc-router 有两套**互相独立、可共存**的路由范式：

| 范式 | 虚拟模型名 | 典型用途 | 门禁 |
| --- | --- | --- | --- |
| 经典四槽 (slots) | `model-fable` / `model-opus` / `model-sonnet` / `model-haiku` | Claude Code 等 Anthropic 工具 | 默认开 |
| 能力分类池 (pool) | `model-llm` / `model-vision` / `model-multimodal` / `model-speech` / `model-embedding` | OpenAI 协议工具（Trae / Open WebUI / Cline 等） | **任何模式下都可用** |
| 两者通用 | `model-jev`（仅 `/v1/systemone` 入口）、`model-fallback` | — | — |

`settings.routing_mode`（`slots` 默认 / `pool`）只决定**门禁**与 `/v1/models` 清单过滤：

- `slots`（默认）：四槽旧名可用，**能力池名也可用**——能力池范式在任何模式下都不被拦截（2026-10 修订：原为两套互斥，现已解耦）。
- `pool`：四槽旧名被拦截（返回 `400` 并提示改用能力名），只暴露能力池。

> 关键：想用能力池（`model-llm` 等）**不需要**切到 pool 模式；slots 模式下直接填 `model-llm` 也能用。  
> 切到 pool 模式的意义只是把四槽旧名挡掉，防止工具误发 `model-opus` 这类名字。

### 用池订阅接入 Trae（OpenAI Chat Completions）

`routing_mode` 保持 `slots` 即可（切 `pool` 也行）。在 Trae 里：

| 字段 | 填什么 |
| --- | --- |
| API 格式 | OpenAI Chat Completions |
| 自定义请求地址 | `http://127.0.0.1:23456/v1` |
| 完整 URL 开关 | **关**（Trae 自动在末尾拼 `/chat/completions`） |
| 模型 ID | `model-llm`（对话主力）；视觉用 `model-multimodal`；纯图理解用 `model-vision` |
| API 密钥 | cc-router 设置页的**访问令牌**（token 留空会 401） |

两个坑：

- 地址末尾**不要**手填 `/chat/completions`——开关关闭时 Trae 自己拼，你填了会变成双拼 → 404。
- 模型 ID 只能填**能力名**（`model-llm` 等）。填真实模型名（如 `qwen3.8-max`）会走 fallback → 经典组 → 0 候选 → **503**。

接入前用 curl 验通道（代理需先启动）：

```bash
# 免鉴权, 看池里有哪些能力模型
curl http://127.0.0.1:23456/v1/models

# 走一次池链（把 <token> 换成你的访问令牌）
curl http://127.0.0.1:23456/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <token>" \
  -d '{"model":"model-llm","messages":[{"role":"user","content":"hi"}],"max_tokens":32}'
```

第二条返回 200 且有回复，Trae 照上表填即可通。若改过代理端口（默认 23456），地址里的端口同步换掉。

### 池订阅的调度与降序

每个能力虚拟模型支持三种调度策略（`RoutingMode`，与全局 `routing_mode` 是两码事）：

- `sequential`（顺序）：按优先级从高到低，失败一个试下一个。
- `round_robin`（轮询）：在多条候选间轮流。
- `sticky`（会话亲和）：同一会话固定走同一条。

某条候选连续失败会被**持久化降序**到链尾（写库），下次优先用更健康的候选；恢复后顺序保留。上游若返回 `Retry-After` 头，冷却时长会被 **clamp 到 30s~30min**（默认 1800s 上限），避免长时间空等或频繁重试。

### 备份与还原（含模型池）

应用内「设置 → 备份 / 导出」生成的备份文件**现已包含模型池配置**（各订阅的能力链与优先级）。还原时：

- 本次**新增**的订阅，其池配置一并还原；
- 本地**已存在**的订阅，其池配置**不会被覆盖**（与订阅调度/配额逻辑一致，避免抹掉你手动调过的优先级）。

如需整体覆盖，先删掉对应订阅再导入。

---

*教程完。安全提示：auth_token 是你的代理口令，别泄露；订阅账号被风控的风险自担（见 README 免责声明）。*
