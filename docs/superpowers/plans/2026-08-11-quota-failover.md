# Quota-Exhaustion Auto Failover Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a sticky-routed account returns a quota-exhaustion error, the gateway automatically retries the request across the remaining account pool (deterministic failover order, 15-minute cooldown per exhausted account), covering `/v1/*`, native `/messages` & `/responses`, and the Messages→Chat-Completions adapter path; `X-Key-Id`/`X-Key-Name` explicit overrides are removed.

**Architecture:** A pure `is_quota_error` classifier + an in-memory per-account cooldown registry in `AppState` + a rendezvous-ordered candidate list replace the single-shot `resolve_target`. `proxy_inner` and the `messages.rs` adapter loop over candidates, mark exhausted accounts in cooldown, and forward the last quota error only when the whole pool fails. Successful streams are forwarded unchanged (non-success responses were already fully buffered by `should_forward_stream`, so classification costs no first-token latency).

**Tech Stack:** Rust (edition 2024), axum 0.8, reqwest 0.13, rusqlite. No new dependencies.

## Global Constraints

- Work on branch `master` (currently clean, up to date with `origin/master`). Do not touch or revert the merged quota-caching work.
- Spec: `docs/superpowers/specs/2026-08-11-quota-failover-design.md`. Every task's requirements implicitly include it.
- No new Cargo dependencies.
- Visibility: new shared items are `pub(crate)`; module-local helpers stay `fn` (private).
- Logging is best-effort via existing helpers `logs::record_failure` / `logs::insert_log` (never break the proxied reply on a logging error).
- User-facing strings are Simplified Chinese (matches existing codebase, e.g. `"no account configured"` stays English — that is existing behavior; new user-facing log strings use Chinese).
- After every task: `cargo test` must pass (unit tests only; no HTTP-mock infrastructure exists — the retry loop is verified manually per Task 6's manual steps). `cargo build` must not warn.
- Commit messages use Conventional Commits and end with:
  `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`
- Do not amend existing commits; do not commit unrelated files.

---

### Task 1: `is_quota_error` classifier

**Files:**
- Modify: `src/routes/proxy.rs` (add near `header_str`, ~line 464)
- Test: `src/routes/proxy.rs` (`mod tests`)

**Interfaces:**
- Consumes: `axum::http::StatusCode` (already imported).
- Produces: `pub(crate) const QUOTA_KEYWORDS: &[&str]` and
  `pub(crate) fn is_quota_error(status: StatusCode, body: &[u8]) -> bool`.
  Task 2 uses `QUOTA_KEYWORDS` (not directly), Task 5/6 use `is_quota_error`.

- [ ] **Step 1: Add the failing test**

In `src/routes/proxy.rs` `mod tests`, add `is_quota_error` to the `use super::{...}` list, then append this test:

```rust
    #[test]
    fn quota_errors_are_recognized_without_false_positives() {
        let quota_body =
            br#"{"error":{"message":"You have exhausted your monthly quota","type":"insufficient_quota","code":null}}"#;
        // HTTP 402 is a hard signal regardless of body.
        assert!(is_quota_error(StatusCode::PAYMENT_REQUIRED, b"{}"));
        // 429 with quota semantics in error fields.
        assert!(is_quota_error(StatusCode::TOO_MANY_REQUESTS, quota_body));
        // Chinese-language body.
        assert!(is_quota_error(
            StatusCode::BAD_REQUEST,
            br#"{"error":{"message":"余额不足"}}"#
        ));
        // Plain rate limiting must NOT trigger failover.
        assert!(!is_quota_error(
            StatusCode::TOO_MANY_REQUESTS,
            br#"{"error":{"message":"Rate limit exceeded","type":"rate_limit_exceeded"}}"#
        ));
        // Model-not-found / server errors / success must not.
        assert!(!is_quota_error(
            StatusCode::NOT_FOUND,
            br#"{"error":{"message":"model not found"}}"#
        ));
        assert!(!is_quota_error(StatusCode::INTERNAL_SERVER_ERROR, quota_body));
        assert!(!is_quota_error(StatusCode::OK, quota_body));
        assert!(!is_quota_error(StatusCode::TOO_MANY_REQUESTS, b"not json"));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test quota_errors_are_recognized_without_false_positives`
Expected: FAIL to compile — `is_quota_error` not defined.

- [ ] **Step 3: Implement the classifier**

Add above `fn header_str` in `src/routes/proxy.rs`:

```rust
/// Substrings that mark an upstream error body as "account quota exhausted".
/// Matched case-insensitively against OpenAI-style `error.message`/`type`/`code`.
pub(crate) const QUOTA_KEYWORDS: &[&str] = &[
    "quota",
    "insufficient",
    "balance",
    "payment",
    "billing",
    "credit",
    "exhausted",
    "额度",
    "余额",
];

/// Classify an upstream non-success response as quota exhaustion. HTTP 402 is a
/// hard signal; other 4xx bodies are scanned in the OpenAI error fields only.
/// Conservative by design: plain rate limiting (`rate_limit_exceeded`), missing
/// models, and 5xx must never trigger failover.
pub(crate) fn is_quota_error(status: StatusCode, body: &[u8]) -> bool {
    if status == StatusCode::PAYMENT_REQUIRED {
        return true;
    }
    if !status.is_client_error() {
        return false;
    }
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    let Some(err) = value.get("error") else {
        return false;
    };
    let haystack = ["message", "type", "code"]
        .into_iter()
        .filter_map(|key| err.get(key).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    QUOTA_KEYWORDS.iter().any(|keyword| haystack.contains(keyword))
}
```

`Value` is already imported (`use serde_json::{Value, json};`).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test quota_errors_are_recognized_without_false_positives`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/routes/proxy.rs
git commit -m "feat(proxy): add quota-exhaustion error classifier"
```

---

### Task 2: Cooldown registry on `AppState`

**Files:**
- Modify: `src/state.rs` (struct fields, `new()`, new methods)
- Test: `src/state.rs` (new `mod tests`)

**Interfaces:**
- Consumes: `QUOTA_COOLDOWN_SECS` defined in this task (in `proxy.rs`), `crate::models::now_secs()`.
- Produces: `AppState::begin_cooldown(&self, id: &str)` and
  `AppState::in_quota_cooldown(&self, id: &str, now: i64) -> bool`.
  Task 4/5/6 use these.

- [ ] **Step 1: Add the failing test**

Append a `mod tests` at the end of `src/state.rs` (the migration test at `src/migration/mod.rs:440` already demonstrates the temp-DB + `crate::migration::run` pattern):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn quota_cooldown_marks_and_expires() {
        let path = std::env::temp_dir()
            .join(format!("oc2a-state-{}.db", uuid::Uuid::new_v4()));
        crate::migration::run(&path).await.unwrap();
        let st = AppState::new(&path, PathBuf::from("frontend/dist")).unwrap();
        let now = crate::models::now_secs();
        assert!(!st.in_quota_cooldown("a", now));
        st.begin_cooldown("a");
        assert!(st.in_quota_cooldown("a", now));
        assert!(!st.in_quota_cooldown("a", now + crate::routes::proxy::QUOTA_COOLDOWN_SECS + 1));
        assert!(!st.in_quota_cooldown("b", now));
        let _ = std::fs::remove_file(path);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test quota_cooldown_marks_and_expires`
Expected: FAIL to compile — `QUOTA_COOLDOWN_SECS`, `begin_cooldown`, `in_quota_cooldown` missing.

- [ ] **Step 3: Define the cooldown window constant**

In `src/routes/proxy.rs`, next to `QUOTA_KEYWORDS` (Task 1):

```rust
/// How long an account that returned a quota-exhaustion error is skipped.
pub(crate) const QUOTA_COOLDOWN_SECS: i64 = 900;
```

- [ ] **Step 4: Add the registry field and methods**

In `src/state.rs`:

Add a field to `AppState` (after `proxy_pool_clients`):

```rust
    /// Account ids currently in quota-exhaustion cooldown, mapped to the Unix
    /// second at which the cooldown expires. In-memory only; cleared on restart.
    pub cooldowns: Arc<Mutex<HashMap<String, i64>>>,
```

Initialize it in `AppState::new` (inside the `Ok(Self { ... })` literal):

```rust
            cooldowns: Arc::new(Mutex::new(HashMap::new())),
```

Add these methods to `impl AppState` (e.g. after `clear_proxy_client_cache`):

```rust
    /// Mark an account as quota-exhausted for the cooldown window. Concurrent
    /// marks keep the longest remaining window (monotonic, idempotent).
    pub fn begin_cooldown(&self, id: &str) {
        let mut map = self.cooldowns.lock().unwrap();
        let until = crate::models::now_secs() + crate::routes::proxy::QUOTA_COOLDOWN_SECS;
        let entry = map.entry(id.to_string()).or_insert(until);
        *entry = (*entry).max(until);
    }

    /// True while `id` is inside its cooldown window at time `now` (Unix secs).
    pub fn in_quota_cooldown(&self, id: &str, now: i64) -> bool {
        self.cooldowns
            .lock()
            .unwrap()
            .get(id)
            .is_some_and(|until| now < *until)
    }
```

`HashMap`, `Arc`, `Mutex` are already imported in `state.rs`. `uuid` and `PathBuf` are available crate-wide.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test quota_cooldown_marks_and_expires`
Expected: PASS. Also run `cargo test` — all existing tests must still pass.

- [ ] **Step 6: Commit**

```bash
git add src/state.rs src/routes/proxy.rs
git commit -m "feat(state): add in-memory quota-exhaustion cooldown registry"
```

---

### Task 3: Rendezvous hash helper + `ordered_candidates`

**Files:**
- Modify: `src/routes/proxy.rs` (refactor `select_sticky_account`, add helpers)
- Test: `src/routes/proxy.rs` (`mod tests`)

**Interfaces:**
- Consumes: `KeyRow` (already imported in tests), `Sha256` (already imported).
- Produces: `fn rendezvous_hash(affinity: &str, id: &str) -> u64` (private),
  `fn ordered_candidates<'a, F>(candidates: &'a [KeyRow], affinity: &str, in_cooldown: F) -> Vec<&'a KeyRow> where F: Fn(&str) -> bool` (private).
  Task 4 uses `ordered_candidates` + `rendezvous_hash`; Task 5/6 rely on Task 4.

- [ ] **Step 1: Add the failing tests**

In `mod tests`, add `ordered_candidates` to the `use super::{...}` list, then append:

```rust
    #[test]
    fn ordered_candidates_are_deterministic_and_match_sticky_first() {
        let rows = vec![key("a", &["m"]), key("b", &["m"]), key("c", &["m"])];
        let affinity = "client:model:session";
        let no_cooldown = |_: &str| false;
        let order = ordered_candidates(&rows, affinity, no_cooldown);
        let ids: Vec<&str> = order.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids.len(), 3);
        // First choice is exactly the current sticky selector's choice.
        assert_eq!(
            select_sticky_account(&rows, affinity).unwrap().id.as_str(),
            ids[0]
        );
        // Order is independent of candidate input order.
        let reversed: Vec<KeyRow> = rows.iter().rev().cloned().collect();
        let reversed_ids: Vec<&str> = ordered_candidates(&reversed, affinity, no_cooldown)
            .into_iter()
            .map(|row| row.id.as_str())
            .collect();
        assert_eq!(ids, reversed_ids);
    }

    #[test]
    fn ordered_candidates_skip_accounts_in_cooldown() {
        let rows = vec![key("a", &["m"]), key("b", &["m"]), key("c", &["m"])];
        let cool_b = |id: &str| id == "b";
        let ids: Vec<&str> = ordered_candidates(&rows, "k", cool_b)
            .into_iter()
            .map(|row| row.id.as_str())
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(!ids.contains(&"b"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test ordered_candidates`
Expected: FAIL to compile — `ordered_candidates` not defined.

- [ ] **Step 3: Implement the helpers**

Replace the existing `select_sticky_account` body (currently `src/routes/proxy.rs:431-443`) and add two helpers above it:

```rust
/// Rendezvous (highest-random-weight) hash of one account under an affinity key.
fn rendezvous_hash(affinity: &str, id: &str) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(affinity.as_bytes());
    hasher.update([0]);
    hasher.update(id.as_bytes());
    let digest = hasher.finalize();
    u64::from_be_bytes(digest[..8].try_into().expect("SHA-256 prefix"))
}

/// Highest-random-weight hashing provides deterministic affinity independent of
/// database ordering and minimizes remapping when accounts are added to or
/// removed from the eligible model pool.
fn select_sticky_account<'a>(
    candidates: &'a [KeyRow],
    affinity: &str,
) -> Option<&'a KeyRow> {
    candidates
        .iter()
        .max_by_key(|row| rendezvous_hash(affinity, &row.id))
}

/// Accounts in `candidates` (already enabled + model-matched) that are not in
/// quota cooldown, ordered by descending rendezvous hash. First = the sticky
/// winner; the remaining order is the deterministic failover order for the
/// affinity key. Independent of input order.
fn ordered_candidates<'a, F>(
    candidates: &'a [KeyRow],
    affinity: &str,
    in_cooldown: F,
) -> Vec<&'a KeyRow>
where
    F: Fn(&str) -> bool,
{
    let mut ordered: Vec<&KeyRow> = candidates
        .iter()
        .filter(|row| !in_cooldown(&row.id))
        .collect();
    ordered.sort_by(|a, b| rendezvous_hash(affinity, &b.id).cmp(&rendezvous_hash(affinity, &a.id)));
    ordered
}
```

`KeyRow` is in scope in `proxy.rs`? The existing functions use `crate::db::KeyRow` (full path). `select_sticky_account` currently takes `&'a [crate::db::KeyRow]`. Keep the full path `crate::db::KeyRow` in the signatures so no new import is needed (the test module already imports `KeyRow`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test`
Expected: PASS (new tests + all existing, including `sticky_routing_is_stable_across_requests_and_candidate_order`).

- [ ] **Step 5: Commit**

```bash
git add src/routes/proxy.rs
git commit -m "feat(proxy): rendezvous-ordered candidate list with cooldown skip"
```

---

### Task 4: `routing_candidates` + remove `X-Key-Id`/`X-Key-Name`

**Files:**
- Modify: `src/routes/proxy.rs` (add `routing_candidates`, slim `resolve_target` to delegate, delete x-key branches)
- Test: `src/routes/proxy.rs` (`mod tests`)

**Interfaces:**
- Consumes: Task 1 `is_quota_error` (unused here, kept for Task 5), Task 2 cooldown methods, Task 3 `ordered_candidates`/`rendezvous_hash`, existing `candidates_for_model`.
- Produces: `pub(crate) async fn routing_candidates(st: &AppState, model: Option<&str>, affinity: &str) -> Result<Vec<KeyRow>, ApiError>`.
  Task 5 replaces `resolve_target` with it in `proxy_inner`; Task 6 uses it in `messages.rs`.

- [ ] **Step 1: Add the failing test**

In `mod tests`, add a test asserting `resolve_target` no longer honors the explicit headers. `resolve_target` needs a real `AppState`, so drive the assertion through the new pure logic instead — add a unit test for the empty/all-cooled error distinction by calling `ordered_candidates` + `candidates_for_model` directly:

```rust
    #[test]
    fn all_cooled_candidates_yield_empty_active_list() {
        let rows = vec![key("a", &["m"]), key("b", &["m"])];
        let base = candidates_for_model(rows, Some("m"));
        assert_eq!(base.len(), 2);
        let cool_all = |_id: &str| true;
        let active = ordered_candidates(&base, "k", cool_all);
        assert!(active.is_empty());
    }
```

(The actual `X-Key-Id` removal is verified by the `cargo build` succeeding after Step 3 — there is no test infra to spin an `AppState` with a real route, so we assert the new pure behavior and rely on the full-suite run.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test all_cooled_candidates_yield_empty_active_list`
Expected: PASS (it exercises existing helpers — the real change is Step 3's removal; run the whole suite after Step 3).

- [ ] **Step 3: Implement `routing_candidates` and remove the override branches**

Replace the entire `resolve_target` function (currently `src/routes/proxy.rs:368-409`) with:

```rust
/// Enabled, model-matching accounts for `model`, ordered by descending
/// rendezvous hash (first = sticky winner), with quota-cooled accounts skipped.
/// Errors distinctly when no account is configured at all vs. every eligible
/// account being in cooldown.
pub(crate) async fn routing_candidates(
    st: &AppState,
    model: Option<&str>,
    affinity: &str,
) -> Result<Vec<KeyRow>, ApiError> {
    let rows = st.db.all_key_rows()?;
    let base = candidates_for_model(rows, model);
    if base.is_empty() {
        return Err(ApiError::BadRequest("no account configured".into()));
    }
    let now = crate::models::now_secs();
    let ordered = ordered_candidates(&base, affinity, |id| st.in_quota_cooldown(id, now));
    if ordered.is_empty() {
        return Err(ApiError::BadRequest(
            "all candidate accounts are in quota cooldown".into(),
        ));
    }
    Ok(ordered.into_iter().cloned().collect())
}
```

This deletes the `x-key-id` and `x-key-name` branches. `resolve_target` no longer exists; update the module doc comment (currently `src/routes/proxy.rs:16-18`) to drop the `X-Key-Id`/`X-Key-Name` mention:

```rust
/// Unified OpenAI-compatible gateway: forwards `/v1/*` to a sticky-session key.
/// SSE (stream: true) is forwarded incrementally via `Body::from_stream`.
```

- [ ] **Step 4: Fix the two remaining `resolve_target` call sites so the build stays green**

`proxy_inner` still calls `resolve_target` (`src/routes/proxy.rs:86`) and `messages.rs:66` still calls it. Update both to the new function, taking the first (sticky winner) row to preserve today's single-attempt behavior. In `src/routes/proxy.rs`, change:

```rust
    let row = match resolve_target(&st, headers, model.as_deref(), &affinity).await {
        Ok(row) => row,
        Err(e) => {
            logs::record_failure(&st, &client, None, &method_str, path, 0, &e);
            return Err(e);
        }
    };
```

to:

```rust
    let candidates = match routing_candidates(&st, model.as_deref(), &affinity).await {
        Ok(rows) => rows,
        Err(e) => {
            logs::record_failure(&st, &client, None, &method_str, path, 0, &e);
            return Err(e);
        }
    };
    let row = &candidates[0];
```

(`routing_candidates` never returns an empty Vec — it errors instead — so `&candidates[0]` is safe.) `row` is used as `&KeyRow` below; adjust the `&row` usages to `row` where they were `&row` before? No — `row` is now `&KeyRow`; existing call sites use `&row` (`Some(&row)`, `client_for_key(&row)`), which become `Some(row)` / `client_for_key(row)`. Apply those mechanical changes in the same block (lines ~93, ~134).

In `src/routes/messages.rs:65-72`, change:

```rust
    let affinity = super::proxy::affinity_key(headers, &client_key.id, Some(&model));
    let row = match super::proxy::resolve_target(&st, headers, Some(&model), &affinity).await {
        Ok(row) => row,
        Err(e) => {
            logs::record_failure(&st, &client_key, None, "POST", "/messages", 0, &e);
            return Err(e);
        }
    };
```

to:

```rust
    let affinity = super::proxy::affinity_key(headers, &client_key.id, Some(&model));
    let candidates =
        match super::proxy::routing_candidates(&st, Some(&model), &affinity).await {
            Ok(rows) => rows,
            Err(e) => {
                logs::record_failure(&st, &client_key, None, "POST", "/messages", 0, &e);
                return Err(e);
            }
        };
    let row = &candidates[0];
```

`messages.rs` uses `&row` at lines 73, 97, 112, 139, 144 — those become `Some(row)` / `client_for_key(row)`. Apply the same mechanical `&row` → `row` fix where a reference-to-reference would otherwise appear.

- [ ] **Step 5: Run the full suite**

Run: `cargo test` and `cargo build`
Expected: PASS; no warnings. This confirms the x-key override paths are fully gone.

- [ ] **Step 6: Commit**

```bash
git add src/routes/proxy.rs src/routes/messages.rs
git commit -m "feat(proxy): replace resolve_target with cooldown-aware routing_candidates"
```

---

### Task 5: `proxy_inner` failover loop

**Files:**
- Modify: `src/routes/proxy.rs` (`proxy_inner` body, imports)
- Test: manual (Step 6); existing unit tests must keep passing.

**Interfaces:**
- Consumes: Task 1 `is_quota_error`, Task 2 cooldown methods, Task 4 `routing_candidates`; existing `should_forward_stream`, `logs::capture_usage_stream`, `insert_upstream_auth`, `request_meta`, `affinity_key`.
- Produces: the full failover loop behavior in `proxy_inner`. Task 6 replicates the same loop shape in `messages.rs`.

- [ ] **Step 1: Add `Bytes` import**

In `src/routes/proxy.rs`, change `use axum::body::Body;` to:

```rust
use axum::body::{Body, Bytes};
```

- [ ] **Step 2: Rewrite `proxy_inner`**

Replace the body of `proxy_inner` from the `let row = match ...` block (currently after `affinity_key`) through the final `Ok(...)` (currently `src/routes/proxy.rs:86-254`) with the loop below. Keep the auth preamble (lines 63-75) and the body-buffer/request-meta/affinity lines (76-84) exactly as they are. The new body:

```rust
    let candidates = match routing_candidates(&st, model.as_deref(), &affinity).await {
        Ok(rows) => rows,
        Err(e) => {
            logs::record_failure(&st, &client, None, &method_str, path, 0, &e);
            return Err(e);
        }
    };

    let base = crate::models::OPENCODE_BASE_URL;
    let path = path.trim_matches('/');
    let url = if path.is_empty() {
        base.to_string()
    } else {
        format!("{base}/{path}")
    };

    // Forward only headers that make sense upstream. Never forward the client's
    // own Authorization — the upstream gets ours instead.
    let mut fwd = HeaderMap::new();
    for h in [
        "content-type",
        "accept",
        "openai-organization",
        "openai-project",
        "anthropic-version",
        "anthropic-beta",
    ] {
        if let Some(v) = headers.get(h) {
            fwd.insert(h, v.clone());
        }
    }

    // Retry across the candidate pool on quota exhaustion. `candidates[0]` is
    // the sticky winner; each failed attempt marks its account in cooldown and
    // the loop moves to the next account. Non-quota failures abort immediately.
    let mut last_error: Option<(StatusCode, HeaderMap, Bytes)> = None;
    for row in &candidates {
        let api_key = match st.decrypt_secret(&row.api_key_enc).await {
            Ok(api_key) => api_key,
            Err(error) => {
                logs::record_failure(
                    &st,
                    &client,
                    Some(row),
                    &method_str,
                    path,
                    started.elapsed().as_millis() as i64,
                    &error,
                );
                return Err(error);
            }
        };
        let upstream_client = match st.client_for_key(row).await {
            Ok(upstream_client) => upstream_client,
            Err(error) => {
                logs::record_failure(
                    &st,
                    &client,
                    Some(row),
                    &method_str,
                    path,
                    started.elapsed().as_millis() as i64,
                    &error,
                );
                return Err(error);
            }
        };
        let mut req_headers = fwd.clone();
        insert_upstream_auth(&mut req_headers, api_key.as_str(), path)?;
        let mut req = upstream_client.request(method.clone(), &url).headers(req_headers);
        if !body_bytes.is_empty() {
            req = req.body(body_bytes.clone());
        }
        let resp = match req.send().await {
            Ok(resp) => resp,
            Err(e) => {
                let err = ApiError::Upstream(format!("upstream unreachable: {e}"));
                logs::record_failure(
                    &st,
                    &client,
                    Some(row),
                    &method_str,
                    path,
                    started.elapsed().as_millis() as i64,
                    &err,
                );
                return Err(err);
            }
        };

        let status = resp.status();
        let latency_ms = started.elapsed().as_millis() as i64;
        let mut resp_headers = resp.headers().clone();
        // Strip hop-by-hop headers: we re-chunk the stream ourselves.
        for h in [
            "content-length",
            "transfer-encoding",
            "connection",
            "accept-encoding",
        ] {
            resp_headers.remove(h);
        }

        if status.is_success() {
            if should_forward_stream(stream, status) {
                // Streaming: forward each SSE frame as it arrives. Record the call
                // with TTFB latency now; token usage is backfilled when the stream
                // ends.
                let log_id = logs::insert_log(
                    &st,
                    &LogInput {
                        client: &client,
                        route: Some(row),
                        method: &method_str,
                        path,
                        model: model.as_deref(),
                        stream: true,
                        status: status.as_u16(),
                        latency_ms,
                        prompt_tokens: None,
                        completion_tokens: None,
                        cached_tokens: None,
                        cache_creation_tokens: None,
                        error: None,
                    },
                )
                .unwrap_or_default();
                let body_stream = resp.bytes_stream();
                let resp_body =
                    Body::from_stream(logs::capture_usage_stream(body_stream, st, log_id, started));
                return Ok((status, resp_headers, resp_body).into_response());
            }
            // Non-stream: buffer the full response so we can read token usage.
            let bytes = match resp.bytes().await {
                Ok(bytes) => bytes,
                Err(e) => {
                    let err = ApiError::Upstream(format!("upstream response read failed: {e}"));
                    logs::record_failure(
                        &st,
                        &client,
                        Some(row),
                        &method_str,
                        path,
                        latency_ms,
                        &err,
                    );
                    return Err(err);
                }
            };
            let usage = logs::usage_from_bytes(&bytes);
            let _ = logs::insert_log(
                &st,
                &LogInput {
                    client: &client,
                    route: Some(row),
                    method: &method_str,
                    path,
                    model: model.as_deref(),
                    stream: false,
                    status: status.as_u16(),
                    latency_ms,
                    prompt_tokens: usage.and_then(|u| u.prompt),
                    completion_tokens: usage.and_then(|u| u.completion),
                    cached_tokens: usage.and_then(|u| u.cached),
                    cache_creation_tokens: usage.and_then(|u| u.cache_creation),
                    error: None,
                },
            );
            return Ok((status, resp_headers, Body::from(bytes)).into_response());
        }

        // Non-success. Quota errors trigger failover; everything else is
        // forwarded verbatim. Error bodies are fully buffered even when the
        // client requested SSE, so the message is captured in the request log.
        let bytes = match resp.bytes().await {
            Ok(bytes) => bytes,
            Err(e) => {
                let err = ApiError::Upstream(format!("upstream response read failed: {e}"));
                logs::record_failure(
                    &st,
                    &client,
                    Some(row),
                    &method_str,
                    path,
                    latency_ms,
                    &err,
                );
                return Err(err);
            }
        };
        if is_quota_error(status, &bytes) {
            st.begin_cooldown(&row.id);
            logs::record_failure(
                &st,
                &client,
                Some(row),
                &method_str,
                path,
                latency_ms,
                &ApiError::Upstream("额度耗尽，切换至备用账号".into()),
            );
            last_error = Some((status, resp_headers, bytes));
            continue;
        }
        let error = if status.is_client_error() || status.is_server_error() {
            logs::error_from_bytes(&bytes)
        } else {
            None
        };
        let _ = logs::insert_log(
            &st,
            &LogInput {
                client: &client,
                route: Some(row),
                method: &method_str,
                path,
                model: model.as_deref(),
                stream: false,
                status: status.as_u16(),
                latency_ms,
                prompt_tokens: None,
                completion_tokens: None,
                cached_tokens: None,
                cache_creation_tokens: None,
                error,
            },
        );
        return Ok((status, resp_headers, Body::from(bytes)).into_response());
    }

    // Every eligible account reported quota exhaustion.
    logs::record_failure(
        &st,
        &client,
        candidates.last(),
        &method_str,
        path,
        started.elapsed().as_millis() as i64,
        &ApiError::Upstream("全部候选账号额度耗尽".into()),
    );
    match last_error {
        Some((status, resp_headers, bytes)) => {
            Ok((status, resp_headers, Body::from(bytes)).into_response())
        }
        None => Err(ApiError::BadRequest("all candidate accounts exhausted".into())),
    }
```

Notes:
- `method.clone()` — `Method` is `Clone`; `method` is consumed per attempt, so clone each iteration.
- `row` is `&KeyRow` (from `for row in &candidates`); pass `Some(row)` / `client_for_key(row)` — no extra `&`.
- The final `logs::record_failure` uses `candidates.last()` so the exhaustion log is attributed to the last account tried.

- [ ] **Step 3: Build and run the full suite**

Run: `cargo build` and `cargo test`
Expected: PASS; no warnings. Existing `messages.rs` native-path and proxy unit tests must all still pass.

- [ ] **Step 4: Commit**

```bash
git add src/routes/proxy.rs
git commit -m "feat(proxy): retry across account pool on quota exhaustion"
```

- [ ] **Step 5: Manual smoke test (optional here, definitive in Task 6)**

Start `cargo run`, create two accounts, and call `/v1/chat/completions` twice with the same `X-Session-Id`: the second request must hit the same account (sticky). (Full failover reproduction needs an account that actually returns a quota error, which requires a real exhausted account — covered in Task 6's manual step.)

---

### Task 6: `messages.rs` adapter-path failover loop

**Files:**
- Modify: `src/routes/messages.rs` (`messages_inner` adapter branch)
- Test: existing unit tests (`to_openai_request`, `to_anthropic_response`) must keep passing; manual smoke test.

**Interfaces:**
- Consumes: `super::proxy::routing_candidates`, `super::proxy::is_quota_error`, `AppState::begin_cooldown`, existing `to_openai_request` / `to_anthropic_response` / `stream_response`.
- Produces: full failover behavior for Messages→Chat-Completions converted requests (the native-Messages path already flows through `proxy_inner` from Task 5, so it needs no change here).

- [ ] **Step 1: Rewrite the adapter branch**

In `messages_inner`, replace everything from `let affinity = ...` through the non-stream success return (currently `src/routes/messages.rs:65-204`) with:

```rust
    let affinity = super::proxy::affinity_key(headers, &client_key.id, Some(&model));
    let candidates =
        match super::proxy::routing_candidates(&st, Some(&model), &affinity).await {
            Ok(rows) => rows,
            Err(e) => {
                logs::record_failure(&st, &client_key, None, "POST", "/messages", 0, &e);
                return Err(e);
            }
        };
    let stream = input
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let url = format!("{}/chat/completions", crate::models::OPENCODE_BASE_URL);

    let mut last_error: Option<(StatusCode, String)> = None;
    for row in &candidates {
        let upstream_key = match st.decrypt_secret(&row.api_key_enc).await {
            Ok(upstream_key) => upstream_key,
            Err(error) => {
                logs::record_failure(
                    &st,
                    &client_key,
                    Some(row),
                    "POST",
                    "/messages",
                    started.elapsed().as_millis() as i64,
                    &error,
                );
                return Err(error);
            }
        };
        // Rebuild the converted request per attempt: to_openai_request is a
        // pure function, so retrying with a different account is cheap.
        let request = to_openai_request(&input)?;
        let upstream_client = match st.client_for_key(row).await {
            Ok(upstream_client) => upstream_client,
            Err(error) => {
                logs::record_failure(
                    &st,
                    &client_key,
                    Some(row),
                    "POST",
                    "/messages",
                    started.elapsed().as_millis() as i64,
                    &error,
                );
                return Err(error);
            }
        };
        let upstream = match upstream_client
            .post(&url)
            .bearer_auth(upstream_key.as_str())
            .json(&request)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                let err = ApiError::Upstream(format!("upstream unreachable: {e}"));
                logs::record_failure(
                    &st,
                    &client_key,
                    Some(row),
                    "POST",
                    "/messages",
                    started.elapsed().as_millis() as i64,
                    &err,
                );
                return Err(err);
            }
        };

        let status = upstream.status();
        let latency_ms = started.elapsed().as_millis() as i64;

        if !status.is_success() {
            let body = upstream.text().await.unwrap_or_default();
            if super::proxy::is_quota_error(status, body.as_bytes()) {
                st.begin_cooldown(&row.id);
                logs::record_failure(
                    &st,
                    &client_key,
                    Some(row),
                    "POST",
                    "/messages",
                    latency_ms,
                    &ApiError::Upstream("额度耗尽，切换至备用账号".into()),
                );
                last_error = Some((status, body));
                continue;
            }
            let error =
                logs::error_from_bytes(body.as_bytes()).unwrap_or_else(|| status.to_string());
            let _ = logs::insert_log(
                &st,
                &LogInput {
                    client: &client_key,
                    route: Some(row),
                    method: "POST",
                    path: "/messages",
                    model: Some(&model),
                    stream,
                    status: status.as_u16(),
                    latency_ms,
                    prompt_tokens: None,
                    completion_tokens: None,
                    cached_tokens: None,
                    cache_creation_tokens: None,
                    error: Some(error),
                },
            );
            return Ok((status, body).into_response());
        }
        if stream {
            let log_id = logs::insert_log(
                &st,
                &LogInput {
                    client: &client_key,
                    route: Some(row),
                    method: "POST",
                    path: "/messages",
                    model: Some(&model),
                    stream: true,
                    status: 200,
                    latency_ms,
                    prompt_tokens: None,
                    completion_tokens: None,
                    cached_tokens: None,
                    cache_creation_tokens: None,
                    error: None,
                },
            )
            .unwrap_or_default();
            return Ok(stream_response(upstream, model, st, log_id, started));
        }
        let value: Value = upstream
            .json()
            .await
            .map_err(|e| ApiError::Upstream(format!("invalid upstream response: {e}")))?;
        let usage = logs::usage_from_json(&value);
        let _ = logs::insert_log(
            &st,
            &LogInput {
                client: &client_key,
                route: Some(row),
                method: "POST",
                path: "/messages",
                model: Some(&model),
                stream: false,
                status: 200,
                latency_ms,
                prompt_tokens: usage.and_then(|u| u.prompt),
                completion_tokens: usage.and_then(|u| u.completion),
                cached_tokens: usage.and_then(|u| u.cached),
                cache_creation_tokens: usage.and_then(|u| u.cache_creation),
                error: None,
            },
        );
        return Ok(Json(to_anthropic_response(value)?).into_response());
    }

    logs::record_failure(
        &st,
        &client_key,
        candidates.last(),
        "POST",
        "/messages",
        started.elapsed().as_millis() as i64,
        &ApiError::Upstream("全部候选账号额度耗尽".into()),
    );
    match last_error {
        Some((status, body)) => Ok((status, body).into_response()),
        None => Err(ApiError::BadRequest("all candidate accounts exhausted".into())),
    }
```

`StatusCode` is already imported in `messages.rs` (`use axum::http::{HeaderMap, Method, StatusCode};`).

- [ ] **Step 2: Build and run the full suite**

Run: `cargo build` and `cargo test`
Expected: PASS; no warnings. The two existing `messages.rs` unit tests (`converts_anthropic_request_to_openai`, `converts_openai_response_to_anthropic`) must pass unchanged.

- [ ] **Step 3: Manual smoke test**

With `cargo run`:
1. Configure two accounts that both advertise the requested model (via connectivity test).
2. `POST /v1/messages` with `X-Session-Id: smoke-1` and an Anthropic-style body (`model`, `messages`) against an exhausted account — observe the log rows: one failure `额度耗尽，切换至备用账号` for the first account, then a 200 row for the second account, and the response is a valid Anthropic `message` object.
3. Repeat immediately with the same session: the exhausted account is in cooldown, so only the healthy account is hit (single 200 row).
4. Wait 15+ minutes (or temporarily lower `QUOTA_COOLDOWN_SECS` to 5 in a scratch build) and confirm the exhausted account re-enters rotation.

- [ ] **Step 4: Commit**

```bash
git add src/routes/messages.rs
git commit -m "feat(messages): failover across account pool for converted requests"
```

---

### Task 7: README cleanup

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Remove the override-option sentence**

In `README.md`, change line 60:

```markdown
> `X-Key-Id` / `X-Key-Name` 仅作为需要固定账号时的显式覆盖。SDK 用法示例：
```

to:

```markdown
> 额度耗尽的账号会自动切换到候选池中的备用账号（15 分钟冷却后恢复）。SDK 用法示例：
```

- [ ] **Step 2: Verify and commit**

Run: `cargo test` (README-only change; confirms nothing else broke).
```bash
git add README.md
git commit -m "docs: document automatic quota-exhaustion failover"
```

---

## Self-Review

- **Spec coverage:** classifier (§1 → Task 1), cooldown registry (§2 → Task 2), ordered candidates (§3 → Task 3), routing_candidates + override removal (§3 → Task 4), proxy_inner loop (§4 → Task 5), messages adapter loop + native path via proxy_inner (§5 → Task 6), logging (§6 → embedded in Task 5/6), tests (§7 → Task 1/2/3/4), README (Task 7). Empty-pool distinction (§4 step 3) is in `routing_candidates` (Task 4).
- **Placeholder scan:** no TBD/TODO; every code step carries full code. Manual smoke tests (Task 5 step 5, Task 6 step 3) are explicitly marked manual because the repo has no HTTP-mock test infra (checked `Cargo.toml` — no wiremock/mockito).
- **Type consistency:** `is_quota_error(StatusCode, &[u8]) -> bool`, `QUOTA_COOLDOWN_SECS: i64`, `begin_cooldown(&str)`, `in_quota_cooldown(&str, i64) -> bool`, `rendezvous_hash(&str,&str)->u64`, `ordered_candidates(&[KeyRow], &str, F) -> Vec<&KeyRow>`, `routing_candidates(&AppState, Option<&str>, &str) -> Result<Vec<KeyRow>, ApiError>` — each defined once and used with identical signatures downstream. `row` is `&KeyRow` in Task 5/6 loops, so call sites use `Some(row)`/`client_for_key(row)` consistently.
