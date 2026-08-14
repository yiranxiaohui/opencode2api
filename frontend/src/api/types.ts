export interface Status {
  installed: boolean
  logged_in: boolean
  key_count: number
}

export interface ModelInfo {
  id: string
  owned_by: string
}

export interface ManagedModel extends ModelInfo {
  account_count: number
  enabled: boolean
}

export interface KeySummary {
  id: string
  name: string
  base_url: string
  tags: string[]
  notes: string
  is_enabled: boolean
  account_type: 'normal' | 'go'
  cooldown_until: number | null
  model_count: number
  created_at: number
  updated_at: number
  proxy_id: string | null
  proxy_name: string | null
  has_cookie: boolean
  workspace_id: string | null
  usage_cache: AccountUsage | null
}

export interface KeyRecord extends KeySummary {
  api_key: string
  model_cache: ModelInfo[]
}

export interface KeyInput {
  name: string
  base_url?: string
  api_key?: string
  tags: string[]
  notes: string
  account_type: 'normal' | 'go'
  proxy_id?: string | null
}

export interface TestResult {
  ok: boolean
  latency_ms: number | null
  models: ModelInfo[]
  error: string | null
}

export interface CookieImportInput { cookie: string; name?: string; proxy_id?: string | null; account_type: 'normal' | 'go' }
export interface UsageWindow { usage_percent: number; remaining_percent: number; reset_in_sec: number; status: string }
export interface AccountUsage {
  plan_name: string; plan_status: string; region: string | null
  balance_microcents: number | null; monthly_limit_microcents: number | null; monthly_usage_microcents: number | null
  rolling: UsageWindow | null; weekly: UsageWindow | null; monthly: UsageWindow | null; fetched_at: number
}

export interface InviteLinkResult {
  account_id: string
  account_name: string
  invite_link: string
}

export interface ExportItem {
  name: string
  base_url?: string
  api_key: string
  tags: string[]
  notes: string
  is_enabled?: boolean
  account_type?: 'normal' | 'go'
  proxy?: string | null
}

export interface ProxyExport {
  name: string
  url: string
}

export interface ExportPayload {
  proxies: ProxyExport[]
  items: ExportItem[]
}

export interface ProxyRecord {
  id: string
  name: string
  url: string
  created_at: number
  updated_at: number
}

export interface ProxyInput {
  name: string
  url: string
}

export interface ImportResult {
  imported: number
  updated: number
}

export interface ClientApiKey {
  id: string
  name: string
  prefix: string
  created_at: number
  last_used_at: number | null
  api_key: string | null
}

export interface ClientApiKeyCreated extends ClientApiKey {
  api_key: string
}

export interface RequestLog {
  id: string
  created_at: number
  client_key_id: string | null
  client_key_name: string
  route_key_id: string | null
  route_key_name: string | null
  method: string
  path: string
  model: string | null
  stream: boolean
  status: number
  latency_ms: number
  first_token_ms: number | null
  prompt_tokens: number | null
  completion_tokens: number | null
  cached_tokens: number | null
  cache_creation_tokens: number | null
  error: string | null
}

export interface LogListResponse {
  items: RequestLog[]
  total: number
}

export interface LogQuery {
  client?: string
  key?: string
  model?: string
  status?: number
  limit?: number
  offset?: number
}

export interface LogStatsTotals {
  total_calls: number
  total_prompt_tokens: number
  total_completion_tokens: number
  total_cached_tokens: number
  total_cache_creation_tokens: number
  total_duration_ms: number
}

export interface LogStatsGroup {
  name: string
  calls: number
  prompt_tokens: number
  completion_tokens: number
  cached_tokens: number
  cache_creation_tokens: number
}

export interface LogStatsResponse {
  totals: LogStatsTotals
  by_model: LogStatsGroup[]
  by_client: LogStatsGroup[]
}
