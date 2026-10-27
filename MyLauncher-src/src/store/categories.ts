// ═══════════════════════════════════════════
// 分类：查询、增删改、层级移动、图标
// ═══════════════════════════════════════════

import * as api from '@/api'
import type { Category } from '@/types'
import { state } from './state'
import { resolveAssetUrl, cleanupEntriesAssets } from './assets'

// ─── Getters ───

export function getTopCategories(): Category[] {
  return state.categories
    .filter(c => c.is_top_level || c.parent_id === null)
    .sort((a, b) => a.sort_order - b.sort_order)
}

export function getSubCategories(parentId: string): Category[] {
  return state.categories
    .filter(c => c.parent_id === parentId)
    .sort((a, b) => a.sort_order - b.sort_order)
}

export function getCategoryById(id: string): Category | undefined {
  return state.categories.find(c => c.id === id)
}

// 收集某分类自身及所有子孙分类 ID（含自身）
export function collectCategoryAndDescendants(id: string): Set<string> {
  const result = new Set<string>([id])
  const visit = (pid: string) => {
    state.categories.forEach(c => {
      if (c.parent_id === pid && !result.has(c.id)) {
        result.add(c.id)
        visit(c.id)
      }
    })
  }
  visit(id)
  return result
}

// 判断分类是否应在前台隐藏（未解锁时 hidden 分类不可见）
export function isCategoryHidden(cat: Category): boolean {
  return !!cat.hidden && !state.hiddenUnlocked
}

// 获取可见的顶级分类（未解锁时过滤隐藏分类）
export function getVisibleTopCategories(): Category[] {
  return getTopCategories().filter(c => !isCategoryHidden(c))
}

// 临时解锁所有隐藏分类（LOGO 三击弹关于时调用，持续到关闭软件）
export function unlockHiddenCategories() {
  state.hiddenUnlocked = true
}

// 解析分类图标：返回 { emoji: string } 或 { img: string } 或 null
export function resolveCategoryIcon(cat: Category | undefined): { emoji?: string; img?: string } | null {
  if (!cat || !cat.icon || !cat.icon.source) return null
  if (cat.icon.type === 'emoji') return { emoji: cat.icon.source }
  if (cat.icon.source.startsWith('data:')) return { img: cat.icon.source }
  // 自定义图片用 resolveAssetUrl 转换
  return { img: resolveAssetUrl(cat.icon.source) }
}


export function getCurrentCategory(): Category | undefined {
  if (!state.currentCategoryId) return undefined
  return getCategoryById(state.currentCategoryId)
}

// ─── Actions: 条目 ───

export async function addCategory(name: string, parentId: string | null): Promise<string> {
  const id = `cat_${String(state.categories.length + 1).padStart(3, '0')}_${Date.now()}`
  const maxOrder = parentId
    ? getSubCategories(parentId).reduce((max, c) => Math.max(max, c.sort_order), 0)
    : getTopCategories().reduce((max, c) => Math.max(max, c.sort_order), 0)

  const cat: Category = {
    id,
    name,
    parent_id: parentId,
    icon: { type: 'emoji', source: '📁' },
    view_mode: 'icon_grid',
    sort_order: maxOrder + 1,
    is_top_level: parentId === null,
    hidden: false,
  }
  state.categories.push(cat)
  await api.save_categories(state.categories)
  return id
}

export async function updateCategory(id: string, updates: Partial<Category>) {
  const idx = state.categories.findIndex(c => c.id === id)
  if (idx >= 0) {
    state.categories[idx] = { ...state.categories[idx], ...updates }
    await api.save_categories(state.categories)
    // 当前选中分类被设为隐藏且未解锁时，回退到第一个可见的顶级分类
    if (state.currentCategoryId === id && isCategoryHidden(state.categories[idx])) {
      const topCats = getTopCategories().filter(c => c.id !== id)
      const firstCat = topCats.find(c => !isCategoryHidden(c)) || topCats[0]
      if (firstCat) state.currentCategoryId = firstCat.id
    }
  }
}

// 递归将所有子孙分类的 view_mode 设置为指定值，返回被修改的分类数
export function applyViewModeToDescendants(parentId: string, viewMode: Category['view_mode']): number {
  let count = 0
  const visit = (pid: string) => {
    getSubCategories(pid).forEach(c => {
      if (c.view_mode !== viewMode) {
        c.view_mode = viewMode
        count++
      }
      visit(c.id)
    })
  }
  visit(parentId)
  return count
}

