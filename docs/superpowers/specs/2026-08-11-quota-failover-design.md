# 额度耗尽自动换号（Quota Failover）设计

日期：2026-08-11
状态：已确认（经分节评审）

## 背景与问题

当前网关的账号选择是**粘性路由**（rendezvous 哈希）：同一会话 + 模型的请求始终落到
同一账号（`src/routes/proxy.rs` 的 `select_sticky_account`）。若该账号额度耗尽，
上游返回额度不足错误，网关目前把错误**原样透传**给客户端——会话的后续请求会持续失败，
用户必须手动换号、换会话或禁用该号。

## 目标

- 自动检测「额度耗尽」类上游错误（不只是 HTTP 402，还包括 429 / 4xx 中携带额度/余额语义的）。
- 按粘性优先级**扫整个账号池**，找到支持该模型且可用的账号重试。
- 对已耗尽账号设置**内存冷却期**（15 分钟），期间自动绕过，冷却后自动恢复。
- 保留会话粘性与缓存复用：同一会话的所有换号落点**确定性**地落到同一个备用账号。
- 移除 `X-Key-Id` / `X-Key-Name` 显式覆盖机制（系统不再支持固定账号）。
- 覆盖全部入口：`/v1/*`（`proxy_inner`）、原生 `/messages`、`/responses`、以及
  Messages → Chat Completions 兼容适配路径。

## 非目标

- 不主动巡检额度（不依赖 Cookie，不做后台拉取）。
- 不持久化冷却状态（服务重启即清空，符合额度按周/月重置的场景）。
- 不自动禁用账号（`is_enabled` 由用户手动控制，冷却不写库）。
- 不处理非额度类错误（模型不存在、限流、上下文超长等仍原样透传）。

## 设计

### 1. 额度耗尽判定：`is_quota_error(status, body) -> bool`

纯函数，新增于 `src/routes/proxy.rs`（`messages.rs` 复用）：

- **HTTP 402**（Payment Required）→ 直接判定为额度耗尽。
- **其它 4xx**（重点 429）→ 将响应体解析为 JSON，在 OpenAI 风格错误的
  `error.message` / `error.type` / `error.code` 字段中做**不区分大小写**的关键词匹配；
  命中任一关键词即判定为额度耗尽：
  - `quota` / `insufficient` / `balance` / `payment` / `billing` / `credit` / `exhausted`
  - `额度` / `余额`
- 其余情况一律返回 `false`。

关键词写成**常量数组**集中存放，将来看到真实报错格式随时补充。
约束是**不误伤**：普通 429 限流（`rate_limit_exceeded`）、模型不存在的 400 等不会触发换号。

> 说明：OpenCode 官方公开资料中没有额度错误的确定格式，故采用「状态码 + 关键词白名单」，
> 以容忍度换取覆盖度。关键词匹配的是「语义字段」而非原始响应体，避免把无关 JSON 文本误判。

### 2. 冷却注册表

- `AppState` 新增 `cooldowns: Arc<Mutex<HashMap<String, i64>>>`，
  键为账号 id，值为冷却到期时刻（Unix 秒）。
- 冷却时长：常量 `QUOTA_COOLDOWN_SECS = 900`（15 分钟）。
- 标记时机：仅在随机路由路径下，某账号请求返回「额度耗尽」判定时标记（写入
  `now + 900`；并发重复标记取最大值，幂等）。
- 生效点：候选账号过滤时排除在冷却期内的账号，效果等同自动绕过；到期后自然回到候选池。
- 纯内存、进程级，不落库。

### 3. 候选账号有序化：`ordered_candidates`

把现有 `select_sticky_account`（返回单个哈希最大者）演进为**返回按 rendezvous 哈希
降序排列的完整有序列表**：

- 候选 = 启用（`is_enabled`）且模型缓存匹配的账号（现有 `candidates_for_model` 逻辑）；
- 额外排除冷却期内的账号；
- 按 rendezvous 哈希值**从大到小**排序。

性质：

- 排序与输入顺序无关，现有测试 `sticky_routing_is_stable_across_requests_and_candidate_order`
  天然继续成立。
