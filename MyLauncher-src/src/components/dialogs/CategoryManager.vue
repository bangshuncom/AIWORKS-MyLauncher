<script setup lang="ts">
import { ref, computed, onMounted, nextTick, onBeforeUnmount } from 'vue'
import {
  state, getTopCategories, getSubCategories,
  addCategory, updateCategory, deleteCategory,
  setCategoryIcon, moveCategories, batchUpdateCategories,
  getCategoryById, resolveCategoryIcon, applyViewModeToDescendants,
  collectCategoryAndDescendants,
} from '@/store'
import * as api from '@/api'
import type { Category, ViewMode } from '@/types'
import CategoryEditPanel from './CategoryEditPanel.vue'
import CategoryIcon from '@/components/CategoryIcon.vue'
import DeleteCategoryDialog from './DeleteCategoryDialog.vue'
import { useI18n } from '@/composables/useI18n'

const props = defineProps<{
  initialEditId?: string
  initialNewParentId?: string | null
}>()

const { t } = useI18n()

// ════════════════════════════════════════════
// 一、行内编辑面板（名称 / 父分类 / 显示模式 / 图标）
// ════════════════════════════════════════════

const editing = ref<Category | null>(null)
const isNew = ref(false)
const newParentId = ref<string | null>(null)
const form = ref({
  name: '',
  parent_id: null as string | null,
  view_mode: 'icon_grid' as ViewMode,
  // 前台隐藏（仅非顶级分类可勾选）
  hidden: false,
  // 新建分类时暂存的图标选择（保存后应用）
  icon: null as { type: 'emoji' | 'custom'; source: string } | null,
})

// 同步显示模式到所有子分类的勾选状态
const syncChildren = ref(false)

// 当前编辑面板应该插入到哪个分类行下方（顶部 + 新建一级分类时为 null）
const panelAnchorId = ref<string | null>(null)

// 递归展平分类树（所有层级，按 sort_order 排序），用于渲染
const flatCategoryTree = computed(() => {
  const result: { cat: Category; depth: number }[] = []
  const visit = (parentId: string, depth: number) => {
    state.categories
      .filter(c => c.parent_id === parentId)
      .sort((a, b) => a.sort_order - b.sort_order)
      .forEach(c => {
        result.push({ cat: c, depth })
        visit(c.id, depth + 1)
      })
  }
  // 根节点：is_top_level 或 parent_id 为 null
  const roots = state.categories
    .filter(c => c.is_top_level || c.parent_id === null)
    .sort((a, b) => a.sort_order - b.sort_order)
  roots.forEach(r => {
    result.push({ cat: r, depth: 0 })
    visit(r.id, 1)
  })
  return result
})

// 判断某个分类行下方是否应该显示编辑面板
function showPanelFor(catId: string): boolean {
  return (editing.value?.id === catId || (isNew.value && newParentId.value === catId)) && panelAnchorId.value === catId
}

// 判断是否在底部显示面板（新建一级分类、无 anchor）
const showPanelAtBottom = computed(() => {
  return (editing.value || isNew.value) && panelAnchorId.value === null
})

// 挂载时根据 props 自动进入编辑/新建模式
onMounted(() => {
  if (props.initialEditId) {
    const cat = getCategoryById(props.initialEditId)
    if (cat) startEdit(cat)
  } else if (props.initialNewParentId !== undefined) {
    startNew(props.initialNewParentId)
  }
  window.addEventListener('keydown', onKeydown)
})

onBeforeUnmount(() => {
  document.removeEventListener('mousemove', onDragMove)
  document.removeEventListener('mouseup', onDragEnd)
  document.body.classList.remove('cat-dragging')
  window.removeEventListener('keydown', onKeydown)
})

function startNew(parentId: string | null) {
  isNew.value = true
  editing.value = null
  newParentId.value = parentId
  panelAnchorId.value = parentId
  form.value = { name: '', parent_id: parentId ?? null, view_mode: 'icon_grid', hidden: false, icon: null }
  syncChildren.value = false
  scrollToPanel(parentId)
}