export async function deleteCategory(id: string, strategy: 'orphan' | 'delete' | 'move', targetId?: string) {
  // 删除前的上下文：当前选中分类、被删分类的父级（选中项被删时回退到它）
  const prevSelected = state.currentCategoryId
  const fallbackParentId = getCategoryById(id)?.parent_id ?? null

  // 统一收集当前分类及所有子孙分类 ID
  const toProcess = new Set<string>([id])
  const collectChildren = (parentId: string) => {
    getSubCategories(parentId).forEach(c => {
      toProcess.add(c.id)
      collectChildren(c.id)
    })
  }
  collectChildren(id)

  // 真正会从分类树中消失的 ID（move 策略只删当前分类本身，子分类上提保留）
  const removedIds = strategy === 'move' ? new Set<string>([id]) : toProcess

  if (strategy === 'delete') {
    // 删除分类和所有子分类，以及其下条目（联动清理条目资源）
    const deletedEntries = state.entries.filter(e => toProcess.has(e.category_id))
    state.entries = state.entries.filter(e => !toProcess.has(e.category_id))
    state.categories = state.categories.filter(c => !toProcess.has(c.id))
    await api.save_categories(state.categories)
    await api.save_entries(state.entries)
    await cleanupEntriesAssets(deletedEntries)
  } else if (strategy === 'move' && targetId) {
    // 移动条目到目标分类
    state.entries.forEach(e => {
      if (toProcess.has(e.category_id)) e.category_id = targetId
    })
    // 只删当前分类本身：直属子分类上提一级挂到被删分类的父级，
    // 否则它们的 parent_id 会指向已删除的分类而从前台导航消失（数据丢失）
    const siblings = fallbackParentId
      ? state.categories.filter(c => c.parent_id === fallbackParentId)
      : state.categories.filter(c => c.parent_id === null)
    let order = siblings.reduce((max, c) => Math.max(max, c.sort_order), 0)
    state.categories.forEach(c => {
      if (c.parent_id === id) {
        c.parent_id = fallbackParentId
        c.sort_order = ++order
        c.is_top_level = fallbackParentId === null
      }
    })
    state.categories = state.categories.filter(c => c.id !== id)
    await api.save_categories(state.categories)
    await api.save_entries(state.entries)
  } else {
    // orphan: 条目移到未分类
    state.entries.forEach(e => {
      if (toProcess.has(e.category_id)) e.category_id = 'uncategorized'
    })
    state.categories = state.categories.filter(c => !toProcess.has(c.id))
    await api.save_categories(state.categories)
    await api.save_entries(state.entries)
  }

  // 只在当前选中分类确实被删掉时才切换导航；否则保持原位
  // （在侧栏右键删除别的子分类时，应继续停留在当前顶级分类，而不是跳到第一个分类）
  const selectionRemoved = !prevSelected
    || removedIds.has(prevSelected)
    || !getCategoryById(prevSelected)
  if (!selectionRemoved) return

  // 优先回退到被删分类的父级（即所在顶级分类），否则回退第一个可见顶级分类
  if (fallbackParentId && getCategoryById(fallbackParentId)) {
    state.currentCategoryId = fallbackParentId
    return
  }
  const topCats = getTopCategories()
  const visibleTop = topCats.filter(c => !isCategoryHidden(c))
  const firstCat = visibleTop[0] || topCats[0]
  if (firstCat) state.currentCategoryId = firstCat.id
}

// 设置分类图标（emoji 字符或自定义图片路径）
export async function setCategoryIcon(id: string, type: 'emoji' | 'custom', source: string) {
  const cat = state.categories.find(c => c.id === id)
  if (cat) {
    cat.icon = { type, source }
    await api.save_categories(state.categories)
  }
}

// 移动分类到新父分类下（拖拽改变层级）
export async function moveCategory(catId: string, newParentId: string | null, newSortOrder?: number) {
  const cat = state.categories.find(c => c.id === catId)
  if (!cat) return
  // 防止移到自己的子分类下（循环）
  if (newParentId) {
    let p: string | null = newParentId
    while (p) {
      if (p === catId) return // 不能移到自己或自己的子孙下
      const parent = state.categories.find(c => c.id === p)
      p = parent?.parent_id ?? null
    }
  }
  cat.parent_id = newParentId
  cat.is_top_level = newParentId === null
  // sort_order
  const siblings = newParentId
    ? getSubCategories(newParentId)
    : getTopCategories()
  if (newSortOrder !== undefined) {
    cat.sort_order = newSortOrder
  } else {
    const maxOrder = siblings.reduce((max, c) => Math.max(max, c.sort_order), 0)
    cat.sort_order = maxOrder + 1
  }
  await api.save_categories(state.categories)
}

// 批量移动分类（多选拖拽 / 工具栏"移动到…"）：统一应用 parent/sort_order/is_top_level 后一次写盘。
// moves 每项给出目标父级与目标 sort_order；目标为自身或自身子孙的项跳过（循环防护）。
export async function moveCategories(moves: { id: string; parent_id: string | null; sort_order: number }[]) {
  let changed = false
  for (const m of moves) {
    const cat = state.categories.find(c => c.id === m.id)
    if (!cat) continue
    if (m.parent_id) {
      let p: string | null = m.parent_id
      let cyclic = false
      while (p) {
        if (p === m.id) { cyclic = true; break }
        const parent = state.categories.find(c => c.id === p)
        p = parent?.parent_id ?? null
      }
      if (cyclic) continue
    }
    cat.parent_id = m.parent_id
    cat.is_top_level = m.parent_id === null
    cat.sort_order = m.sort_order
    changed = true
  }
  if (changed) await api.save_categories(state.categories)
}

// 批量更新分类字段（如批量设置显示模式），一次写盘
export async function batchUpdateCategories(ids: string[], updates: Partial<Category>) {
  let changed = false
  ids.forEach(id => {
    const idx = state.categories.findIndex(c => c.id === id)
    if (idx >= 0) {
      state.categories[idx] = { ...state.categories[idx], ...updates }
      changed = true
    }
  })
  if (changed) await api.save_categories(state.categories)
}
