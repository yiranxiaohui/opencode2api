import { useEffect, useState } from 'react'
import { keysApi } from '../api/keys'
import type { KeySummary, TestResult } from '../api/types'
import { toast } from '../lib/toast'
import CopyButton from './CopyButton'
import { BoltIcon, CheckIcon, EditIcon, EyeIcon, EyeOffIcon, RefreshIcon, StarIcon, TrashIcon, XIcon } from './icons'

interface Props {
  summary: KeySummary
  onClose: () => void
  onEdit: (s: KeySummary) => void
  onDelete: (s: KeySummary) => void
  onSetDefault: (id: string) => void
}

export default function KeyDetail({ summary, onClose, onEdit, onDelete, onSetDefault }: Props) {
  const [record, setRecord] = useState<(typeof summary) & { api_key?: string; model_cache?: unknown[] }>(summary)
  const [revealed, setRevealed] = useState(false)
  const [testing, setTesting] = useState(false)
  const [test, setTest] = useState<TestResult | null>(null)
  const [loadErr, setLoadErr] = useState('')

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

  return (
    <div className="overlay" onClick={onClose}>
      <div className="drawer" onClick={(e) => e.stopPropagation()}>
        <div className="drawer-head">
          <h2>{summary.name}</h2>
          {summary.is_default && <span className="default-badge">默认账号</span>}
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
            {!summary.is_default && (
              <button className="btn" onClick={() => onSetDefault(summary.id)}>
                <StarIcon size={13} /> 设为默认
              </button>
            )}
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