function startEdit(cat: Category) {
  isNew.value = false
  editing.value = cat
  newParentId.value = null
  panelAnchorId.value = cat.id
  form.value = {
    name: cat.name,
    parent_id: cat.parent_id,
    view_mode: cat.view_mode,
    hidden: !!cat.hidden,
    icon: cat.icon ? { type: cat.icon.type as 'emoji' | 'custom', source: cat.icon.source } : null,
  }
  syncChildren.value = false
  scrollToPanel(cat.id)
}

function scrollToPanel(anchorId: string | null) {
  nextTick(() => {
    const el = document.querySelector(anchorId ? `[data-cat-id="${anchorId}"]` : '[data-panel-bottom]')
    if (el) el.scrollIntoView({ behavior: 'smooth', block: 'nearest' })
  })
}

function cancelEdit() {
  editing.value = null
  isNew.value = false
  panelAnchorId.value = null
}

async function save() {
  if (!form.value.name.trim()) return
  if (isNew.value) {
    const newId = await addCategory(form.value.name.trim(), form.value.parent_id)
    // 新建时如果有暂存的图标选择，保存后应用
    if (form.value.icon) {
      await setCategoryIcon(newId, form.value.icon.type, form.value.icon.source)
    }
  } else if (editing.value) {
    const parentId = form.value.parent_id
    const isNowTopLevel = parentId === null
    // 父分类变化时，重新计算 sort_order 和 is_top_level
    const updates: Partial<Category> = {
      name: form.value.name.trim(),
      parent_id: parentId,
      view_mode: form.value.view_mode,
      is_top_level: isNowTopLevel,
      hidden: form.value.hidden,
    }
    if (parentId !== editing.value.parent_id) {
      // 移到新父级下，排到末尾
      const siblings = parentId
        ? getSubCategories(parentId).filter(c => c.id !== editing.value!.id)
        : getTopCategories().filter(c => c.id !== editing.value!.id)
      const maxOrder = siblings.reduce((max, c) => Math.max(max, c.sort_order), 0)
      updates.sort_order = maxOrder + 1
    }
    await updateCategory(editing.value.id, updates)
    // 勾选同步时，将所有子孙分类的显示模式一并更新
    if (syncChildren.value) {
      const count = applyViewModeToDescendants(editing.value.id, form.value.view_mode)
      if (count > 0) {
        await api.save_categories(state.categories)
      }
    }
  }
  cancelEdit()
}

// ─── 删除分类（单行删除走独立对话框，三策略） ───
const deleteTargetId = ref<string | null>(null)

function remove(id: string) {
  deleteTargetId.value = id
}

function handleDeleted() {
  if (editing.value?.id === deleteTargetId.value) cancelEdit()
  deleteTargetId.value = null
}

// ─── 图标编辑 ───
// 编辑已有分类：直接应用图标；新建分类：暂存到 form.icon，保存后应用
async function setEmoji(emoji: string) {
  if (editing.value) {
    await setCategoryIcon(editing.value.id, 'emoji', emoji)
  } else {
    form.value.icon = { type: 'emoji', source: emoji }
  }
}

// 图标上传：IconPicker 已完成文件选择和复制，这里只需应用
async function uploadIcon(relPath: string) {
  if (editing.value) {
    await setCategoryIcon(editing.value.id, 'custom', relPath)
  } else {
    form.value.icon = { type: 'custom', source: relPath }
  }
}

// ════════════════════════════════════════════
// 二、多选（对话框内本地选中集，与前台条目批量模式独立）
// ════════════════════════════════════════════

const selectedCatIds = ref<Set<string>>(new Set())
const lastClickedId = ref<string | null>(null)
const selectedCount = computed(() => selectedCatIds.value.size)

function toggleSelect(id: string) {
  const next = new Set(selectedCatIds.value)
  if (next.has(id)) next.delete(id)
  else next.add(id)
  selectedCatIds.value = next
  lastClickedId.value = id
}

// Shift 连选：flatCategoryTree 顺序上 lastClicked → 当前行整段加入
function rangeSelect(id: string) {
  const ids = flatCategoryTree.value.map(n => n.cat.id)
  const from = lastClickedId.value ? ids.indexOf(lastClickedId.value) : -1
  const to = ids.indexOf(id)
  if (from < 0 || to < 0) {
    toggleSelect(id)
    return
  }
  const [lo, hi] = from <= to ? [from, to] : [to, from]
  const next = new Set(selectedCatIds.value)
  ids.slice(lo, hi + 1).forEach(i => next.add(i))
  selectedCatIds.value = next
  lastClickedId.value = id
}

