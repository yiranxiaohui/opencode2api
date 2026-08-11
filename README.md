# opencode2api

一个本地运行的 **OpenCode 账号管理 + 多协议代理网关**。集中保管多个
OpenCode API Key，一键测试连通性、搜索标签、导入导出，并提供统一的 `/v1/*` 调用入口。
OpenCode 官方 Base URL（`https://opencode.ai/zen/go/v1`）由系统内置，无需填写。

API Key 使用 **登录密码派生密钥 AES-256-GCM 加密** 后存于 SQLite，数据库里只有密文。

## 功能

- **账号管理**：增删改查、搜索、标签筛选、启用/禁用、设置默认账号
- **连通性测试**：一键请求 OpenCode 官方 `/models`，显示延迟与模型列表并缓存
- **统一代理**：`POST /v1/chat/completions` 等，SSE 流式原样透传；使用会话粘性哈希在支持请求模型的账号池中负载均衡，避免连续对话切换账号导致缓存未命中
- **访问密钥**：为调用代理的客户端自动生成独立 API Key，可随时撤销
- **代理池**：管理一组 HTTP / HTTPS / SOCKS5 出口转发代理，每个账号可挂一个；网关转发、连通性测试、`/v1/models` 聚合均走该代理
- **多协议模型路由**：同一账号可同时使用 Chat Completions、Responses 和 Anthropic Messages 模型；原生 Messages 模型直接透传，其余模型仍支持 Messages → Chat Completions 兼容转换
- **导入 / 导出**：JSON 一键迁移，导出含明文 Key（注意保管）
- **一键复制**：URL 与 Key 随时复制，Key 仅在点击时解密
- **本机运行**：仅绑定 `127.0.0.1`；派生密钥持久化到本地数据目录，服务重启后自动恢复

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
2. **新增账号**：填入名称和 API Key；Base URL 由系统自动处理。
3. **设为默认**：作为内置对话页面的初始账号。
4. **代理调用**：任何 OpenAI 兼容客户端指向 `http://127.0.0.1:8787/v1`：

```bash
curl -N http://127.0.0.1:8787/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "X-Session-Id: conversation-123" \
  -d '{"model":"deepseek-chat","stream":true,"messages":[{"role":"user","content":"你好"}]}'
```

> 建议为每段对话传入稳定且唯一的 `X-Session-Id`（也兼容 `X-Conversation-Id`）。同一会话和模型始终映射到同一账号，不同会话则分散到账号池。未传会话 ID 时，以客户端访问密钥和模型作为粘性键；模型缓存未命中时在全部账号中执行相同的粘性选择。
> 额度耗尽的账号会自动切换到候选池中的备用账号，15 分钟冷却后恢复。SDK 用法示例：

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

## 配置（环境变量，均可选）

| 变量 | 默认值 | 说明 |
|---|---|---|
| `OPENCODE2API_BIND` | `127.0.0.1:8787` | 监听地址 |
| `OPENCODE2API_DATA_DIR` | `./data` | SQLite 与数据库所在目录 |
| `OPENCODE2API_WEB_DIST` | `./frontend/dist` | 前端构建产物目录 |
| `RUST_LOG` | `info` | 日志级别 |

## API 一览

| 方法 | 路径 | 说明 |
|---|---|---|
| `GET` | `/api/status` | `{installed, unlocked, key_count}` |
| `POST` | `/api/auth/setup` / `unlock` / `change-password` | 初始化、旧数据库迁移与修改登录密码 |
| `GET` | `/api/keys?q=&tag=` | 账号列表（不含 Key） |
| `GET` | `/api/keys/{id}` | 账号详情（含解密 Key） |
| `POST`/`PUT`/`DELETE` | `/api/keys[/{id}]` | 增删改 |
| `POST` | `/api/keys/{id}/test` | 连通性测试（`{ok, latency_ms, models}`） |
| `POST` | `/api/keys/{id}/set-default` | 设为默认 |
| `GET`/`POST`/`DELETE` | `/api/client-keys[/{id}]` | 管理客户端访问密钥 |
| `GET`/`POST` | `/api/proxies` | 代理池列表 / 新增 |
| `PUT`/`DELETE` | `/api/proxies/{id}` | 编辑 / 删除代理（删除后相关账号恢复直连） |
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
- 上游请求禁跟随重定向，避免 Bearer Token 泄露给第三方主机。
- 登录密码经 argon2id（`m=19456,t=2,p=1`）派生 AES-256 密钥，Key 与代理 URL（可能含凭据）
  逐个用随机 nonce 加密；派生密钥会持久化，使服务重启后能够自动恢复。
- 明文 Key 只短暂存在于进程内存，绝不落盘、不写日志。

## 目录结构

```
src/
├── main.rs          # 入口
├── config.rs        # 环境变量配置
├── crypto.rs        # argon2id 派生 + AES-GCM 加解密（安全核心）
├── db.rs            # rusqlite CRUD
├── migration/       # SeaORM Migration 数据库版本迁移
├── state.rs         # AppState（DB、内存密钥、reqwest 客户端）
├── error.rs         # 统一错误
├── middleware.rs    # Unlocked 提取器（423）
└── routes/          # auth / keys / import_export / proxy
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
推送 `v*` 版本标签会自动创建 GitHub Release，并发布 amd64/arm64 镜像到 GHCR。

## 已知限制 / Roadmap

- 导出文件含明文 Key，请妥善保管
- 暂无基于 Key 的用量统计与余额查询
- 暂无多用户 / 共享访问（设计为单人本地工具）

## 许可证

本项目采用 [MIT License](LICENSE) 开源。
