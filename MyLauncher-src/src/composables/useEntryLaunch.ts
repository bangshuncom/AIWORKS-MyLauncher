// ════════════════════════════════════════════
// 条目启动：网址 / 程序 / 平台游戏（provider）
// 平台游戏支持两种启动方式（平台协议链接 / 直接跑主程序），由调用方传入的弹窗状态驱动
// ════════════════════════════════════════════

import type { Ref } from 'vue'
import * as api from '@/api'
import { state, updateLastUsed } from '@/store'
import type { Entry } from '@/types'
import { useToast } from '@/composables/useToast'

export interface ProviderChoiceRefs {
  showProviderChoiceDialog: Ref<boolean>
  providerChoiceEntry: Ref<Entry | null>
}

/** 有文件路径的类型（网址/系统功能/APPX 不显示「目录」；平台游戏定位到主程序） */
export function hasFilePath(entry: Entry): boolean {
  return entry.type === 'program' || entry.type === 'folder' || entry.type === 'file'
    || (entry.type === 'provider' && !!Object.values(entry.absolute_paths)[0])
}

export function useEntryLaunch(provider: ProviderChoiceRefs) {
  const { showToast } = useToast()

  async function handleLaunch(entry: Entry) {
    // 平台游戏条目：主程序与协议链接并存时弹窗让用户选择启动方式
    if (entry.type === 'provider' && entry.url && Object.values(entry.absolute_paths)[0]) {
      provider.providerChoiceEntry.value = entry
      provider.showProviderChoiceDialog.value = true
      return
    }

    if (entry.type === 'url') {
      const success = await api.launch_program(entry, state.currentEnvId)
      if (success) {
        await updateLastUsed(entry.id)
        showToast(`已打开: ${entry.name}`)
      }
      return
    }

    // 平台游戏仅有协议链接（无主程序）：直接走链接，不校验路径
    if (entry.type === 'provider' && entry.url) {
      const success = await api.launch_program(entry, state.currentEnvId)
      if (success) {
        await updateLastUsed(entry.id)
        showToast(`已启动: ${entry.name}`)
      } else {
        showToast(`启动失败: ${entry.name}`)
      }
      return
    }

    const pathInfo = await api.resolve_path(entry, state.currentEnvId)
    if (!pathInfo.exists) {
      showToast(`路径失效: ${entry.name}`)
      return
    }
    const success = await api.launch_program(entry, state.currentEnvId)
    if (success) {
      await updateLastUsed(entry.id)
      showToast(`已启动: ${entry.name}`)
    } else {
      showToast(`启动失败: ${entry.name}`)
    }
  }

  /** 执行平台游戏启动（按用户选择的方式） */
  async function launchProviderEntry(mode: 'provider' | 'exe') {
    const entry = provider.providerChoiceEntry.value
    provider.showProviderChoiceDialog.value = false
    if (!entry) return

    if (mode === 'provider') {
      // 直接用原条目（url = 平台协议链接）启动
      const success = await api.launch_program(entry, state.currentEnvId)
      if (success) {
        await updateLastUsed(entry.id)
        showToast(`已启动: ${entry.name}`)
      } else {
        showToast(`启动失败: ${entry.name}`)
      }
      return
    }

    // exe 方式：条目 absolute_paths 中保存了主程序路径（导入时存入），
    // 构造临时 program 条目直接启动
    const exePath = Object.values(entry.absolute_paths)[0] || ''
    if (!exePath) {
      showToast(`主程序路径未保存: ${entry.name}`)
      return
    }
    const proxyEntry: Entry = {
      ...entry,
      type: 'program',
      path_mode: 'absolute',
      relative_path: '',
      absolute_paths: { [state.currentEnvId]: exePath },
    }
    const success = await api.launch_program(proxyEntry, state.currentEnvId)
    if (success) {
      await updateLastUsed(entry.id)
      showToast(`已启动: ${entry.name}`)
    } else {
      showToast(`启动失败: ${entry.name}`)
    }
  }

  /** 打开条目所在目录（资源管理器定位并选中） */
  async function openInExplorer(entry: Entry) {
    try {
      await api.open_in_explorer(entry, state.currentEnvId)
    } catch (e) {
      showToast(`打开目录失败: ${e}`)
    }
  }

  return { handleLaunch, launchProviderEntry, openInExplorer, hasFilePath }
}
