import type { Status } from './types'

export class ApiError extends Error {
  status: number
  constructor(status: number, message: string) {
    super(message)
    this.status = status
  }
}

/** Global handler for HTTP 423 (vault locked). Set by App. */
let onLocked: (() => void) | null = null
export function setLockedHandler(fn: (() => void) | null) {
  onLocked = fn
}

export function handleLocked() {
  onLocked?.()
}

interface ReqOpts extends Omit<RequestInit, 'body'> {
  json?: unknown
  body?: BodyInit
}

export async function request<T>(path: string, opts: ReqOpts = {}): Promise<T> {
  const headers: Record<string, string> = { ...(opts.headers as Record<string, string> | undefined) }
  if (opts.json !== undefined) headers['Content-Type'] = 'application/json'

  const res = await fetch(path, {
    ...opts,
    headers,
    body: opts.json !== undefined ? JSON.stringify(opts.json) : opts.body,
  })

  if (res.status === 423) {
    handleLocked()
    throw new ApiError(423, 'locked')
  }

  if (!res.ok) {
    let message = res.statusText
    try {
      const body = await res.json()
      if (body?.error) message = typeof body.error === 'string' ? body.error : body.error.message
    } catch {
      /* keep statusText */
    }
    throw new ApiError(res.status, message)
  }

  return res.json() as Promise<T>
}

export const get = <T>(path: string) => request<T>(path)
export const post = <T>(path: string, json?: unknown) => request<T>(path, { method: 'POST', json })
export const put = <T>(path: string, json?: unknown) => request<T>(path, { method: 'PUT', json })
export const del = <T>(path: string) => request<T>(path, { method: 'DELETE' })

export const apiStatus = () => get<Status>('/api/status')
