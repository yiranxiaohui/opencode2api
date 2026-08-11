import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { keysApi } from '../api/keys'
import { toast } from '../lib/toast'

export const keysQueryKey = ['keys'] as const

export function useKeys(enabled: boolean) {
  const query = useQuery({
    queryKey: keysQueryKey,
    queryFn: keysApi.list,
    enabled,
    staleTime: 10_000,
  })
  const qc = useQueryClient()
  const invalidate = () => qc.invalidateQueries({ queryKey: keysQueryKey })

  const createKey = useMutation({
    mutationFn: keysApi.create,
    onSuccess: () => {
      invalidate()
      toast('已添加密钥', 'ok')
    },
  })

  const updateKey = useMutation({
    mutationFn: ({ id, input }: { id: string; input: Parameters<typeof keysApi.update>[1] }) =>
      keysApi.update(id, input),
    onSuccess: () => {
      invalidate()
      toast('已保存修改', 'ok')
    },
  })

  const deleteKey = useMutation({
    mutationFn: keysApi.remove,
    onSuccess: (_, id) => {
      invalidate()
      toast(`已删除 ${id}`, 'ok')
    },
  })

  const setDefault = useMutation({
    mutationFn: keysApi.setDefault,
    onSuccess: () => {
      invalidate()
      toast('已设为默认账号', 'ok')
    },
  })

  const setEnabled = useMutation({
    mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) =>
      keysApi.setEnabled(id, enabled),
    onSuccess: (_, { enabled }) => {
      invalidate()
      toast(enabled ? '账号已启用' : '账号已禁用', 'ok')
    },
  })

  const testKey = useMutation({ mutationFn: keysApi.test })

  const importItems = useMutation({
    mutationFn: keysApi.import,
    onSuccess: (r) => {
      invalidate()
      toast(`导入完成：新增 ${r.imported}，更新 ${r.updated}`, 'ok')
    },
  })

  return { query, createKey, updateKey, deleteKey, setDefault, setEnabled, testKey, importItems }
}