function selectAll() {
  selectedCatIds.value = new Set(flatCategoryTree.value.map(n => n.cat.id))
}

function invertSelection() {
  const next = new Set<string>()
  flatCategoryTree.value.forEach(n => {
    if (!selectedCatIds.value.has(n.cat.id)) next.add(n.cat.id)
  })
  selectedCatIds.value = next
}

function clearSelection() {
  selectedCatIds.value = new Set()
  deleteConfirming.value = false
}

// 快捷键：Esc 清除选择；Ctrl/Cmd+A 全选（焦点在输入框内时不拦截，保留文本编辑行为）
function onKeydown(e: KeyboardEvent) {
  const el = e.target as HTMLElement | null
  if (el?.closest?.('input, textarea, select, [contenteditable]')) return
  if (e.key === 'Escape') {
    clearSelection()
    return
  }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'a') {
    e.preventDefault()
    selectAll()
  }
}

// ════════════════════════════════════════════
// 三、批量操作（移动 / 显示模式 / 删除）
// ════════════════════════════════════════════

const batchMoveTarget = ref('')          // '' 表示一级分类（parent = null）
const batchViewMode = ref<ViewMode>('icon_grid')
const deleteConfirming = ref(false)

const viewModes: ViewMode[] = ['vertical_card', 'horizontal_card', 'icon_grid', 'table', 'waterfall']

// 批量删除会影响的条目数（选中分类 + 全部子孙分类下的条目）
const batchDeleteEntryCount = computed(() => {
  const ids = new Set<string>()
  selectedCatIds.value.forEach(id => collectCategoryAndDescendants(id).forEach(d => ids.add(d)))
  return state.entries.filter(e => ids.has(e.category_id)).length
})

// 可选的移动目标：排除选中项及其子孙（防循环），含"无（一级分类）"
const moveTargetOptions = computed(() => {
  const excluded = new Set<string>()
  selectedCatIds.value.forEach(id => collectCategoryAndDescendants(id).forEach(d => excluded.add(d)))
  const result: { id: string; name: string; depth: number }[] = []
  const visit = (parentId: string | null, depth: number) => {
    state.categories
      .filter(c => (c.parent_id ?? null) === parentId && !excluded.has(c.id))
      .sort((a, b) => a.sort_order - b.sort_order)
      .forEach(c => {
        result.push({ id: c.id, name: c.name, depth })
        visit(c.id, depth + 1)
      })
  }
  visit(null, 0)
  return result
})

// 批量移动：保持选择时的相对顺序，挂到目标父级末尾
async function applyBatchMove() {
  const treeIds = flatCategoryTree.value.map(n => n.cat.id)
  const ids = Array.from(selectedCatIds.value).sort((a, b) => treeIds.indexOf(a) - treeIds.indexOf(b))
  const parentId = batchMoveTarget.value || null
  const siblings = parentId ? getSubCategories(parentId) : getTopCategories()
  let order = siblings.reduce((max, c) => Math.max(max, c.sort_order), 0)
  await moveCategories(ids.map(id => ({ id, parent_id: parentId, sort_order: ++order })))
  clearSelection()
}

// 批量设置显示模式
async function applyBatchViewMode() {
  await batchUpdateCategories(Array.from(selectedCatIds.value), { view_mode: batchViewMode.value })
}

// 批量删除：逐个删（父级先删会连带子孙，后续项自动跳过）；strategy 决定条目去向
async function applyBatchDelete(strategy: 'delete' | 'orphan') {
  for (const id of Array.from(selectedCatIds.value)) {
    if (getCategoryById(id)) await deleteCategory(id, strategy)
  }
  clearSelection()
}

// ════════════════════════════════════════════
// 四、指针拖拽（排序 / 变为子分类 / 多选整组拖拽）
//
// 说明：HTML5 DnD API（draggable + dragstart/dragover/drop）在 WebView2 中不可靠
// （行内的 img、按钮、文本选择都会干扰），继续用指针事件手动实现。相比旧版：
//   ① 拖拽幽灵卡片跟随光标，多选时显示数量 —— 拖拽有明确的视觉载体
//   ② 行上/下 25% 区域显示插入线（排序），行中部高亮（变为子分类）
//   ③ 目标为自身或任一拖动项的子孙时标记无效，不给"能放下"的错觉
//   ④ 贴近滚动容器边缘自动滚动，长列表也能拖到底
// ════════════════════════════════════════════