- 第一选择仍是「粘性选中的账号」；换号顺序 = 哈希降序，对同一会话**确定性**，
  不会在多个备用账号之间抖动，最大化备用账号上的缓存复用。
- `X-Key-Id` / `X-Key-Name` 分支删除后，`resolve_target` 不再需要，
  路由只走这一个候选池逻辑。

### 4. `proxy_inner` 重试循环

重构 `proxy_inner`，把「resolve → 解密 → 建 client → 发请求 → 处理响应」包进循环：

1. 进入时用 `ordered_candidates` 拿到有序候选列表（一次）。
2. 每轮取第一个「未尝试」的候选账号。
3. 空列表需区分两种原因：**没有任何启用账号**（含模型缓存完全不匹配）→ 保留现有
   `no account configured` 错误；**启用账号全部在冷却期或已全部尝试** → 返回
   「全部候选账号额度耗尽」错误。
3. 发送上游请求（请求体 `body_bytes` 已缓冲，可安全复用）。
4. 对响应做判定：
   - **非额度错误**（4xx/5xx 且非额度）→ 按现有逻辑处理（缓冲/透传、日志），结束。
   - **额度错误** → 标记该号冷却、记录失败日志、进入下一轮。
   - **成功**（含流式）→ 按现有逻辑处理（透传/缓冲），结束。
5. 关于**流式**：`stream: true` 时，为避免「响应头已发出无法重试」，改为**先缓冲**——
   累积到出现完整的 `data:` 帧或错误响应体后再判定：
   - 额度错误 → 换号重试；
   - 否则 → 把已缓冲字节 + 后续流原样透传。
   - 代价：首帧延迟增加约一个网络往返；粘性保证该会话只发生一次，换号成功后后续请求直接命中新号。

### 5. `messages.rs` 适配路径重试

Messages → Chat Completions 兼容路径（`messages_inner` 中非原生 Messages 模型的
分支，当前自建 upstream 请求）同样加入重试循环：

- 复用 `is_quota_error` / 冷却标记 / 有序候选逻辑；
- 每轮重试用现有纯函数 `to_openai_request(&input)` **重新构建** OpenAI 请求体
  （该函数无副作用、成本低），并用候选账号的 key / client 发送；
- 响应处理与日志与现有逻辑一致。

原生 `/messages`（`native_endpoint_for_model == "messages"`）与 `/responses` 直接走
`proxy_inner`，自动获得换号，**零改动**。

### 6. 日志

- 每次「额度耗尽」的失败尝试：记录一条失败日志，`status` 为该次上游额度错误的状态码，
  `error` 字段写 `额度耗尽，切换至备用账号`。
- 换号成功后的最终响应：照常记录成功日志（status 200）。
- 整个候选池全部耗尽：记录一条失败日志，`error` 注明「全部候选账号额度耗尽」，并把
  **最后一次**的原始额度错误返回客户端（保持对客户端透明的 OpenAI 错误格式）。
- 冷却期内被绕过的账号不写日志（保持日志行简单、聚焦真实请求）。

### 7. 测试

- `is_quota_error`：402 → true；429 + quota 关键词 → true；`rate_limit_exceeded` → false；
  200 → false。
- `ordered_candidates`：哈希降序确定性与输入顺序无关；冷却期账号被排除；
  冷却到期后恢复。
- 现有单元测试全数保持通过。

## 文件改动

| 文件 | 改动 |
|---|---|
| `src/state.rs` | 新增 `cooldowns` 注册表字段与初始化 |
| `src/routes/proxy.rs` | `is_quota_error`、`ordered_candidates`、`proxy_inner` 重试循环、移除 `X-Key-Id`/`X-Key-Name` 分支、冷却标记与过滤、常量 `QUOTA_COOLDOWN_SECS`、测试 |
| `src/routes/messages.rs` | 适配路径重试循环，复用共享判定/冷却/候选逻辑 |
| `src/routes/logs.rs` | （如需要）失败日志文案的共享常量 |
| `README.md` | 删除 `X-Key-Id` / `X-Key-Name` 说明 |

## 不做（明确排除）

- 额度巡检 / 主动禁用账号
- 冷却持久化
- 失败换号的用户可配置项（先以常量实现，YAGNI）
