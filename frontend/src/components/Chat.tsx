import { useEffect, useRef, useState } from 'react'
import { handleLocked } from '../api/client'
import { keysApi } from '../api/keys'
import type { KeySummary, ModelInfo } from '../api/types'
import { ChatIcon, RefreshIcon, SendIcon, StopIcon, TrashIcon } from './icons'

interface Props {
  keys: KeySummary[]
}

interface Message {
  id: string
  role: 'user' | 'assistant'
  content: string
  error?: boolean
}

const messageId = () => crypto.randomUUID()

export default function Chat({ keys }: Props) {
  const [keyId, setKeyId] = useState(keys.find((key) => key.is_default)?.id ?? keys[0]?.id ?? '')
  const [models, setModels] = useState<ModelInfo[]>([])
  const [model, setModel] = useState('')
  const [modelsLoading, setModelsLoading] = useState(false)
  const [modelsError, setModelsError] = useState('')
  const [messages, setMessages] = useState<Message[]>([])
  const [input, setInput] = useState('')
  const [running, setRunning] = useState(false)
  const abortRef = useRef<AbortController | null>(null)
  const endRef = useRef<HTMLDivElement | null>(null)

  useEffect(() => {
    if (!keyId && keys.length > 0) {
      setKeyId(keys.find((key) => key.is_default)?.id ?? keys[0].id)
    }
  }, [keyId, keys])

  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: running ? 'auto' : 'smooth' })
  }, [messages, running])

  const loadModels = async (id: string, force = false) => {
    if (!id) return
    setModelsLoading(true)
    setModelsError('')
    try {
      const record = await keysApi.get(id)
      const available = force || record.model_cache.length === 0
        ? (await keysApi.test(id)).models
        : record.model_cache
      setModels(available)
      setModel((current) => available.some((item) => item.id === current) ? current : (available[0]?.id ?? ''))
      if (available.length === 0) setModelsError('上游没有返回可用模型')
    } catch (error) {
      setModels([])
      setModel('')
      setModelsError(error instanceof Error ? error.message : '模型加载失败')
    } finally {
      setModelsLoading(false)
    }
  }

  useEffect(() => {
    loadModels(keyId)
  }, [keyId])

  const appendAssistantText = (id: string, text: string) => {
    if (!text) return
    setMessages((current) => current.map((message) => (
      message.id === id ? { ...message, content: message.content + text } : message
    )))
  }

  const send = async () => {
    const content = input.trim()
    if (!content || !keyId || !model || running) return

    const userMessage: Message = { id: messageId(), role: 'user', content }
    const assistantId = messageId()
    const history = [...messages.filter((message) => !message.error), userMessage]
    setMessages([...history, { id: assistantId, role: 'assistant', content: '' }])
    setInput('')
    setRunning(true)
    const controller = new AbortController()
    abortRef.current = controller

    try {
      const response = await fetch('/api/chat/chat/completions', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'X-Key-Id': keyId },
        body: JSON.stringify({
          model,
          stream: true,
          stream_options: { include_usage: true },
          messages: history.map(({ role, content: text }) => ({ role, content: text })),
        }),
        signal: controller.signal,
      })

      if (response.status === 423) {
        handleLocked()
        throw new Error('登录已失效，请重新登录')
      }
      if (!response.ok) {
        const value = await response.json().catch(() => null)
        throw new Error(value?.error?.message ?? `请求失败（${response.status}）`)
      }
      if (!response.body) throw new Error('上游未返回响应流')

      const reader = response.body.getReader()
      const decoder = new TextDecoder()
      let buffer = ''
      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        buffer += decoder.decode(value, { stream: true })
        const lines = buffer.split('\n')
        buffer = lines.pop() ?? ''
        for (const rawLine of lines) {
          const data = rawLine.trim().replace(/^data:\s*/, '')
          if (!rawLine.trim().startsWith('data:') || !data || data === '[DONE]') continue
          const event = JSON.parse(data)
          appendAssistantText(assistantId, event.choices?.[0]?.delta?.content ?? '')
        }
      }
    } catch (error) {
      if (error instanceof DOMException && error.name === 'AbortError') {
        setMessages((current) => current.filter((message) => message.id !== assistantId || message.content))
        return
      }
      const detail = error instanceof Error ? error.message : '对话请求失败'
      setMessages((current) => current.map((message) => (
        message.id === assistantId && !message.content
          ? { ...message, content: detail, error: true }
          : message
      )))
    } finally {
      abortRef.current = null
      setRunning(false)
    }
  }

  return (
    <div className="chat-shell">
      <div className="chat-controls" aria-label="对话设置">
        <div className="chat-select-group">
          <label htmlFor="chat-route">线路</label>
          <select id="chat-route" className="input" value={keyId} onChange={(event) => setKeyId(event.target.value)} disabled={running}>
            {keys.length === 0 && <option value="">（暂无线路）</option>}
            {keys.map((key) => <option key={key.id} value={key.id}>{key.name}{key.is_default ? ' · 默认' : ''}</option>)}
          </select>
        </div>
        <div className="chat-select-group chat-model-select">
          <label htmlFor="chat-model">模型</label>
          <select id="chat-model" className="input" value={model} onChange={(event) => setModel(event.target.value)} disabled={running || modelsLoading || models.length === 0}>
            {modelsLoading && <option value="">正在获取模型…</option>}
            {!modelsLoading && models.length === 0 && <option value="">暂无可用模型</option>}
            {models.map((item) => <option key={item.id} value={item.id}>{item.id}</option>)}
          </select>
        </div>
        <button className="btn btn-icon" type="button" title="刷新模型" aria-label="刷新模型" disabled={running || modelsLoading || !keyId} onClick={() => loadModels(keyId, true)}>
          <RefreshIcon size={14} />
        </button>
        <button className="btn btn-ghost btn-sm" type="button" disabled={running || messages.length === 0} onClick={() => setMessages([])}>
          <TrashIcon size={13} /> 清空对话
        </button>
      </div>

      {modelsError && <div className="chat-alert" role="alert">{modelsError}</div>}

      <div className="chat-thread" aria-live="polite">
        {messages.length === 0 && (
          <div className="chat-empty">
            <span className="chat-empty-icon"><ChatIcon size={24} /></span>
            <h2>开始一次对话</h2>
            <p>选择线路和模型，然后直接输入你想说的内容。</p>
          </div>
        )}
        {messages.map((message) => (
          <article key={message.id} className={`chat-message ${message.role} ${message.error ? 'error' : ''}`}>
            <div className="chat-avatar">{message.role === 'user' ? '你' : 'AI'}</div>
            <div className="chat-message-body">
              <div className="chat-role">{message.role === 'user' ? '你' : model || '助手'}</div>
              <div className="chat-content">{message.content || (running ? <span className="chat-thinking">正在思考<span>…</span></span> : '')}</div>
            </div>
          </article>
        ))}
        <div ref={endRef} />
      </div>

      <div className="chat-composer">
        <label className="sr-only" htmlFor="chat-input">输入消息</label>
        <textarea
          id="chat-input"
          className="chat-input"
          value={input}
          onChange={(event) => setInput(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && !event.shiftKey) {
              event.preventDefault()
              send()
            }
          }}
          placeholder="输入消息…"
          rows={1}
          disabled={keys.length === 0}
        />
        {running ? (
          <button className="btn chat-send" type="button" onClick={() => abortRef.current?.abort()} aria-label="停止生成" title="停止生成"><StopIcon size={14} /></button>
        ) : (
          <button className="btn btn-primary chat-send" type="button" onClick={send} disabled={!input.trim() || !keyId || !model} aria-label="发送消息" title="发送消息"><SendIcon size={15} /></button>
        )}
        <div className="chat-hint">Enter 发送 · Shift + Enter 换行</div>
      </div>
    </div>
  )
}