const dragIds = ref<Set<string>>(new Set())   // 本次拖拽的分类集合
const dropTarget = ref<{ id: string; mode: 'child' | 'before' | 'after' } | null>(null)
const invalidTargetId = ref<string | null>(null)
const ghostVisible = ref(false)
const ghostRef = ref<HTMLElement | null>(null)
const bodyRef = ref<HTMLElement | null>(null)
let dragStartX = 0
let dragStartY = 0
let dragMoved = false
let justDragged = false

// 幽灵卡片内容：单选显示名称+图标，多选显示数量
const dragGhost = computed(() => {
  const ids = Array.from(dragIds.value)
  if (ids.length === 1) {
    const c = getCategoryById(ids[0])
    return { multi: false, count: 1, name: c?.name ?? '', icon: resolveCategoryIcon(c) }
  }
  return { multi: true, count: ids.length, name: '', icon: null }
})

// 拖拽已选中项 → 整组拖动；拖拽未选中项 → 只拖它自己（不改变当前选择）
function resolveDragSet(catId: string): Set<string> {
  if (selectedCatIds.value.has(catId) && selectedCatIds.value.size > 1) {
    return new Set(selectedCatIds.value)
  }
  return new Set([catId])
}

function onRowMouseDown(e: MouseEvent, catId: string) {
  // 仅左键；从按钮/表单控件上按下时不启动拖拽（避免与点击操作冲突）
  if (e.button !== 0) return
  const t = e.target as HTMLElement
  if (t.closest('button, input, select, textarea, .cat-row-actions')) return

  dragMoved = false
  justDragged = false
  dragStartX = e.clientX
  dragStartY = e.clientY
  dragIds.value = resolveDragSet(catId)
  document.addEventListener('mousemove', onDragMove)
  document.addEventListener('mouseup', onDragEnd)
}

// 无效落点：自身或任一拖动项的子孙分类
function isInvalidDropTarget(targetId: string): boolean {
  for (const id of dragIds.value) {
    if (id === targetId) return true
    if (collectCategoryAndDescendants(id).has(targetId)) return true
  }
  return false
}

// 贴近滚动容器上下边缘时自动滚动（速度与距离成正比）
function autoScroll(clientY: number) {
  const el = bodyRef.value
  if (!el) return
  const rect = el.getBoundingClientRect()
  const edge = 40
  if (clientY < rect.top + edge) {
    el.scrollTop -= Math.max(3, (rect.top + edge - clientY) / 4)
  } else if (clientY > rect.bottom - edge) {
    el.scrollTop += Math.max(3, (clientY - (rect.bottom - edge)) / 4)
  }
}

function onDragMove(e: MouseEvent) {
  if (!dragIds.value.size) return
  // 移动阈值，避免单击抖动
  const dx = e.clientX - dragStartX
  const dy = e.clientY - dragStartY
  if (!dragMoved && Math.abs(dx) + Math.abs(dy) < 5) return

  if (!dragMoved) {
    dragMoved = true
    ghostVisible.value = true
    document.body.classList.add('cat-dragging')
  }

  // 幽灵卡片跟随光标（直接写 transform，避开每帧响应式开销）
  if (ghostRef.value) {
    ghostRef.value.style.transform = `translate(${e.clientX + 12}px, ${e.clientY + 8}px)`
  }

  autoScroll(e.clientY)

  // 实时计算鼠标所在行与落点模式
  const el = document.elementFromPoint(e.clientX, e.clientY)
  let row = el?.closest?.('[data-cat-id]') as HTMLElement | null
  if (!row) {
    // 行间缝隙（.cat-tree 的 4px gap）内命不中行：向上微移 6px 再测，
    // 归入上一行的 after 区域，避免在缝隙释放被当成取消拖拽
    const el2 = document.elementFromPoint(e.clientX, e.clientY - 6)
    row = el2?.closest?.('[data-cat-id]') as HTMLElement | null
  }
  const hoveredId = row?.dataset.catId
  if (!hoveredId || !row) {
    dropTarget.value = null
    invalidTargetId.value = null
    return
  }

  if (isInvalidDropTarget(hoveredId)) {
    dropTarget.value = null
    invalidTargetId.value = hoveredId
    return
  }
  invalidTargetId.value = null

  // 上 25% → 插到前面；下 25% → 插到后面；中部 → 成为子分类
  const rect = row.getBoundingClientRect()
  const ratio = (e.clientY - rect.top) / rect.height
  const mode = ratio < 0.25 ? 'before' : ratio > 0.75 ? 'after' : 'child'
  dropTarget.value = { id: hoveredId, mode }
}

