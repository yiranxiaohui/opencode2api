export interface Status {
  installed: boolean
  unlocked: boolean
  key_count: number
}

export interface ModelInfo {
  id: string
  owned_by: string
}

export interface KeySummary {
  id: string
  name: string
  base_url: string
  tags: string[]
  notes: string
  is_default: boolean
  model_count: number
  created_at: number
  updated_at: number
  proxy_id: string | null
  proxy_name: string | null
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
  is_default: boolean
  proxy_id?: string | null
}

export interface TestResult {
  ok: boolean
  latency_ms: number | null
  models: ModelInfo[]
  error: string | null
}

export interface ExportItem {
  name: string
  base_url?: string
  api_key: string
  tags: string[]
  notes: string
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
  total_duration_ms: number
}

export interface LogStatsGroup {
  name: string
  calls: number
  prompt_tokens: number
  completion_tokens: number
}

export interface LogStatsResponse {
  totals: LogStatsTotals
  by_model: LogStatsGroup[]
  by_client: LogStatsGroup[]
}
