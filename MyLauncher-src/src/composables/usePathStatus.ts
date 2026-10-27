// ════════════════════════════════════════════
// 条目路径有效性检查（批量问后端，结果缓存在 Map）
// ════════════════════════════════════════════

import { ref } from 'vue'
import * as api from '@/api'
import { state } from '@/store'
import type { Entry } from '@/types'

/** 有真实文件路径、需要检查是否失效的类型（网址/系统功能/APPX 恒有效） */
const CHECKABLE_TYPES = new Set(['program', 'folder', 'file', 'provider'])

export function usePathStatus(getEntries: () => Entry[]) {
  const pathStatusMap = ref<Map<string, boolean>>(new Map())

  async function refreshPathStatus() {
    const entries = getEntries().filter(e => CHECKABLE_TYPES.has(e.type))
    if (entries.length === 0) return
    try {
      const results = await api.check_paths_batch(entries, state.currentEnvId)
      const map = new Map<string, boolean>()
      entries.forEach((e, i) => map.set(e.id, results[i]))
      pathStatusMap.value = map
    } catch (e) {
      console.error('路径状态检查失败', e)
    }
  }

  return { pathStatusMap, refreshPathStatus }
}
