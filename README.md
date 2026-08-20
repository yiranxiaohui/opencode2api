# opencode2api

一个本地运行的 **OpenCode 账号管理 + 多协议代理网关**。集中保管多个
OpenCode API Key，一键测试连通性、搜索标签、导入导出，并提供统一的 `/v1/*` 调用入口。
OpenCode 官方 Base URL（`https://opencode.ai/zen/go/v1`）由系统内置，无需填写。

API Key 使用 **登录密码派生密钥 AES-256-GCM 加密** 后存于 SQLite，数据库里只有密文。

## 功能

- **账号管理**：增删改查、搜索、标签筛选、启用/禁用
- **网页登录导入**：在无桌面的服务器中启动临时 Chromium，通过网页虚拟桌面手动登录并自动读取 Cookie、发现 workspace 与 API Key；支持把本机文本自动粘贴到远程当前输入框、接收远程复制内容，并可让登录与账号使用绑定同一个代理出口
- **服务器订阅 Go**：为 Cookie 账号启动同一套临时 Chromium，注入已有登录 Cookie，并通过账号绑定代理打开 workspace 的 Go 订阅页
- **连通性测试**：一键请求 OpenCode 官方 `/models`，显示延迟与模型列表并缓存
- **模型管理**：汇总账号支持的模型，可全局启用或禁用并控制网关访问
- **统一代理**：`POST /v1/chat/completions` 等，SSE 流式原样透传；使用会话粘性哈希在支持请求模型的账号池中负载均衡，避免连续对话切换账号导致缓存未命中
- **访问密钥**：为调用代理的客户端自动生成独立 API Key，可随时撤销
- **管理 API Token**：为自动化脚本创建独立的只读或读写 Bearer Token，支持审计最后使用时间与随时撤销
- **代理池**：管理一组 HTTP / HTTPS / SOCKS5 出口转发代理，每个账号可挂一个；网关转发、连通性测试、`/v1/models` 聚合均走该代理
- **多协议模型路由**：同一账号可同时使用 Chat Completions、Responses 和 Anthropic Messages 模型；原生 Messages 模型直接透传，其余模型仍支持 Messages → Chat Completions 兼容转换
- **请求日志与用量统计**：按客户端密钥、路由账号、模型、状态和时间范围筛选并分页查看请求，记录延迟、首 Token 延迟以及输入、输出和缓存 Token 用量
- **导入 / 导出**：JSON 一键迁移，导出含明文 Key（注意保管）
- **一键复制**：URL 与 Key 随时复制，Key 仅在点击时解密
- **本机运行**：仅绑定 `127.0.0.1`；登录状态可在服务重启后自动恢复，也可从管理页面主动退出

## 快速开始

### 开发模式（两个终端）

```bash
# 终端 1：后端 API（127.0.0.1:8787）
cd opencode2api
cargo run

# 终端 2：前端（http://localhost:5173）
cd frontend
bun install
bun run dev
```

### 生产模式（单二进制 + 静态资源）

```bash
cd frontend && bun run build && cd ..
cargo build --release
./target/release/opencode2api
# 打开 http://127.0.0.1:8787
```

## 用法

1. **首次运行**：设置登录密码，创建账号。
2. **新增账号**：可填入 API Key，也可点击「网页登录导入」，在服务器的临时 Chromium 中手动登录后自动导入；Base URL 由系统自动处理。
3. **创建访问密钥**：在「密钥管理」中创建供客户端调用网关的 API Key。
4. **代理调用**：将兼容客户端指向 `http://127.0.0.1:8787/v1`：

```bash
curl -N http://127.0.0.1:8787/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "X-Session-Id: conversation-123" \
  -d '{"model":"deepseek-chat","stream":true,"messages":[{"role":"user","content":"你好"}]}'
```

> 建议为每段对话传入稳定且唯一的 `X-Session-Id`（也兼容 `X-Conversation-Id`）。同一会话和模型始终映射到同一账号，不同会话则分散到账号池。未传会话 ID 时，以客户端访问密钥和模型作为粘性键；模型缓存未命中时在全部账号中执行相同的粘性选择。
> 额度耗尽的账号会自动切换到候选池中的备用账号，并持久化为“额度耗尽”状态，不再接收外部流量。通过 Cookie 导入的账号由系统独立查询额度；只有查询确认额度恢复后才重新加入路由，服务重启也不会提前清除该状态。SDK 用法示例：

仅使用 API Key 添加的账号没有独立额度查询能力，耗尽后会保持退出路由；充值后需要在账号编辑中更新 API Key，或改用 Cookie 导入以启用自动恢复检测。

```python
from openai import OpenAI

client = OpenAI(
    base_url="http://127.0.0.1:8787/v1",
    api_key="sk-...",  # 在左侧「密钥管理」中创建并复制
)
print(client.chat.completions.create(
    model="deepseek-chat",
    messages=[{"role": "user", "content": "你好"}],
    extra_headers={"X-Session-Id": "conversation-123"},
))
```

管理 API Token 可在「设置」中创建。完整 Token 仅显示一次：