async function onDragEnd() {
  document.removeEventListener('mousemove', onDragMove)
  document.removeEventListener('mouseup', onDragEnd)
  document.body.classList.remove('cat-dragging')
  ghostVisible.value = false

  const ids = Array.from(dragIds.value)
  const target = dropTarget.value
  const moved = dragMoved

  dragIds.value = new Set()
  dropTarget.value = null
  invalidTargetId.value = null

  if (!moved || !target || !ids.length) return // 未移动：视为普通点击
  justDragged = true
  await applyDrop(ids, target)
}

// 执行一次拖拽落点（单选与多选共用）
async function applyDrop(ids: string[], target: { id: string; mode: 'child' | 'before' | 'after' }) {
  const targetCat = getCategoryById(target.id)
  if (!targetCat) return
  // 拖动项按当前树序排列，保持组内相对顺序
  const ordered = ids
    .map(id => getCategoryById(id))
    .filter((c): c is Category => !!c)
    .sort((a, b) => {
      const ai = flatCategoryTree.value.findIndex(n => n.cat.id === a.id)
      const bi = flatCategoryTree.value.findIndex(n => n.cat.id === b.id)
      return ai - bi
    })

  if (target.mode === 'child') {
    // 全部挂到目标分类下，按原相对顺序排在末尾
    const siblings = getSubCategories(target.id)
    let order = siblings.reduce((max, c) => Math.max(max, c.sort_order), 0)
    await moveCategories(ordered.map(c => ({ id: c.id, parent_id: target.id, sort_order: ++order })))
  } else {
    // 同级排序：插到目标行前/后，重排该层全部 sort_order
    const draggedSet = new Set(ids)
    // 目标层级 = 目标行的父级（null 表示顶级）
    const targetParentId = targetCat.parent_id ?? null
    const siblings = (targetParentId ? getSubCategories(targetParentId) : getTopCategories())
      .filter(c => !draggedSet.has(c.id))
      .sort((a, b) => a.sort_order - b.sort_order)
    const targetIdx = siblings.findIndex(c => c.id === target.id)
    if (targetIdx < 0) return
    // 关键：跨层级拖入时（如把子分类拖到与父级平级），parent_id 必须覆写为目标层级的父级，
    // 否则 moveCategories 只改 sort_order，拖动项仍留在原来的父级下，视觉上"拖不动"
    const orderedInLayer = ordered.map(c => ({ ...c, parent_id: targetParentId }))
    siblings.splice(target.mode === 'before' ? targetIdx : targetIdx + 1, 0, ...orderedInLayer)
    await moveCategories(siblings.map((c, i) => ({ id: c.id, parent_id: c.parent_id, sort_order: i + 1 })))
  }
}

// ─── 行点击：多选 / Shift 连选；名称点击：编辑 ───

function onRowClick(e: MouseEvent, cat: Category) {
  if (justDragged) {
    justDragged = false
    return
  }
  if (e.shiftKey) {
    rangeSelect(cat.id)
    return
  }
  toggleSelect(cat.id)
}

// 点击名称编辑：拖拽刚结束时抑制 click 触发的编辑
function onCatNameClick(cat: Category) {
  if (justDragged) {
    justDragged = false
    return
  }
  startEdit(cat)
}
</script>

