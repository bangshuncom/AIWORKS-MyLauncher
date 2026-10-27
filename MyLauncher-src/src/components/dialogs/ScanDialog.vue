<script setup lang="ts">
import { ref, computed, onBeforeUnmount } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { state, appendImportedEntries } from '@/store'
import * as api from '@/api'
import type { ExeInfo, ImportProgressEvent } from '@/types'
import { onImgError } from '@/composables/useImageFallback'

const emit = defineEmits<{
  (e: 'close'): void
  (e: 'scanned'): void
}>()

const scanPath = ref('')
const scanning = ref(false)
const scanResults = ref<ExeInfo[]>([])
const scanWarnings = ref<string[]>([])
const selectedIds = ref<Set<string>>(new Set())
const targetCategoryId = ref(state.currentCategoryId || 'cat_001')

// ─── 扫描进度可视化 ───
interface ScanProgressPayload {
  phase: 'walking' | 'icons'
  found: number
  done: number
  total: number
  /** 图标提取阶段：当前正在处理的程序/游戏名 */
  current?: string
}

const progressVisible = ref(false)
const progressPhase = ref<'walking' | 'icons'>('walking')
const progressFound = ref(0)
const progressDone = ref(0)
const progressTotal = ref(0)
const progressCurrent = ref('')
let progressUnlisten: UnlistenFn | null = null

// ─── 导入进度可视化 ───

const importing = ref(false)
const importVisible = ref(false)
const importDone = ref(0)
const importTotal = ref(0)
const importCurrent = ref('')
const importStatus = ref<'success' | 'failed'>('success')
const importMessage = ref('')
let importUnlisten: UnlistenFn | null = null

/** 导入进度条百分比 */
const importPercent = computed(() =>
  importTotal.value > 0 ? Math.round(importDone.value / importTotal.value * 100) : 0
)

/** 导入当前条目提示文案 */
const importText = computed(() =>
  `正在处理 ${importDone.value}/${importTotal.value}：${importCurrent.value || '…'}`
)

/** 进度条百分比：walking 阶段为不确定（动画），icons 阶段按 done/total */
const progressPercent = computed(() => {
  if (progressPhase.value === 'walking') return 0
  return progressTotal.value > 0 ? Math.round(progressDone.value / progressTotal.value * 100) : 0
})

/** 当前阶段提示文案（用户可视化正在执行的操作） */
const progressText = computed(() => {
  if (progressPhase.value === 'walking') {
    return `正在扫描目录，已发现 ${progressFound.value} 个程序…`
  }
  return `正在解析结果 ${progressDone.value}/${progressTotal.value}：${progressCurrent.value || '…'}`
})

/** 进度弹层图标（阶段区分） */
const progressIcon = computed(() => progressPhase.value === 'walking' ? '🔍' : '🎨')

/** 分类选项：按层级树显示（与条目编辑器统一的多层级缩进样式） */
const categoryOptions = computed(() => {
  const result: { id: string; name: string; depth: number }[] = []
  const visit = (parentId: string | null, depth: number) => {
    state.categories
      .filter(c => c.parent_id === parentId)
      .sort((a, b) => a.sort_order - b.sort_order)
      .forEach(c => {
        result.push({ id: c.id, name: c.name, depth })
        visit(c.id, depth + 1)
      })
  }
  visit(null, 0)
  return result
})

async function pickDir() {
  const path = await api.pick_directory()
  if (path) scanPath.value = path
}

async function scan() {
  if (!scanPath.value) return
  scanning.value = true
  scanResults.value = []
  scanWarnings.value = []
  selectedIds.value.clear()

  // 显示进度弹层并监听后端 scan-progress 事件
  progressVisible.value = true
  progressPhase.value = 'walking'
  progressFound.value = 0
  progressDone.value = 0
  progressTotal.value = 0
  progressCurrent.value = ''
  try {
    progressUnlisten = await listen<ScanProgressPayload>('scan-progress', event => {
      const p = event.payload
      progressPhase.value = p.phase
      progressFound.value = p.found
      progressDone.value = p.done
      progressTotal.value = p.total
      progressCurrent.value = p.current || ''
    })

    const result = await api.scan_directory(scanPath.value, 3)
    scanResults.value = result.results
    scanWarnings.value = result.warnings || []
    // 默认选中策略：多 exe 条目只选中主程序（红色提示用户自行勾选其余），其余全选
    result.results.forEach(r => {
      if (r.multi_exe && !r.is_primary) return
      const id = `${r.name}_${r.absolute_path}`
      selectedIds.value.add(id)
    })
  } catch (e) {
    console.error('扫描失败', e)
  } finally {
    progressVisible.value = false
    progressUnlisten?.()
    progressUnlisten = null
    scanning.value = false
  }
}

