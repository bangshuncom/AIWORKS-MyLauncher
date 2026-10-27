// ════════════════════════════════════════════
// 对话框显隐状态集中管理
// 原先十来个 ref 全散在 App.vue 里，统一到这里便于扩展新弹窗
// ════════════════════════════════════════════

import { ref } from 'vue'
import type { Entry, EntryType } from '@/types'

export function useDialogs() {
  const showEntryEditor = ref(false)
  const editingEntry = ref<Entry | null>(null)
  const editingEntryType = ref<EntryType>('program')

  const showEnvManager = ref(false)
  const showCatManager = ref(false)
  const showScanDialog = ref(false)
  const showBookmarkDialog = ref(false)
  const showSettingsDialog = ref(false)
  const showAboutDialog = ref(false)

  const deleteCatId = ref<string | null>(null)

  // 分类管理器的初始状态：编辑某个分类 / 在某分类下新建子分类
  const catManagerEditId = ref<string | undefined>(undefined)
  const catManagerNewParentId = ref<string | null | undefined>(undefined)

  // 平台游戏启动方式选择弹窗
  const showProviderChoiceDialog = ref(false)
  const providerChoiceEntry = ref<Entry | null>(null)

  /** 打开条目编辑器；传 null 表示新增（默认 program 类型） */
  function openEntryEditor(entry: Entry | null) {
    editingEntry.value = entry
    editingEntryType.value = entry?.type ?? 'program'
    showEntryEditor.value = true
  }

  function openCatManager(editId?: string, newParentId?: string | null) {
    catManagerEditId.value = editId
    catManagerNewParentId.value = newParentId
    showCatManager.value = true
  }

  function closeCatManager() {
    showCatManager.value = false
    catManagerEditId.value = undefined
    catManagerNewParentId.value = undefined
  }

  /** LOGO 三击：弹出关于窗口的同时解锁隐藏分类（持续到关闭软件，不持久化） */
  function openAboutAndUnlock(unlock: () => void) {
    unlock()
    showAboutDialog.value = true
  }

  return {
    showEntryEditor, editingEntry, editingEntryType,
    showEnvManager, showCatManager, showScanDialog, showBookmarkDialog,
    showSettingsDialog, showAboutDialog,
    deleteCatId, catManagerEditId, catManagerNewParentId,
    showProviderChoiceDialog, providerChoiceEntry,
    openEntryEditor, openCatManager, closeCatManager, openAboutAndUnlock,
  }
}
