import type { Status } from './types'

export class ApiError extends Error {
  status: number
  constructor(status: number, message: string) {
    super(message)
    this.status = status
  }
}

/** Global handler for management API requests made after logout. Set by App. */
let onUnauthorized: (() => void) | null = null
export function setUnauthorizedHandler(fn: (() => void) | null) {
  onUnauthorized = fn
}

export function handleUnauthorized() {
  onUnauthorized?.()
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
    credentials: opts.credentials ?? 'same-origin',
    headers,
    body: opts.json !== undefined ? JSON.stringify(opts.json) : opts.body,
  })

  if (!res.ok) {
    let message = res.statusText
    try {
      const body = await res.json()
      if (body?.error) message = typeof body.error === 'string' ? body.error : body.error.message
    } catch {
      /* keep statusText */
    }
    if (res.status === 401 && message.includes('web login')) {
      handleUnauthorized()
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