onBeforeUnmount(() => {
  progressUnlisten?.()
  importUnlisten?.()
})

function getId(r: ExeInfo): string {
  return `${r.name}_${r.absolute_path}`
}

function toggleSelect(id: string) {
  if (selectedIds.value.has(id)) {
    selectedIds.value.delete(id)
  } else {
    selectedIds.value.add(id)
  }
}

function selectAll() {
  scanResults.value.forEach(r => selectedIds.value.add(getId(r)))
}

function invertSelection() {
  scanResults.value.forEach(r => {
    const id = getId(r)
    if (selectedIds.value.has(id)) {
      selectedIds.value.delete(id)
    } else {
      selectedIds.value.add(id)
    }
  })
}

async function importSelected() {
  if (importing.value) return
  const items = scanResults.value.filter(r => selectedIds.value.has(getId(r)))
  if (items.length === 0) return

  importing.value = true
  importVisible.value = true
  importDone.value = 0
  importTotal.value = items.length
  importCurrent.value = ''
  importStatus.value = 'success'
  importMessage.value = ''
  try {
    // 监听后端逐条导入进度（图标提取/封面下载/写盘）
    importUnlisten = await listen<ImportProgressEvent>('import-progress', event => {
      const p = event.payload
      importDone.value = p.done
      importTotal.value = p.total
      importCurrent.value = p.name
      importStatus.value = p.status
      importMessage.value = p.message || ''
    })

    // 后端多线程导入：提取图标/下载封面/构造 Entry/统一写盘，返回已写盘的新条目
    const summary = await api.import_scan_items(items, targetCategoryId.value, state.currentEnvId)
    // 同步新条目到内存状态（后端已写盘并分配 id）
    appendImportedEntries(summary.entries)
  } catch (e) {
    console.error('导入失败', e)
  } finally {
    importVisible.value = false
    importUnlisten?.()
    importUnlisten = null
    importing.value = false
    emit('scanned')
    emit('close')
  }
}
</script>