```bash
curl http://127.0.0.1:8787/api/keys \
  -H "Authorization: Bearer oca_admin_..."
```

`admin:read` 权限允许 `GET`/`HEAD` 请求，`admin:write` 权限允许其他修改请求。管理 Token 与网页登录、退出登录及 `/v1/*` 客户端访问密钥相互独立。

## 配置（环境变量，均可选）

| 变量 | 默认值 | 说明 |
|---|---|---|
| `OPENCODE2API_BIND` | `127.0.0.1:8787` | 监听地址 |
| `OPENCODE2API_DATA_DIR` | `./data` | SQLite 与数据库所在目录 |
| `OPENCODE2API_WEB_DIST` | `./frontend/dist` | 前端构建产物目录 |
| `OPENCODE2API_CHROMIUM_BIN` | 自动查找 | Chromium 可执行文件；官方 Docker 镜像使用内置的低权限启动包装器 |
| `OPENCODE2API_XVFB_BIN` | `Xvfb` | Xvfb 可执行文件 |
| `OPENCODE2API_X11VNC_BIN` | `x11vnc` | x11vnc 可执行文件 |
| `OPENCODE2API_CHROMIUM_NO_SANDBOX` | `false` | 为 Chromium 添加 `--no-sandbox`；仅在运行环境无法使用沙箱且已接受安全风险时启用 |
| `RUST_LOG` | `info` | 日志级别 |

## API 一览

| 方法 | 路径 | 说明 |
|---|---|---|
| `GET` | `/api/status` | `{installed, logged_in, key_count}` |
| `POST` | `/api/auth/setup` / `login` / `logout` / `change-password` | 初始化、登录、退出登录与修改登录密码 |
| `GET`/`POST`/`DELETE` | `/api/admin-tokens[/{id}]` | 管理独立 Bearer Token（仅限网页登录并验证密码） |
| `GET` | `/api/keys?q=&tag=` | 账号列表（不含 Key） |
| `GET` | `/api/keys/{id}` | 账号详情（含解密 Key） |
| `POST`/`PUT`/`DELETE` | `/api/keys[/{id}]` | 增删改 |
| `GET`/`POST`/`DELETE` | `/api/browser-login[/{id}]` | 查询状态 / 启动 / 结束临时网页登录会话 |
| `GET` | `/api/browser-login/{id}/vnc` | 登录会话的受保护虚拟桌面 WebSocket |
| `POST` | `/api/browser-login/{id}/capture` | 读取 OpenCode Cookie、验证并导入账号 |
| `POST` | `/api/keys/{id}/browser/go` | 使用账号 Cookie 与绑定代理启动 Go 订阅浏览器 |
| `POST` | `/api/keys/{id}/test` | 连通性测试（`{ok, latency_ms, models}`） |
| `POST` | `/api/keys/{id}/set-enabled` | 启用或禁用账号 |
| `GET` | `/api/keys/{id}/usage` | 查询通过 Cookie 导入账号的套餐额度 |
| `GET` | `/api/keys/{id}/invite-link` | 获取单个 Cookie 账号的邀请链接 |
| `GET` | `/api/keys/{id}/invite-rewards` | 查询单个 Cookie 账号的邀请奖励列表 |
| `POST` | `/api/keys/{id}/invite-rewards/{reward_id}/claim` | 使用一条待领取的邀请奖励 |
| `GET` | `/api/models` | 获取模型管理列表及启用状态 |
| `POST` | `/api/models/set-enabled` | 全局启用或禁用指定模型 |
| `GET`/`POST`/`DELETE` | `/api/client-keys[/{id}]` | 管理客户端访问密钥 |
| `GET`/`POST` | `/api/proxies` | 代理池列表 / 新增 |
| `PUT`/`DELETE` | `/api/proxies/{id}` | 编辑 / 删除代理（删除后相关账号恢复直连） |
| `GET`/`DELETE` | `/api/logs` | 查询 / 清空请求日志 |
| `GET` | `/api/logs/stats` | 查询请求与 Token 用量汇总 |
| `GET` | `/api/export` / `POST` `/api/import` | 导出 / 导入 JSON（含代理池与账号代理关联） |
| `GET`/`POST` | `/v1/{*path}` | 统一代理（需 Bearer 访问密钥，支持流式） |
| `GET` | `/v1/models` | 并发汇总全部账号的模型并按 ID 去重 |
| `POST` | `/v1/messages` | 原生 Messages 透传；其他模型兼容转换为 Chat Completions |

OpenCode Zen 会根据模型使用不同协议。客户端仍统一连接本服务的 `/v1`，但应使用对应 SDK：

| 模型 | 本地/上游端点 | AI SDK 包 |
|---|---|---|
| `gpt-5.6-luna` | `/v1/responses` | `@ai-sdk/openai` |
| `minimax-m3`, `minimax-m2.7`, `minimax-m2.5`, `qwen3.8-max`, `qwen3.7-max`, `qwen3.7-plus`, `qwen3.6-plus` | `/v1/messages` | `@ai-sdk/anthropic` |
| `grok-4.5`, `glm-5.2`, `glm-5.1`, `kimi-k3`, `kimi-k2.7-code`, `kimi-k2.6`, `deepseek-v4-pro`, `deepseek-v4-flash`, `mimo-v2.5`, `mimo-v2.5-pro`, `hy3` | `/v1/chat/completions` | `@ai-sdk/openai-compatible` |

