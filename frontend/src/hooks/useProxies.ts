import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { proxiesApi } from '../api/keys'
import { keysQueryKey } from './useKeys'
import { toast } from '../lib/toast'

export const proxiesQueryKey = ['proxies'] as const

export function useProxies(enabled: boolean) {
  const query = useQuery({
    queryKey: proxiesQueryKey,
    queryFn: proxiesApi.list,
    enabled,
    staleTime: 10_000,
  })
  const qc = useQueryClient()
  // Proxy changes can detach keys (delete) or change which proxy a key resolves
  // to by name, so refresh the key list too.
  const invalidate = () => {
    qc.invalidateQueries({ queryKey: proxiesQueryKey })
    qc.invalidateQueries({ queryKey: keysQueryKey })
  }

  const createProxy = useMutation({
    mutationFn: proxiesApi.create,
    onSuccess: () => {
      invalidate()
      toast('代理已添加', 'ok')
    },
  })

  const updateProxy = useMutation({
    mutationFn: ({ id, input }: { id: string; input: Parameters<typeof proxiesApi.update>[1] }) =>
      proxiesApi.update(id, input),
    onSuccess: () => {
      invalidate()
      toast('代理已保存', 'ok')
    },
  })

  const deleteProxy = useMutation({
    mutationFn: proxiesApi.remove,
    onSuccess: () => {
      invalidate()
      toast('代理已删除，关联账号已解除', 'ok')
    },
  })

  const testProxy = useMutation({
    mutationFn: ({ id, kind }: { id: string; kind: Parameters<typeof proxiesApi.test>[1] }) =>
      proxiesApi.test(id, kind),
  })

  return { query, createProxy, updateProxy, deleteProxy, testProxy }
}