<template>
  <div class="dialog-overlay" @click.self="$emit('close')">
    <div class="dialog scan-dialog">
      <div class="dialog-header">
        <span>扫描文件夹</span>
        <button class="btn btn-icon" @click="$emit('close')">✕</button>
      </div>

      <div class="dialog-body">
        <!-- 扫描路径选择 -->
        <div class="scan-input-row">
          <input v-model="scanPath" class="input" placeholder="选择要扫描的文件夹" />
          <button class="btn" @click="pickDir">浏览...</button>
          <button class="btn btn-primary" @click="scan" :disabled="scanning || !scanPath">
            {{ scanning ? '扫描中...' : '扫描' }}
          </button>
        </div>

        <!-- 扫描警告（插件超时/目录不可访问等，不影响其余结果） -->
        <div v-if="scanWarnings.length > 0" class="scan-warnings">
          <div v-for="(w, i) in scanWarnings" :key="i" class="scan-warning-item">⚠ {{ w }}</div>
        </div>

        <!-- 扫描结果 -->
        <div v-if="scanResults.length > 0" class="scan-results">
          <div class="scan-results-toolbar">
            <button class="btn" @click="selectAll">全选</button>
            <button class="btn" @click="invertSelection">反选</button>
            <span class="selected-count">已选 {{ selectedIds.size }}/{{ scanResults.length }} 项</span>
          </div>

          <div class="scan-results-list">
            <div
              v-for="r in scanResults"
              :key="getId(r)"
              class="scan-item"
              :class="{ selected: selectedIds.has(getId(r)), 'multi-exe': r.multi_exe }"
              @click="toggleSelect(getId(r))"
            >
              <input type="checkbox" :checked="selectedIds.has(getId(r))" @click.stop="toggleSelect(getId(r))" />
              <img v-if="r.icon_path" :src="r.icon_path" class="scan-item-icon" alt="icon" @error="onImgError" />
              <div v-else class="scan-item-icon-placeholder">{{ r.name.charAt(0) }}</div>
              <div class="scan-item-info">
                <span class="scan-item-name">{{ r.name }}</span>
                <span class="scan-item-path">{{ r.absolute_path }}</span>
                <span v-if="r.multi_exe" class="scan-item-warn">
                  {{ r.is_primary ? '发现多个可执行程序，此为主程序' : '非主程序，按需勾选导入' }}
                </span>
              </div>
              <span v-if="r.provider" class="scan-item-mode provider-badge">{{ r.provider.id }}</span>
              <span v-else class="scan-item-mode">{{ r.suggested_path_mode === 'relative' ? '相对' : '绝对' }}</span>
            </div>
          </div>

          <!-- 导入目标分类 -->
          <div class="scan-import-row">
            <label class="form-label">导入到分类</label>
            <select v-model="targetCategoryId" class="select">
              <option v-for="cat in categoryOptions" :key="cat.id" :value="cat.id">
                {{ '　'.repeat(cat.depth) }}{{ cat.depth > 0 ? '└ ' : '' }}{{ cat.name }}
              </option>
            </select>
          </div>
        </div>

        <!-- 空状态 -->
        <div v-else-if="!scanning" class="empty-state">
          <div class="empty-icon">📁</div>
          <div class="empty-text">选择文件夹后点击扫描</div>
        </div>
      </div>

      <div class="dialog-footer" v-if="scanResults.length > 0">
        <button class="btn" @click="$emit('close')" :disabled="importing">取消</button>
        <button class="btn btn-primary" @click="importSelected" :disabled="selectedIds.size === 0 || importing">
          {{ importing ? '导入中…' : `导入 ${selectedIds.size} 项` }}
        </button>
      </div>

      <!-- 扫描进度弹层：让用户可视化正在执行的操作 -->
      <div v-if="progressVisible" class="scan-progress-overlay">
        <div class="scan-progress-card">
          <div class="scan-progress-icon" :class="{ spinning: progressPhase === 'walking' }">{{ progressIcon }}</div>
          <div class="scan-progress-title">
            {{ progressPhase === 'walking' ? '正在扫描文件夹' : '正在解析扫描结果' }}
          </div>
          <div class="scan-progress-text">{{ progressText }}</div>
          <div class="scan-progress-bar-outer">
            <div
              class="scan-progress-bar-inner"
              :class="{ indeterminate: progressPhase === 'walking' }"
              :style="progressPhase === 'walking' ? undefined : { width: progressPercent + '%' }"
            ></div>
          </div>
          <div class="scan-progress-hint">扫描目录越大耗时越长，请稍候…</div>
        </div>
      </div>

      <!-- 导入进度弹层：图标提取/封面下载/写盘可视化 -->
      <div v-if="importVisible" class="scan-progress-overlay">
        <div class="scan-progress-card">
          <div class="scan-progress-icon import-spin">⬇</div>
          <div class="scan-progress-title">正在导入扫描结果</div>
          <div class="scan-progress-text">{{ importText }}</div>
          <div class="scan-progress-text import-status" :class="{ 'status-failed': importStatus === 'failed' }">
            {{ importMessage || '正在提取图标与封面…' }}
          </div>
          <div class="scan-progress-bar-outer">
            <div class="scan-progress-bar-inner" :style="{ width: importPercent + '%' }"></div>
          </div>
          <div class="scan-progress-hint">正在提取图标与下载封面（多线程处理），请稍候…</div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.scan-dialog {
  width: 640px;
  max-height: 80vh;
  position: relative;
}

.scan-input-row {
  display: flex;
  gap: 4px;
  margin-bottom: 16px;
}

.scan-input-row .input {
  flex: 1;
}

.scan-results {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.scan-results-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
}

.selected-count {
  font-size: 12px;
  color: var(--color-text-secondary);
  margin-left: auto;
}

.scan-results-list {
  max-height: 360px;
  overflow-y: auto;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm);
}

.scan-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  border-bottom: 1px solid var(--color-border-light);
  cursor: pointer;
  transition: background var(--transition);
}

.scan-item:hover {
  background: var(--color-bg-hover);
}