旧数据库升级后首次运行需要登录一次，之后服务会自动恢复。

## 安全说明

- 仅绑定回环地址；**没有**开放 CORS —— 浏览器同源才能访问，防恶意网页调用代理。
- 如修改监听地址向其他主机开放，必须放在 HTTPS 反向代理后；Cookie、管理 Token 和客户端密钥都应视为高敏感凭据。
- 网页登录使用只存哈希的 HttpOnly、SameSite=Strict 会话 Cookie；每个浏览器会话相互独立。
- 管理 API Token 具有 `admin:read` / `admin:write` 权限范围，数据库只保存 SHA-256 哈希，完整 Token 仅在创建时返回一次。
- 上游请求禁跟随重定向，避免 Bearer Token 泄露给第三方主机。
- 登录密码经 argon2id（`m=19456,t=2,p=1`）派生 AES-256 密钥，Key 与代理 URL（可能含凭据）
  逐个用随机 nonce 加密；加密密钥持久化后由网关使用，不依赖某个网页会话。
- 持久化的加密密钥与密文保存在同一数据目录，因此该设计用于避免凭据明文落盘，不能抵御整个数据目录被窃取的情况。请使用操作系统权限和磁盘加密保护数据目录。
- 明文 Key 只短暂存在于进程内存，绝不落盘、不写日志。
- 网页登录会话同一时间只允许一个，使用独立的临时 Chromium 配置，15 分钟后自动结束；VNC 仅监听容器回环地址，并由已登录的同源管理 WebSocket 转发。
- Cookie 账号的 Go 订阅入口也使用临时 Chromium；Cookie 仅在进程内注入临时配置，关闭或超时后随配置目录销毁。
- 网页登录的 Chromium 默认保留沙箱。不要轻易启用 `OPENCODE2API_CHROMIUM_NO_SANDBOX`；选择代理后，临时回环代理桥会让 Chromium、Cookie 验证和后续网关请求使用同一个 HTTP、HTTPS、SOCKS5 或 SOCKS5H 出口，并支持代理用户名密码。

## 目录结构

```
src/
├── main.rs          # 入口
├── browser_login.rs # 无桌面 Chromium 网页登录与 Cookie 读取
├── config.rs        # 环境变量配置
├── crypto.rs        # argon2id 派生 + AES-GCM 加解密（安全核心）
├── db.rs            # rusqlite CRUD
├── migration/       # SeaORM Migration 数据库版本迁移
├── proxy_bridge.rs  # Chromium 到账号绑定代理的临时回环桥
├── state.rs         # AppState（DB、内存密钥、reqwest 客户端）
├── error.rs         # 统一错误
├── middleware.rs    # 网页会话、管理 Token 与网关状态校验
└── routes/          # auth / admin_tokens / keys / import_export / proxy
frontend/src/
├── App.tsx          # 设置/登录/主界面切换
├── api/             # fetch 封装 + 类型
├── hooks/           # React Query hooks
└── components/      # 账号管理、对话、详情抽屉、表单等
```

## 技术栈

Rust：axum 0.8 · tokio · rusqlite(bundled) · SeaORM Migration · argon2 · aes-gcm · reqwest 0.13
前端：React 19 · Vite · TypeScript · TanStack Query（无 UI 框架，手写暗色主题）

## Docker

使用 Docker Compose：

```bash
docker compose up -d
```

默认拉取 GHCR 最新镜像；如需基于当前源码重新构建：

```bash
docker compose build
docker compose up -d
```

可通过 `OPENCODE2API_PORT=9000 docker compose up -d` 修改宿主机端口。数据保存在
`opencode2api-data` 命名卷中。

也可以直接使用 Docker：

```bash
docker run -d \
  --name opencode2api \
  -p 8787:8787 \
  -v opencode2api-data:/data \
  ghcr.io/yiranxiaohui/opencode2api:latest
```

服务默认监听 `0.0.0.0:8787`，SQLite 数据保存在 `/data`。
官方镜像已经包含 Chromium、Xvfb 和 x11vnc，无桌面服务器无需额外安装。直接运行二进制并使用「网页登录导入」时，需要自行安装这三个程序。
推送 `v*` 版本标签会自动创建 GitHub Release，并发布 amd64/arm64 镜像到 GHCR。

## 已知限制 / Roadmap

- 导出文件含明文 Key，请妥善保管
- 请求日志与 Token 用量统计保存在本地；套餐额度查询仅支持通过 Cookie 导入的账号
- 网页登录同一时间只允许一个临时浏览器会话
- 如上游是每次连接自动换 IP 的轮换代理，需要在代理服务商侧配置粘性会话；本服务保证复用同一代理配置，但无法阻止上游自行更换出口
- 暂无多用户 / 共享访问（设计为单人本地工具）

## 许可证

本项目采用 [MIT License](LICENSE) 开源。