<template>
  <div class="dialog-overlay" @click.self="$emit('close')" @contextmenu.prevent>
    <div class="dialog cat-manager-dialog">
      <div class="dialog-header">
        <span>{{ t('cat.manager.title') }}</span>
        <button class="btn btn-icon" @click="$emit('close')">✕</button>
      </div>

      <div ref="bodyRef" class="dialog-body">
        <div class="drag-hint">{{ t('cat.manager.hint') }}</div>

        <!-- 批量操作工具栏：有选中时出现 -->
        <div v-if="selectedCount > 0 && !deleteConfirming" class="cat-batch-bar">
          <span class="batch-count">{{ t('cat.manager.selectedCount', { count: selectedCount }) }}</span>
          <button class="btn" @click="selectAll">{{ t('common.all') }}</button>
          <button class="btn" @click="invertSelection">{{ t('common.invert') }}</button>
          <button class="btn" @click="clearSelection">{{ t('common.clearSelection') }}</button>
          <span class="batch-sep"></span>
          <select v-model="batchMoveTarget" class="select" :title="t('cat.manager.moveTo')" @change="applyBatchMove">
            <option value="">{{ t('cat.manager.moveToTop') }}</option>
            <option v-for="opt in moveTargetOptions" :key="opt.id" :value="opt.id">
              {{ '  '.repeat(opt.depth) }}{{ opt.name }}
            </option>
          </select>
          <select v-model="batchViewMode" class="select" :title="t('cat.manager.setViewMode')" @change="applyBatchViewMode">
            <option v-for="m in viewModes" :key="m" :value="m">{{ t('cat.manager.viewMode.' + m) }}</option>
          </select>
          <button class="btn btn-danger" @click="deleteConfirming = true">{{ t('common.delete') }}</button>
        </div>

        <!-- 批量删除二次确认（内联，不弹窗） -->
        <div v-if="deleteConfirming" class="cat-batch-bar cat-delete-confirm">
          <span class="batch-count">{{ t('cat.manager.deleteConfirm', { count: selectedCount }) }}</span>
          <span v-if="batchDeleteEntryCount > 0" class="delete-warn">
            {{ t('cat.manager.deleteWithEntries', { count: batchDeleteEntryCount }) }}
          </span>
          <button class="btn btn-danger" @click="applyBatchDelete('delete')">{{ t('cat.manager.deleteWithEntriesBtn') }}</button>
          <button class="btn" @click="applyBatchDelete('orphan')">{{ t('cat.manager.keepEntries') }}</button>
          <button class="btn" @click="deleteConfirming = false">{{ t('common.cancel') }}</button>
        </div>

        <!-- 分类树：递归展平渲染所有层级 -->
        <div class="cat-tree">
          <template v-for="node in flatCategoryTree" :key="node.cat.id">
            <div
              class="cat-row"
              :class="{
                selected: selectedCatIds.has(node.cat.id),
                editing: editing?.id === node.cat.id,
                'dragging': dragIds.has(node.cat.id),
                'drop-child': dropTarget?.id === node.cat.id && dropTarget.mode === 'child',
                'drop-before': dropTarget?.id === node.cat.id && dropTarget.mode === 'before',
                'drop-after': dropTarget?.id === node.cat.id && dropTarget.mode === 'after',
                'drop-invalid': invalidTargetId === node.cat.id,
                'sub-level': node.depth > 0,
              }"
              :data-cat-id="node.cat.id"
              :style="node.depth > 0 ? { marginLeft: (node.depth * 20) + 'px' } : undefined"
              @mousedown="onRowMouseDown($event, node.cat.id)"
              @click="onRowClick($event, node.cat)"
            >
              <input
                type="checkbox"
                class="cat-checkbox"
                :checked="selectedCatIds.has(node.cat.id)"
                :title="t('cat.manager.checkbox')"
                @click.stop="toggleSelect(node.cat.id)"
              />
              <span class="drag-handle">⠿</span>
              <span v-if="node.depth > 0" class="cat-tree-prefix">└</span>
              <CategoryIcon :category="node.cat" class="cat-icon-slot" />
              <span class="cat-name" @click.stop="onCatNameClick(node.cat)">{{ node.cat.name }}</span>
              <span v-if="node.cat.hidden" class="cat-hidden-badge" :title="t('cat.manager.hiddenBadgeTitle')">
                {{ t('cat.manager.hiddenBadge') }}
              </span>
              <span class="cat-view-mode">{{ t('cat.manager.viewMode.' + node.cat.view_mode) }}</span>
              <div class="cat-row-actions">
                <button class="btn btn-icon" :title="t('cat.manager.addSub')" @click.stop="startNew(node.cat.id)">+</button>
                <button class="btn btn-icon btn-danger" :title="t('common.delete')" @click.stop="remove(node.cat.id)">🗑</button>
              </div>
            </div>
            <!-- 编辑面板：编辑/新建子分类时内联在当前分类下方 -->
            <CategoryEditPanel
              v-if="showPanelFor(node.cat.id)"
              :editing="editing"
              :is-new="isNew"
              :form="form"
              :pending-icon="form.icon"
              :categories="state.categories"
              v-model:sync-children="syncChildren"
              @save="save"
              @cancel="cancelEdit"
              @set-emoji="setEmoji"
              @upload-icon="uploadIcon"
            />
          </template>
        </div>

        <button class="btn btn-primary cat-add-top" @click="startNew(null)">+ {{ t('cat.manager.newTop') }}</button>

        <!-- 底部编辑面板：新建一级分类时在此显示 -->
        <div v-if="showPanelAtBottom" data-panel-bottom>
          <CategoryEditPanel
            :editing="editing"
            :is-new="isNew"
            :form="form"
            :pending-icon="form.icon"
            :categories="state.categories"
            v-model:sync-children="syncChildren"
            @save="save"
            @cancel="cancelEdit"
            @set-emoji="setEmoji"
            @upload-icon="uploadIcon"
          />
        </div>
      </div>
    </div>

    <DeleteCategoryDialog
      v-if="deleteTargetId"
      :category-id="deleteTargetId"
      @close="deleteTargetId = null"
      @deleted="handleDeleted"
    />

    <!-- 拖拽幽灵卡片：跟随光标，多选时显示数量（pointer-events: none 不挡落点检测） -->
    <Teleport to="body">
      <div v-show="ghostVisible" ref="ghostRef" class="cat-drag-ghost">
        <template v-if="!dragGhost.multi">
          <span class="ghost-icon">
            <img v-if="dragGhost.icon?.img" :src="dragGhost.icon.img" alt="" />
            <template v-else>{{ dragGhost.icon?.emoji ?? '📁' }}</template>
          </span>
          <span class="ghost-name">{{ dragGhost.name }}</span>
        </template>
        <template v-else>
          <span class="ghost-icon">📑</span>
          <span class="ghost-name">{{ t('cat.manager.dragMulti', { count: dragGhost.count }) }}</span>
        </template>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.cat-manager-dialog {
  width: 560px;
}