.scan-item.selected {
  background: var(--color-primary-bg);
}

.scan-item-icon {
  width: 24px;
  height: 24px;
  object-fit: contain;
}

/* Steam 封面缩略图：横版 header 图，等比缩放到 40x19 区域 */
.scan-item-cover {
  width: 40px;
  height: 19px;
  object-fit: cover;
  border-radius: 3px;
  flex-shrink: 0;
}

/* 平台条目徽标（通用深色底，无平台特判） */
.scan-item-mode.provider-badge {
  color: #fff;
  background: #1b2838;
  font-weight: 600;
}

/* 扫描警告区 */
.scan-warnings {
  margin-bottom: 8px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.scan-warning-item {
  font-size: 12px;
  color: var(--color-danger);
  background: var(--color-danger-bg);
  border-radius: var(--radius-sm);
  padding: 6px 10px;
}

.scan-item-icon-placeholder {
  width: 24px;
  height: 24px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--color-bg-sidebar);
  border-radius: 4px;
  font-size: 12px;
}

.scan-item-info {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
  overflow: hidden;
}

.scan-item-name {
  font-size: 13px;
  font-weight: 500;
}

.scan-item-path {
  font-size: 11px;
  color: var(--color-text-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.scan-item-mode {
  font-size: 10px;
  color: var(--color-primary-dark);
  background: var(--color-primary-bg);
  padding: 2px 6px;
  border-radius: 8px;
  flex-shrink: 0;
}

/* 多 exe Steam 游戏条目：左侧红色竖线标记，提示用户需要选择 */
.scan-item.multi-exe {
  border-left: 3px solid var(--color-danger);
  padding-left: 9px;
}

/* 红色提示文案（主程序/非主程序区分） */
.scan-item-warn {
  font-size: 11px;
  color: var(--color-danger);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* 导入进度图标动画 */
.scan-progress-icon.import-spin {
  animation: import-pulse 0.9s ease-in-out infinite;
}

@keyframes import-pulse {
  0%, 100% { transform: translateY(0); }
  50% { transform: translateY(4px); }
}

/* 导入状态文案：失败时红色强调 */
.import-status {
  color: var(--color-text-secondary);
}

.import-status.status-failed {
  color: var(--color-danger);
  font-weight: 500;
}

.scan-import-row {
  display: flex;
  align-items: center;
  gap: 12px;
}

/* ScanDialog 的 form-label 比 70px 更宽 */
.form-label {
  width: 80px;
}

.scan-import-row .select {
  flex: 1;
}

/* ─── 扫描进度弹层 ─── */
.scan-progress-overlay {
  position: absolute;
  inset: 0;
  background: rgba(255, 255, 255, 0.82);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 20;
  border-radius: var(--radius-lg);
  backdrop-filter: blur(2px);
}

html.dark .scan-progress-overlay {
  background: rgba(32, 32, 32, 0.82);
}

.scan-progress-card {
  width: 320px;
  background: var(--color-bg-card);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg);
  box-shadow: var(--color-shadow-strong);
  padding: 24px 24px 18px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
}

.scan-progress-icon {
  font-size: 34px;
  line-height: 1;
}

.scan-progress-icon.spinning {
  animation: scan-bounce 1s ease-in-out infinite;
}

@keyframes scan-bounce {
  0%, 100% { transform: scale(1); }
  50% { transform: scale(1.18); }
}

.scan-progress-title {
  font-size: 15px;
  font-weight: 600;
}

.scan-progress-text {
  font-size: 12px;
  color: var(--color-text-secondary);
  max-width: 280px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.scan-progress-bar-outer {
  width: 100%;
  height: 8px;
  background: var(--color-border-light);
  border-radius: 4px;
  overflow: hidden;
}

.scan-progress-bar-inner {
  height: 100%;
  background: var(--color-primary);
  border-radius: 4px;
  transition: width 0.25s ease;
}

/* walking 阶段：不确定进度条动画 */
.scan-progress-bar-inner.indeterminate {
  width: 35% !important;
  animation: scan-indeterminate 1.2s ease-in-out infinite;
}

@keyframes scan-indeterminate {
  0% { margin-left: -35%; }
  100% { margin-left: 100%; }
}

.scan-progress-hint {
  font-size: 11px;
  color: var(--color-text-tertiary);
}
</style>
