import { del, get, post, put } from './client'
import type {
  ClientApiKey,
  ClientApiKeyCreated,
  CookieImportInput,
  AccountUsage,
  ExportPayload,
  ImportResult,
  KeyInput,
  KeyRecord,
  KeySummary,
  LogListResponse,
  LogQuery,
  LogStatsResponse,
  ProxyInput,
  ProxyRecord,
  TestResult,
} from './types'

export const auth = {
  setup: (password: string) => post<{ ok: boolean }>('/api/auth/setup', { password }),
  unlock: (password: string) => post<{ ok: boolean }>('/api/auth/unlock', { password }),
  changePassword: (old_password: string, new_password: string) =>
    post<{ ok: boolean }>('/api/auth/change-password', { old_password, new_password }),
}

export const keysApi = {
  list: () => get<KeySummary[]>('/api/keys'),
  get: (id: string) => get<KeyRecord>(`/api/keys/${id}`),
  create: (input: KeyInput) => post<KeyRecord>('/api/keys', input),
  update: (id: string, input: KeyInput) => put<KeyRecord>(`/api/keys/${id}`, input),
  remove: (id: string) => del<{ ok: boolean }>(`/api/keys/${id}`),
  test: (id: string) => post<TestResult>(`/api/keys/${id}/test`),
  setDefault: (id: string) => post<{ ok: boolean }>(`/api/keys/${id}/set-default`),
  setEnabled: (id: string, enabled: boolean) =>
    post<{ ok: boolean }>(`/api/keys/${id}/set-enabled`, { enabled }),
  exportAll: () => get<ExportPayload>('/api/export'),
  import: (payload: unknown) => post<ImportResult>('/api/import', payload),
  importCookie: (input: CookieImportInput) => post<KeyRecord>('/api/keys/import-cookie', input),
  usage: (id: string) => get<AccountUsage>(`/api/keys/${id}/usage`),
}

export const proxiesApi = {
  list: () => get<ProxyRecord[]>('/api/proxies'),
  create: (input: ProxyInput) => post<ProxyRecord>('/api/proxies', input),
  update: (id: string, input: ProxyInput) => put<ProxyRecord>(`/api/proxies/${id}`, input),
  remove: (id: string) => del<{ ok: boolean }>(`/api/proxies/${id}`),
}

export const clientKeysApi = {
  list: () => get<ClientApiKey[]>('/api/client-keys'),
  create: (name: string) => post<ClientApiKeyCreated>('/api/client-keys', { name }),
  remove: (id: string) => del<{ ok: boolean }>(`/api/client-keys/${id}`),
}

function logsQuery(params: LogQuery): string {
  const qs = new URLSearchParams()
  if (params.client) qs.set('client', params.client)
  if (params.key) qs.set('key', params.key)
  if (params.model) qs.set('model', params.model)
  if (params.status !== undefined) qs.set('status', String(params.status))
  if (params.limit !== undefined) qs.set('limit', String(params.limit))
  if (params.offset !== undefined) qs.set('offset', String(params.offset))
  const q = qs.toString()
  return q ? `?${q}` : ''
}

export const logsApi = {
  list: (params: LogQuery = {}) =>
    get<LogListResponse>(`/api/logs${logsQuery(params)}`),
  stats: (params: LogQuery = {}) =>
    get<LogStatsResponse>(`/api/logs/stats${logsQuery(params)}`),
  clear: () => del<{ ok: boolean }>('/api/logs'),
}