.drag-hint {
  font-size: 11px;
  color: var(--color-text-tertiary);
  margin-bottom: 8px;
  padding: 4px 8px;
  background: var(--color-bg-sidebar);
  border-radius: var(--radius-sm);
  line-height: 1.5;
}

/* ─── 批量操作工具栏 ─── */
.cat-batch-bar {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
  padding: 8px 10px;
  margin-bottom: 10px;
  background: var(--color-primary-bg);
  border: 1px solid var(--color-primary);
  border-radius: var(--radius-sm);
  font-size: 12px;
}

.cat-batch-bar .batch-count {
  font-weight: 600;
  color: var(--color-primary-dark);
  margin-right: 2px;
}

.cat-batch-bar .select {
  width: auto;
  min-width: 110px;
  padding: 4px 8px;
  font-size: 12px;
}

.cat-batch-bar .batch-sep {
  width: 1px;
  height: 16px;
  background: var(--color-border);
}

.cat-delete-confirm {
  background: var(--color-danger-bg);
  border-color: var(--color-danger);
}

.cat-delete-confirm .batch-count {
  color: var(--color-danger);
}

.cat-delete-confirm .delete-warn {
  color: var(--color-text-secondary);
  font-size: 11px;
}

.cat-tree {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin-bottom: 12px;
}

.cat-row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 12px;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm);
  transition: border-color 0.15s, background 0.15s, opacity 0.15s;
  user-select: none;
  -webkit-user-select: none;
  cursor: pointer;
  position: relative;
}

.cat-row:hover {
  background: var(--color-bg-hover);
}

