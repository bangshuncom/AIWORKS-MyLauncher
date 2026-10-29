// ═══════════════════════════════════════════
// 条目：增删改、路径模式转换、图标重提取、批量启动
// ═══════════════════════════════════════════

import * as api from '@/api'
import type { Entry, EntryType } from '@/types'
import { SYSTEM_TOOLS, toolIconSource } from '@/data/systemTools'
import { state } from './state'
import { getSubCategories, isCategoryHidden } from './categories'
import { bumpIconVersion, cleanupEntriesAssets } from './assets'

export function getCurrentEntries(): Entry[] {
  if (!state.currentCategoryId) return []

  // 收集当前分类及所有子分类 ID（隐藏分类及其子树跳过）
  const categoryIds = new Set<string>()
  const collectChildren = (parentId: string) => {
    categoryIds.add(parentId)
    getSubCategories(parentId).forEach(sub => {
      if (isCategoryHidden(sub)) return
      collectChildren(sub.id)
    })
  }
  collectChildren(state.currentCategoryId)

  // 如果有搜索关键词，全局搜索（隐藏分类下的条目同样不出现）
  if (state.searchQuery.trim()) {
    const q = state.searchQuery.toLowerCase().trim()
    const hiddenIds = new Set(state.categories.filter(c => isCategoryHidden(c)).map(c => c.id))
    return state.entries.filter(e => {
      // 隐藏分类及其子树的条目不参与搜索
      if (hiddenIds.has(e.category_id)) return false
      // 搜索时全局搜索，不限分类
      const matchName = e.name.toLowerCase().includes(q)
      const matchTags = e.tags.some(t => t.toLowerCase().includes(q))
      const matchNotes = e.notes.toLowerCase().includes(q)
      const matchUrl = e.url.toLowerCase().includes(q)
      return matchName || matchTags || matchNotes || matchUrl
    })
  }

  // 按分类 + 环境过滤
  return state.entries.filter(e => {
    if (!categoryIds.has(e.category_id)) return false

    // 环境过滤
    if (e.path_mode === 'absolute') {
      return e.absolute_paths && state.currentEnvId in e.absolute_paths
    }
    // 相对路径在所有环境都显示
    return true
  })
}

// ─── Actions: 条目 ───

export async function addEntry(entry: Omit<Entry, 'id' | 'add_time' | 'last_used'>): Promise<string> {
  const id = `entry_${state.idCounter++}`
  const now = await api.get_now()
  const newEntry: Entry = {
    ...entry,
    id,
    add_time: now,
    last_used: '',
  }
  state.entries.push(newEntry)
  await api.save_entries(state.entries)
  return id
}

export async function updateEntry(id: string, updates: Partial<Entry>) {
  const idx = state.entries.findIndex(e => e.id === id)
  if (idx >= 0) {
    state.entries[idx] = { ...state.entries[idx], ...updates }
    await api.save_entries(state.entries)
  }
}

/** 同步后端批量导入已写盘的新条目到内存状态（id/分类已由后端分配），并校正 ID 计数器 */
export function appendImportedEntries(entries: Entry[]) {
  if (entries.length === 0) return
  state.entries.push(...entries)
  const maxId = entries.reduce((max, e) => {
    const match = e.id.match(/entry_(\d+)/)
    return match ? Math.max(max, parseInt(match[1])) : max
  }, 0)
  if (maxId + 1 > state.idCounter) {
    state.idCounter = maxId + 1
  }
}

// 收集条目的资源文件引用（图标 + 封面，assets 相对路径），供删除后联动清理

export async function deleteEntries(ids: string[]) {
  const idSet = new Set(ids)
  // 先收集被删条目（含资源引用），再过滤删除
  const deletedEntries = state.entries.filter(e => idSet.has(e.id))
  state.entries = state.entries.filter(e => !idSet.has(e.id))
  await api.save_entries(state.entries)
  // 联动清理不再被引用的图标/封面
  await cleanupEntriesAssets(deletedEntries)
}

export async function moveEntriesToCategory(ids: string[], categoryId: string) {
  const idSet = new Set(ids)
  state.entries.forEach(e => {
    if (idSet.has(e.id)) e.category_id = categoryId
  })
  await api.save_entries(state.entries)
}

// 批量转换路径模式（相对↔绝对）
// 已经是目标模式的条目直接忽略
export async function convertEntriesPathMode(ids: string[], targetMode: 'relative' | 'absolute'): Promise<{ success: number; failed: number; skipped: number; messages: string[] }> {
  const messages: string[] = []
  let success = 0
  let failed = 0
  let skipped = 0

  for (const id of ids) {
    const idx = state.entries.findIndex(e => e.id === id)
    if (idx < 0) continue
    const entry = state.entries[idx]
    // 网址/系统功能/APPX/平台游戏不转换路径（平台游戏的 exe 路径仅作定位/直启用，模式固定绝对）
    if (entry.type === 'url' || entry.type === 'system' || entry.type === 'appx' || entry.type === 'provider') continue

    // 已经是目标模式，直接忽略
    if (entry.path_mode === targetMode) {
      skipped++
      continue
    }

    try {
      const result = await api.convert_path_mode(entry, state.currentEnvId, targetMode)
      if (result.success) {
        state.entries[idx].path_mode = result.new_path_mode as 'relative' | 'absolute'
        if (targetMode === 'relative') {
          state.entries[idx].relative_path = result.new_relative_path
          // 转为相对路径后，清除当前环境的绝对路径
          delete state.entries[idx].absolute_paths[state.currentEnvId]
        } else {
          // 转为绝对路径后，设置当前环境的绝对路径
          state.entries[idx].absolute_paths[state.currentEnvId] = result.new_absolute_path
        }
        success++
      } else {
        failed++
        messages.push(`${entry.name}: ${result.message}`)
      }
    } catch (e) {
      failed++
      messages.push(`${entry.name}: ${e}`)
    }
  }

  await api.save_entries(state.entries)
  return { success, failed, skipped, messages }
}

