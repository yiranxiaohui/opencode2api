import { useEffect, useState } from 'react'
import { clientKeysApi, modelsApi } from '../api/keys'
import type { ClientApiKey, ManagedModel } from '../api/types'
import { toast } from '../lib/toast'
import ClientKeyModelsModal from './ClientKeyModelsModal'
import CopyButton from './CopyButton'
import { EyeIcon, EyeOffIcon, KeyIcon, ModelIcon, PlusIcon, TrashIcon } from './icons'

interface Props {
  onKeysChange?: (keys: ClientApiKey[]) => void
}

export default function ClientKeys({ onKeysChange }: Props) {
  const [keys, setKeys] = useState<ClientApiKey[]>([])
  const [name, setName] = useState('')
  const [busy, setBusy] = useState(false)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  const [visible, setVisible] = useState<Set<string>>(new Set())
  const [models, setModels] = useState<ManagedModel[]>([])
  const [modelsLoading, setModelsLoading] = useState(true)
  const [createAllowedModels, setCreateAllowedModels] = useState<string[] | null>(null)
  const [modelEditor, setModelEditor] = useState<{ kind: 'create' } | { kind: 'key', key: ClientApiKey } | null>(null)
  const [modelBusy, setModelBusy] = useState(false)

  const replaceKeys = (next: ClientApiKey[]) => {
    setKeys(next)
    onKeysChange?.(next)
  }

  useEffect(() => {
    let live = true
    clientKeysApi.list()
      .then((items) => {
        if (live) replaceKeys(items)
      })
      .catch((cause) => {
        if (live) setError(cause instanceof Error ? cause.message : '访问密钥加载失败')
      })
      .finally(() => {
        if (live) setLoading(false)
      })

    modelsApi.list()
      .then((items) => {
        if (live) setModels(items)
      })
      .catch((cause) => {
        if (live) toast(cause instanceof Error ? cause.message : '模型加载失败', 'err')
      })
      .finally(() => {
        if (live) setModelsLoading(false)
      })
    return () => { live = false }
    // The callback is intentionally excluded; loading depends only on mounting.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const createKey = async (event: React.FormEvent) => {
    event.preventDefault()
    setError('')
    setBusy(true)
    try {
      const created = await clientKeysApi.create(name.trim(), createAllowedModels)
      replaceKeys([created, ...keys])
      setVisible((current) => new Set(current).add(created.id))
      setName('')
      setCreateAllowedModels(null)
      toast('客户端访问密钥已创建，可随时再次复制', 'ok')
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '创建失败')
    } finally {
      setBusy(false)
    }
  }

  const removeKey = async (key: ClientApiKey) => {
    if (!window.confirm(`撤销访问密钥「${key.name}」？使用它的程序将立即无法调用。`)) return
    try {
      await clientKeysApi.remove(key.id)
      replaceKeys(keys.filter((item) => item.id !== key.id))
      toast('访问密钥已撤销', 'ok')
    } catch (cause) {
      toast(cause instanceof Error ? cause.message : '撤销失败', 'err')
    }
  }

  const toggleVisible = (id: string) => {
    setVisible((current) => {
      const next = new Set(current)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const saveModelScope = async (allowedModels: string[] | null) => {
    if (!modelEditor) return
    if (modelEditor.kind === 'create') {
      setCreateAllowedModels(allowedModels)
      setModelEditor(null)
      return
    }

    setModelBusy(true)
    try {
      await clientKeysApi.updateModels(modelEditor.key.id, allowedModels)
      replaceKeys(keys.map((key) => (
        key.id === modelEditor.key.id ? { ...key, allowed_models: allowedModels } : key
      )))
      setModelEditor(null)
      toast('模型访问范围已更新', 'ok')
    } catch (cause) {
      toast(cause instanceof Error ? cause.message : '模型范围保存失败', 'err')
    } finally {
      setModelBusy(false)
    }
  }

  return (
    <div className="client-keys-page">
      <div className="panel client-key-create-panel">
        <div className="client-key-intro">
          <div className="auth-mark client-key-mark"><KeyIcon size={20} /></div>
          <div>
            <strong>创建客户端访问密钥</strong>
            <p className="small">用于其他程序调用 /v1/* 接口。密钥会加密保存，创建后仍可在下方复制。</p>
          </div>
        </div>
        <form className="client-key-create-form" onSubmit={createKey}>
          <label className="sr-only" htmlFor="client-key-name">密钥名称</label>
          <input
            id="client-key-name"
            className="input"
            placeholder="密钥名称，例如：Cherry Studio"
            maxLength={80}
            value={name}
            onChange={(event) => setName(event.target.value)}
          />
          <button
            className="btn client-key-create-models"
            type="button"
            disabled={modelsLoading}
            onClick={() => setModelEditor({ kind: 'create' })}
          >
            <ModelIcon size={13} /> {modelScopeLabel(createAllowedModels)}
          </button>
          <button className="btn btn-primary" type="submit" disabled={busy || !name.trim()}>
            <PlusIcon size={13} /> {busy ? '创建中…' : '创建密钥'}
          </button>
        </form>
        {error && <p className="auth-err client-key-error" role="alert">{error}</p>}
      </div>

      <div className="client-key-list-head">
        <div>
          <strong>访问密钥</strong>
          <span className="count">{keys.length} 条</span>
        </div>
        <span className="small">完整密钥仅在已登录的管理页面返回</span>
      </div>

      <div className="panel client-key-table">
        <div className="client-key-table-row client-key-table-header" aria-hidden="true">
          <span>名称</span><span>密钥</span><span>模型范围</span><span>使用情况</span><span>操作</span>
        </div>
        {loading ? (
          <div className="client-key-empty small">正在加载…</div>
        ) : keys.length === 0 ? (
          <div className="client-key-empty">
            <KeyIcon size={24} />
            <strong>尚未创建访问密钥</strong>
            <span className="small">创建后，OpenAI 兼容客户端才能调用代理接口。</span>
          </div>
        ) : keys.map((key) => {
          const isVisible = visible.has(key.id)
          return (
            <div className="client-key-table-row" key={key.id}>
              <div className="client-key-name">
                <strong>{key.name}</strong>
                <span className="small">创建于 {formatTime(key.created_at)}</span>
              </div>
              <div className="client-key-value">
                <code>{key.api_key ? (isVisible ? key.api_key : maskKey(key.api_key)) : key.prefix}</code>
                {key.api_key ? (
                  <button
                    className="btn btn-ghost btn-sm client-key-eye"
                    type="button"
                    aria-label={isVisible ? `隐藏 ${key.name} 的密钥` : `显示 ${key.name} 的密钥`}
                    onClick={() => toggleVisible(key.id)}
                  >
                    {isVisible ? <EyeOffIcon size={14} /> : <EyeIcon size={14} />}
                  </button>
                ) : <span className="legacy-key-badge">旧密钥不可恢复</span>}
              </div>
              <button
                className="btn btn-sm client-key-model-button"
                type="button"
                onClick={() => setModelEditor({ kind: 'key', key })}
                title="设置此密钥可以使用的模型"
              >
                <ModelIcon size={12} /> {modelScopeLabel(key.allowed_models)}
              </button>
              <span className="small client-key-usage">
                {key.last_used_at ? `最后使用 ${formatTime(key.last_used_at)}` : '尚未使用'}
              </span>
              <div className="client-key-actions">
                {key.api_key && <CopyButton text={key.api_key} label="复制" />}
                <button className="btn btn-danger btn-sm" type="button" onClick={() => removeKey(key)}>
                  <TrashIcon size={12} /> 撤销
                </button>
              </div>
            </div>
          )
        })}
      </div>

      {modelEditor && (
        <ClientKeyModelsModal
          name={modelEditor.kind === 'create' ? '新访问密钥' : modelEditor.key.name}
          models={models}
          initial={modelEditor.kind === 'create' ? createAllowedModels : modelEditor.key.allowed_models}
          busy={modelBusy}
          onClose={() => setModelEditor(null)}
          onSave={(allowedModels) => void saveModelScope(allowedModels)}
        />
      )}
    </div>
  )
}

function modelScopeLabel(allowedModels: string[] | null) {
  return allowedModels === null ? '全部模型' : `${allowedModels.length} 个模型`
}

function maskKey(key: string) {
  return `${key.slice(0, 8)}${'•'.repeat(16)}${key.slice(-4)}`
}

function formatTime(seconds: number) {
  return new Date(seconds * 1000).toLocaleString()
}