.cat-row.editing {
  border-color: var(--color-primary);
  background: var(--color-primary-bg);
}

.cat-row.sub-level {
  border-style: dashed;
}

/* 多选选中态 */
.cat-row.selected {
  border-color: var(--color-primary);
  background: var(--color-primary-bg);
}

/* ─── 勾选框：悬停或选中时显示 ─── */
.cat-checkbox {
  width: 15px;
  height: 15px;
  accent-color: var(--color-primary);
  cursor: pointer;
  flex-shrink: 0;
  opacity: 0;
  transition: opacity 0.15s;
}

.cat-row:hover .cat-checkbox,
.cat-row.selected .cat-checkbox {
  opacity: 1;
}

.cat-tree-prefix {
  color: var(--color-text-tertiary);
  font-size: 12px;
  flex-shrink: 0;
  margin-left: -4px;
}

/* ─── 拖拽视觉反馈 ─── */

/* 被拖走的行：半透明占位 */
.cat-row.dragging {
  opacity: 0.35;
}

/* 落点为子分类：整行高亮 */
.cat-row.drop-child {
  border-color: var(--color-primary);
  border-style: solid;
  background: var(--color-primary-bg);
  box-shadow: 0 0 0 2px var(--color-primary-light);
}

/* 落点为排序：行上/下边缘显示插入线 */
.cat-row.drop-before::before,
.cat-row.drop-after::after {
  content: '';
  position: absolute;
  left: 0;
  right: 0;
  height: 2px;
  border-radius: 1px;
  background: var(--color-primary);
  box-shadow: 0 0 6px var(--color-primary-light);
}

.cat-row.drop-before::before {
  top: -3px;
}

.cat-row.drop-after::after {
  bottom: -3px;
}

/* 无效落点（自身或子孙）：红色警示，不给"能放下"的错觉 */
.cat-row.drop-invalid {
  border-color: var(--color-danger);
  border-style: solid;
  background: var(--color-danger-bg);
}

.drag-handle {
  cursor: grab;
  color: var(--color-text-tertiary);
  font-size: 14px;
  flex-shrink: 0;
  user-select: none;
}

.drag-handle:active {
  cursor: grabbing;
}

.cat-icon-slot {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  flex-shrink: 0;
  font-size: 15px;
}

.cat-name {
  flex: 1;
  cursor: pointer;
  font-size: 13px;
  user-select: none;
  -webkit-user-select: none;
}

/* 名称可点击编辑的暗示 */
.cat-name:hover {
  color: var(--color-primary);
  text-decoration: underline;
}

.cat-hidden-badge {
  font-size: 10px;
  color: #b45309;
  background: rgba(245, 158, 11, 0.15);
  padding: 2px 6px;
  border-radius: 8px;
  flex-shrink: 0;
}

.cat-view-mode {
  font-size: 10px;
  color: var(--color-text-tertiary);
  background: var(--color-bg-sidebar);
  padding: 2px 6px;
  border-radius: 8px;
}

.cat-row-actions {
  display: flex;
  gap: 2px;
}

.cat-add-top {
  width: 100%;
  justify-content: center;
  margin-bottom: 16px;
}
</style>

<style>
/* 拖拽中的全局状态：抓取光标 + 禁选（scoped 无法作用于 body，放全局） */
body.cat-dragging,
body.cat-dragging * {
  cursor: grabbing !important;
  user-select: none !important;
}

/* 拖拽幽灵卡片（Teleport 到 body，需非 scoped） */
.cat-drag-ghost {
  position: fixed;
  top: 0;
  left: 0;
  z-index: 1200;
  display: flex;
  align-items: center;
  gap: 8px;
  max-width: 220px;
  padding: 6px 12px;
  background: var(--color-bg-card);
  border: 1px solid var(--color-primary);
  border-radius: var(--radius-md);
  box-shadow: 0 8px 24px var(--color-shadow-strong);
  font-size: 12.5px;
  color: var(--color-text);
  pointer-events: none;
}

.cat-drag-ghost .ghost-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cat-drag-ghost .ghost-icon {
  font-size: 14px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
}

.cat-drag-ghost .ghost-icon img {
  width: 16px;
  height: 16px;
  object-fit: cover;
  border-radius: 3px;
}
</style>
