import { useEffect, useState } from 'react'
import { keysApi } from '../api/keys'
import type { AccountUsage, KeySummary, TestResult } from '../api/types'
import { toast } from '../lib/toast'
import CopyButton from './CopyButton'
import { BoltIcon, CheckIcon, EditIcon, EyeIcon, EyeOffIcon, PowerIcon, RefreshIcon, StarIcon, TrashIcon, XIcon } from './icons'

interface Props {
  summary: KeySummary
  onClose: () => void
  onEdit: (s: KeySummary) => void
  onDelete: (s: KeySummary) => void
  onSetDefault: (id: string) => void
  onSetEnabled: (id: string, enabled: boolean) => void
}

export default function KeyDetail({ summary, onClose, onEdit, onDelete, onSetDefault, onSetEnabled }: Props) {
  const [record, setRecord] = useState<(typeof summary) & { api_key?: string; model_cache?: unknown[] }>(summary)
  const [revealed, setRevealed] = useState(false)
  const [testing, setTesting] = useState(false)
  const [test, setTest] = useState<TestResult | null>(null)
  const [loadErr, setLoadErr] = useState('')
  const [usage, setUsage] = useState<AccountUsage | null>(null)
  const [usageLoading, setUsageLoading] = useState(false)

  useEffect(() => {
    let alive = true
    keysApi
      .get(summary.id)
      .then((r) => {
        if (alive) setRecord(r)
      })
      .catch((e) => alive && setLoadErr(e instanceof Error ? e.message : '加载失败'))
    return () => {
      alive = false
    }
  }, [summary.id])

  const runTest = async () => {
    setTesting(true)
    setTest(null)
    try {
      const r = await keysApi.test(summary.id)
      setTest(r)
      if (r.ok) toast(`连通正常 · ${r.latency_ms}ms`, 'ok')
      else toast(r.error || '连通失败', 'err')
    } catch (e) {
      setTest({ ok: false, latency_ms: null, models: [], error: e instanceof Error ? e.message : '请求失败' })
    } finally {
      setTesting(false)
    }
  }

  const maskKey = (k: string) => {
    if (k.length <= 8) return k.slice(0, 2) + '••••'
    return `${k.slice(0, 4)}••••••••${k.slice(-4)}`
  }

  const models = (record.model_cache as { id: string; owned_by?: string }[] | undefined) ?? []
  const lastTestModels = test?.ok ? test.models : null
  const shownModels = lastTestModels ?? models
  const loadUsage = async () => { setUsageLoading(true); try { setUsage(await keysApi.usage(summary.id)) } catch (e) { toast(e instanceof Error ? e.message : '额度查询失败', 'err') } finally { setUsageLoading(false) } }
  const money = (value: number | null) => value == null ? '—' : `$${(value / 100_000_000).toFixed(2)}`

  return (
    <div className="overlay" onClick={onClose}>
      <div className="drawer" onClick={(e) => e.stopPropagation()}>
        <div className="drawer-head">
          <h2>{summary.name}</h2>
          {summary.is_default && <span className="default-badge">默认账号</span>}
          {!summary.is_enabled && <span className="disabled-badge">已禁用</span>}
          <button className="btn btn-ghost btn-sm" onClick={onClose} aria-label="关闭">
            <XIcon size={16} />
          </button>
        </div>

        <div className="drawer-body">
          {loadErr && <p className="auth-err">{loadErr}</p>}

          <dl className="kv">
            <dt>API Key</dt>
            <dd>
              <div className="secret-line">
                <span className={`val ${revealed ? 'revealed' : ''}`}>
                  {record.api_key ? (revealed ? record.api_key : maskKey(record.api_key)) : '…'}
                </span>
                <button
                  type="button"
                  className="btn btn-ghost btn-sm"
                  title={revealed ? '隐藏' : '显示'}
                  onClick={() => setRevealed((v) => !v)}
                >
                  {revealed ? <EyeOffIcon size={14} /> : <EyeIcon size={14} />}
                </button>
                {record.api_key && <CopyButton text={record.api_key} label="" />}
              </div>
            </dd>

            {record.tags.length > 0 && (
              <>
                <dt>标签</dt>
                <dd>
                  <div className="tags">
                    {record.tags.map((t) => (
                      <span key={t} className="tag">
                        {t}
                      </span>
                    ))}
                  </div>
                </dd>
              </>
            )}

            {record.notes && (
              <>
                <dt>备注</dt>
                <dd className="small">{record.notes}</dd>
              </>
            )}

            <dt>出口代理</dt>
            <dd>
              {record.proxy_name ? (
                <span className="proxy-badge">🌐 {record.proxy_name}</span>
              ) : (
                <span className="small">直连（不使用代理）</span>
              )}
            </dd>
          </dl>

          {summary.has_cookie && <div className="test-box"><div style={{display:'flex', alignItems:'center', justifyContent:'space-between'}}><div className="section-label" style={{margin:0}}>套餐与额度</div><button className="btn btn-sm" disabled={usageLoading} onClick={loadUsage}>{usageLoading ? '查询中…' : usage ? '刷新' : '查询额度'}</button></div>{usage && <><h3 style={{margin:'14px 0 4px'}}>{usage.plan_name}</h3><div className="small">状态 {usage.plan_status}{usage.region ? ` · ${usage.region}` : ''} · 余额 {money(usage.balance_microcents)}</div><div className="usage-grid">{([['滚动额度', usage.rolling], ['每周额度', usage.weekly], ['每月额度', usage.monthly]] as const).map(([label,w]) => w && <div className="usage-card" key={label}><span className="small">{label}</span><strong>{w.remaining_percent.toFixed(1)}%</strong><div className="usage-track"><i style={{width:`${Math.max(0,Math.min(100,w.remaining_percent))}%`}} /></div><span className="small">剩余 · {Math.ceil(w.reset_in_sec/3600)} 小时后重置</span></div>)}</div>{usage.monthly_limit_microcents != null && <div className="small" style={{marginTop:10}}>月度消费 {money(usage.monthly_usage_microcents)} / {money(usage.monthly_limit_microcents)}</div>}</>}</div>}

          <div className="test-box">
            <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
              <button className="btn btn-primary btn-sm" onClick={runTest} disabled={testing}>
                {testing ? <RefreshIcon size={13} className="spin" /> : <BoltIcon size={13} />}
                {testing ? '测试中…' : test ? '重新测试' : '连通性测试'}
              </button>
              {test && (
                <span className={`test-result ${test.ok ? 'ok' : 'err'}`}>
                  {test.ok ? <CheckIcon size={13} /> : null}
                  {test.ok ? '连通正常' : '连接失败'}
                  {test.latency_ms != null && <span className="latency">{test.latency_ms}ms</span>}
                </span>
              )}
            </div>
            {test && !test.ok && test.error && (
              <div className="test-result err" style={{ marginTop: 8 }}>
                {test.error}
              </div>
            )}
            {shownModels.length > 0 && (
              <>
                <div className="model-list">
                  {shownModels.map((m) => (
                    <span key={m.id} className="model-item">
                      {m.id}
                    </span>
                  ))}
                </div>
                <div className="small" style={{ marginTop: 8 }}>
                  共 {shownModels.length} 个模型
                  {test ? '（来自本次测试）' : '（上次测试缓存）'}
                </div>
              </>
            )}
          </div>

          <div className="section-label">操作</div>
          <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
            {summary.is_enabled && !summary.is_default && (
              <button className="btn" onClick={() => onSetDefault(summary.id)}>
                <StarIcon size={13} /> 设为默认
              </button>
            )}
            <button className={summary.is_enabled ? 'btn' : 'btn btn-enable'} onClick={() => onSetEnabled(summary.id, !summary.is_enabled)}>
              <PowerIcon size={13} /> {summary.is_enabled ? '禁用账号' : '启用账号'}
            </button>
            <button className="btn" onClick={() => onEdit(summary)}>
              <EditIcon size={13} /> 编辑
            </button>
            <button className="btn btn-danger" onClick={() => onDelete(summary)}>
              <TrashIcon size={13} /> 删除
            </button>
          </div>
          <div className="divider" />
          <div className="small" style={{ color: 'var(--faint)' }}>
            ID <span className="mono">{summary.id}</span>
          </div>
        </div>
      </div>
    </div>
  )
}