// 批量重新提取图标
// 系统功能：有 icon_source 的按来源重提取；旧 emoji 条目按清单反向匹配升级，
// 都没有的跳过；其余类型调用后端按目标重新提取，
// 成功后更新条目图标（type 统一记为 extracted——favicon 也存于 assets 内的文件资源）。
// url 类型多源 favicon 链全部失败时保持原图标不变（跳过），不计失败。
// force=false（默认）：url 命中 domain 缓存直接复用，不重复下载；
// force=true（勾选「强制」）：忽略本地已有图标与缓存，全部重新下载/提取。
export async function refreshEntriesIcons(ids: string[], force = false): Promise<{ success: number; failed: number; skipped: number; messages: string[] }> {
  const messages: string[] = []
  let success = 0
  let failed = 0
  let skipped = 0

  for (const id of ids) {
    const idx = state.entries.findIndex(e => e.id === id)
    if (idx < 0) continue
    const entry = state.entries[idx]

    // 系统功能：icon_source 或清单反查（target + args 匹配）任一命中即可重提取
    if (entry.type === 'system') {
      let source = entry.icon_source?.trim() || ''
      if (!source) {
        const target = entry.relative_path?.trim() || ''
        const tool = SYSTEM_TOOLS.find(
          t => t.target === target && (t.args || '') === (entry.launch_args || '')
        )
        if (tool) source = toolIconSource(tool)
      }
      if (!source) {
        skipped++
        continue
      }
      try {
        const p = await api.extract_system_icon(source, true)
        state.entries[idx].icon = p
          ? { type: 'extracted', source: p }
          : { type: 'default', source: '' }
        state.entries[idx].icon_source = source
        bumpIconVersion(p)
        success++
      } catch (e) {
        failed++
        messages.push(`${entry.name}: ${e}`)
      }
      continue
    }

    try {
      const r = await api.refresh_entry_icon(entry, state.currentEnvId, force)
      if (r.success) {
        state.entries[idx].icon = r.icon_path
          ? { type: 'extracted', source: r.icon_path }
          : { type: 'default', source: '' }
        // bump 图标版本：URL 拼上 ?v= 强制 WebView 重载新图（文件路径不变时会命中缓存）
        bumpIconVersion(r.icon_path)
        success++
      } else if (entry.type === 'url' || entry.type === 'provider') {
        // url 类型 favicon 链全失败 / 平台游戏条目无 exe 路径：跳过（保持原图标不变）
        skipped++
      } else {
        failed++
        messages.push(`${entry.name}: ${r.message || '提取失败'}`)
      }
    } catch (e) {
      failed++
      messages.push(`${entry.name}: ${e}`)
    }
  }

  if (success > 0) await api.save_entries(state.entries)
  return { success, failed, skipped, messages }
}

export async function updateLastUsed(id: string) {
  const idx = state.entries.findIndex(e => e.id === id)
  if (idx >= 0) {
    state.entries[idx].last_used = await api.get_now()
    await api.save_entries(state.entries)
  }
}

// 批量启动：逐条调用后端启动，成功后更新最后使用时间
// 路径类条目先 resolve_path 校验，路径失效则计入失败并记录提示；网址/系统/APPX 直接启动
export async function launchEntries(ids: string[]): Promise<{ success: number; failed: number; skipped: number; messages: string[] }> {
  const messages: string[] = []
  let success = 0
  let failed = 0
  let skipped = 0

  for (const id of ids) {
    const entry = state.entries.find(e => e.id === id)
    if (!entry) continue
    try {
      let ok = false
      if (entry.type === 'url' || entry.type === 'system' || entry.type === 'appx' || entry.type === 'provider') {
        ok = await api.launch_program(entry, state.currentEnvId)
      } else {
        const pathInfo = await api.resolve_path(entry, state.currentEnvId)
        if (!pathInfo.exists) {
          failed++
          messages.push(`${entry.name}: 路径失效`)
          continue
        }
        ok = await api.launch_program(entry, state.currentEnvId)
      }
      if (ok) {
        success++
        state.entries[state.entries.findIndex(e => e.id === id)].last_used = await api.get_now()
      } else {
        failed++
        messages.push(`${entry.name}: 启动失败`)
      }
    } catch (e) {
      failed++
      messages.push(`${entry.name}: ${e}`)
    }
  }

  if (success > 0) await api.save_entries(state.entries)
  return { success, failed, skipped, messages }
}

// ─── 工具函数 ───

/** 条目工厂：创建一个带有默认值的新条目草稿（不含 id/add_time/last_used） */
export function createEntryDraft(categoryId: string, type: EntryType): Omit<Entry, 'id' | 'add_time' | 'last_used'> {
  return {
    name: '',
    type,
    category_id: categoryId,
    relative_path: '',
    absolute_paths: {},
    path_mode: 'relative',
    launch_args: '',
    working_directory: '',
    window_style: 'normal',
    run_as_admin: false,
    url: '',
    provider: null,
    update_url: '',
    icon: { type: 'default', source: '' },
    icon_source: '',
    cover: { enabled: false, source: '' },
    tags: [],
    notes: '',
  }
}
