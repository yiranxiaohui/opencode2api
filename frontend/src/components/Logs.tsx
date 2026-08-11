import { useEffect, useMemo, useState } from 'react'
import { logsApi } from '../api/keys'
import type {
  ClientApiKey,
  KeySummary,
  LogQuery,
  LogStatsGroup,
  LogStatsResponse,
  LogStatsTotals,
  RequestLog,
} from '../api/types'
import { toast } from '../lib/toast'
import { ActivityIcon, RefreshIcon, TrashIcon, XIcon } from './icons'

interface Props {
  keys: KeySummary[]
  clientKeys: ClientApiKey[]
}

const STATUS_CHOICES = [200, 400, 401, 404, 429, 500, 502, 503, 504]

export default function Logs({ keys, clientKeys }: Props) {
  const [client, setClient] = useState('')
  const [keyId, setKeyId] = useState('')
  const [model, setModel] = useState('')
  const [status, setStatus] = useState('')
  const [rows, setRows] = useState<RequestLog[]>([])
  const [total, setTotal] = useState(0)
  const [stats, setStats] = useState<LogStatsResponse | null>(null)
  const [loading, setLoading] = useState(false)
  const [err, setErr] = useState('')
  const [paused, setPaused] = useState(false)

  const filters: LogQuery = useMemo(() => {
    const q: LogQuery = {}
    if (client) q.client = client
    if (keyId) q.key = keyId
    if (model.trim()) q.model = model.trim()
    if (status) q.status = Number(status)
    return q
  }, [client, keyId, model, status])

  const load = async (q: LogQuery = filters) => {
    setLoading(true)
    try {
      const res = await logsApi.list({ ...q, limit: 200 })
      setRows(res.items)
      setTotal(res.total)
      setErr('')
    } catch (e) {
      setErr(e instanceof Error ? e.message : '日志加载失败')
      return
    } finally {
      setLoading(false)
    }
    try {
      setStats(await logsApi.stats(q))
    } catch {
      /* stats are best-effort; the table still works without them */
    }
  }

  useEffect(() => {
    load()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, keyId, model, status])

  useEffect(() => {
    if (paused) return
    const t = setInterval(() => load(), 5000)
    return () => clearInterval(t)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [paused, filters])

  const clearAll = async () => {
    if (!window.confirm('清空全部请求日志？此操作不可撤销。')) return
    try {
      await logsApi.clear()
      await load()
      toast('请求日志已清空', 'ok')
    } catch (e) {
      toast(e instanceof Error ? e.message : '清空失败', 'err')
    }
  }

  const activeCount = [client, keyId, model, status].filter(Boolean).length

  return (
    <div>
      <div className="toolbar" style={{ flexWrap: 'wrap' }}>
        <select className="input log-filter" value={client} onChange={(e) => setClient(e.target.value)}>
          <option value="">全部访问密钥</option>
          {clientKeys.map((k) => (
            <option key={k.id} value={k.id}>{k.name}</option>
          ))}
        </select>
        <select className="input log-filter" value={keyId} onChange={(e) => setKeyId(e.target.value)}>
          <option value="">全部账号</option>
          {keys.map((k) => (
            <option key={k.id} value={k.id}>{k.name}</option>
          ))}
        </select>
        <input
          className="input log-filter"
          placeholder="模型关键字（deepseek…）"
          value={model}
          onChange={(e) => setModel(e.target.value)}
        />
        <select className="input log-filter log-status" value={status} onChange={(e) => setStatus(e.target.value)}>
          <option value="">全部状态</option>
          {STATUS_CHOICES.map((s) => (
            <option key={s} value={s}>{s}</option>
          ))}
        </select>

        <div className="grow" />

        {activeCount > 0 && (
          <button className="btn btn-ghost btn-sm" onClick={() => { setClient(''); setKeyId(''); setModel(''); setStatus('') }}>
            <XIcon size={13} /> 清除筛选{activeCount > 0 ? ` (${activeCount})` : ''}
          </button>
        )}
        <button className={`btn btn-sm ${paused ? 'btn-ghost' : ''}`} onClick={() => setPaused((p) => !p)}>
          <RefreshIcon size={13} /> {paused ? '已暂停 · 点击恢复' : '自动刷新'}
        </button>
        <button className="btn btn-sm" onClick={() => load()}>
          <RefreshIcon size={13} /> 刷新
        </button>
        <button className="btn btn-danger btn-sm" onClick={clearAll}>
          <TrashIcon size={13} /> 清空
        </button>
      </div>

      {stats && (
        <div className="log-stats">
          <div className="log-stat-tiles">
            <StatTile label="调用次数" value={stats.totals.total_calls.toLocaleString()} />
            <StatTile label="Prompt Token" value={fmtTokensNum(stats.totals.total_prompt_tokens)} />
            <StatTile label="Completion Token" value={fmtTokensNum(stats.totals.total_completion_tokens)} />
            <StatTile label="平均耗时" value={avgMs(stats.totals)} />
          </div>
          {(stats.by_model.length > 0 || stats.by_client.length > 0) && (
            <div className="log-stat-cols">
              <GroupBarList title="按模型用量" groups={stats.by_model} />
              <GroupBarList title="按访问密钥用量" groups={stats.by_client} />
            </div>
          )}
        </div>
      )}

      <div className="log-head small">
        <span>{loading ? '加载中…' : `共 ${total} 条记录`}</span>
        <span style={{ color: 'var(--faint)' }}>每 5 秒自动刷新 · 仅记录元数据，不含请求/响应内容</span>
      </div>

      {err && <p className="auth-err">{err}</p>}

      {rows.length === 0 ? (
        <div className="empty">
          <div className="big"><ActivityIcon size={40} /></div>
          <p>{activeCount > 0 ? '没有符合条件的调用记录' : '暂无调用记录'}</p>
          <p className="small">客户端通过 /v1/* 或 /v1/messages 发起的调用会显示在这里。</p>
        </div>
      ) : (
        <div className="log-table">
          <div className="log-row log-row-head">
            <span>时间</span>
            <span>状态</span>
            <span>访问密钥</span>
            <span>账号</span>
            <span>模型</span>
            <span>调用</span>
            <span>首字 / 总耗时</span>
            <span>Token</span>
          </div>
          {rows.map((r) => <LogRow key={r.id} log={r} />)}
        </div>
      )}
    </div>
  )
}

function LogRow({ log }: { log: RequestLog }) {
  const [open, setOpen] = useState(false)
  const isErr = log.status >= 400
  const hasDetail = log.error !== null

  return (
    <>
      <div className="log-row" role="button" tabIndex={0} onClick={() => hasDetail && setOpen((o) => !o)}>
        <span className="log-time">{formatTime(log.created_at)}</span>
        <span>
          <span className={`log-status-pill ${isErr ? 'err' : 'ok'}`}>{log.status}</span>
          {log.stream && <span className="log-stream" title="SSE 流式">流</span>}
        </span>
        <span className="log-cell log-name">{log.client_key_name}</span>
        <span className="log-cell log-name">{log.route_key_name ?? '—'}</span>
        <span className="log-cell log-model">{log.model ?? '—'}</span>
        <span className="log-cell log-path"><span className="log-method">{log.method}</span>{log.path}</span>
        <span className="log-cell log-latency">
          {log.stream
            ? `${log.first_token_ms === null ? '—' : `${log.first_token_ms}ms`} / ${log.latency_ms}ms`
            : `${log.latency_ms}ms`}
        </span>
        <span className="log-cell log-tokens">{fmtTokens(log)}</span>
      </div>
      {open && hasDetail && (
        <div className="log-detail">
          <div className="log-detail-title">错误信息</div>
          <code>{log.error}</code>
        </div>
      )}
    </>
  )
}

function fmtTokens(log: RequestLog) {
  if (log.prompt_tokens === null && log.completion_tokens === null) return '—'
  const p = log.prompt_tokens ?? 0
  const c = log.completion_tokens ?? 0
  return `${p}/${c}`
}

function formatTime(seconds: number) {
  return new Date(seconds * 1000).toLocaleString()
}

function StatTile({ label, value }: { label: string; value: string }) {
  return (
    <div className="log-stat-tile">
      <span className="log-stat-value">{value}</span>
      <span className="log-stat-label">{label}</span>
    </div>
  )
}

function fmtTokensNum(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`
  return String(n)
}

function avgMs(t: LogStatsTotals) {
  return t.total_calls > 0 ? `${Math.round(t.total_duration_ms / t.total_calls)}ms` : '—'
}

function GroupBarList({ title, groups }: { title: string; groups: LogStatsGroup[] }) {
  if (groups.length === 0) return null
  const max = Math.max(...groups.map((g) => g.prompt_tokens + g.completion_tokens))
  return (
    <div className="log-stat-col">
      <div className="log-stat-col-title">{title}</div>
      {groups.map((g) => {
        const total = g.prompt_tokens + g.completion_tokens
        const pct = max > 0 ? (total / max) * 100 : 0
        return (
          <div className="log-stat-row" key={g.name}>
            <div className="log-stat-row-top">
              <span className="log-stat-name" title={g.name}>{g.name || '（未知）'}</span>
              <span className="log-stat-nums">{fmtTokensNum(total)} · {g.calls} 次</span>
            </div>
            <div className="log-stat-bar">
              <div className="log-stat-bar-fill" style={{ width: `${pct}%` }} />
            </div>
          </div>
        )
      })}
    </div>
  )
}
