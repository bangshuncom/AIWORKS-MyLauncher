<script setup lang="ts">
import { computed, ref } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { state, bumpIconVersion } from '@/store'
import * as api from '@/api'
import type { BatchIconProgressEvent } from '@/types'

const emit = defineEmits<{
  (e: 'close'): void
}>()

const props = defineProps<{
  /** 强制重置：忽略本地已有图标与缓存，全部重新下载/提取 */
  force?: boolean
}>()

/** 批量模式选中的条目 */
const entries = state.entries.filter(e => state.selectedIds.has(e.id))

/** 初始状态：全部「待处理」 */
type RowStatus = 'pending' | 'success' | 'skipped' | 'failed'
/** 展示行（status 比 BatchIconItemResult 多一个 pending 态） */
interface Row {
  entry_id: string
  entry_name: string
  entry_type: string
  status: RowStatus
  message: string
  icon_path: string
}

const rows = ref<Row[]>(entries.map(e => ({
  entry_id: e.id,
  entry_name: e.name,
  entry_type: e.type,
  status: 'pending',
  message: '',
  icon_path: '',
})))

const running = ref(true)
const done = ref(0)
const total = rows.value.length
const percent = computed(() => total > 0 ? Math.round(done.value / total * 100) : 0)
const successCount = computed(() => rows.value.filter(r => r.status === 'success').length)
const skippedCount = computed(() => rows.value.filter(r => r.status === 'skipped').length)
const failedCount = computed(() => rows.value.filter(r => r.status === 'failed').length)

const statusText: Record<RowStatus, string> = {
  pending: '待处理',
  success: '成功',
  skipped: '跳过',
  failed: '失败',
}

/** 处理完成的行自动滚动到可视区 */
const listEl = ref<HTMLElement | null>(null)
function scrollToRow(idx: number) {
  requestAnimationFrame(() => {
    const el = listEl.value?.children[idx] as HTMLElement | undefined
    el?.scrollIntoView({ block: 'nearest' })
  })
}

/** 将成功结果写回 store 条目图标（favicon 也存于 assets 内的文件资源，type 统一记 extracted） */
async function applyResults() {
  let changed = false
  for (const r of rows.value) {
    if (r.status !== 'success') continue
    const idx = state.entries.findIndex(e => e.id === r.entry_id)
    if (idx < 0) continue
    state.entries[idx].icon = r.icon_path
      ? { type: 'extracted', source: r.icon_path }
      : { type: 'default', source: '' }
    // bump 图标版本：URL 拼上 ?v= 绕过 WebView 缓存，列表图标立即更新
    bumpIconVersion(r.icon_path)
    changed = true
  }
  if (changed) await api.save_entries(state.entries)
}

/** 启动批量任务并监听进度事件 */
async function start() {
  const payload = entries.map(e => ({ ...e }))
  let unlisten: UnlistenFn | null = null
  try {
    unlisten = await listen<BatchIconProgressEvent>('batch-icon-progress', event => {
      const p = event.payload
      const idx = rows.value.findIndex(r => r.entry_id === p.entry_id)
      if (idx >= 0) {
        rows.value[idx].status = p.status
        rows.value[idx].message = p.message
        rows.value[idx].icon_path = p.icon_path
        scrollToRow(idx)
      }
      done.value = p.done
    })

    const summary = await api.batch_refresh_icons(payload, state.currentEnvId, props.force === true)

    // 事件推送可能有乱序/遗漏，最终以命令返回值为准整体校正一次
    for (const r of summary.results) {
      const idx = rows.value.findIndex(row => row.entry_id === r.entry_id)
      if (idx >= 0) {
        rows.value[idx].status = r.status
        rows.value[idx].message = r.message
        rows.value[idx].icon_path = r.icon_path
      }
    }
    done.value = summary.total

    // 按返回结果写回 store 条目图标并保存
    await applyResults()
  } catch (e) {
    console.error('批量重置图标失败', e)
    rows.value.forEach(r => {
      if (r.status === 'pending') r.status = 'failed'
      if (r.status === 'failed' && !r.message) r.message = '任务中断或失败'
    })
  } finally {
    unlisten?.()
    running.value = false
  }
}

start()

const statusFilter = ref<'all' | RowStatus>('all')
const filteredRows = computed(() =>
  statusFilter.value === 'all' ? rows.value : rows.value.filter(r => r.status === statusFilter.value)
)

function close() {
  if (running.value) {
    // 运行中不允许关闭：后端线程池无法安全中止，关闭窗口只会让结果无处展示
    return
  }
  emit('close')
}

// ─── 展示辅助 ───

const typeIcon: Record<string, string> = {
  program: '🪟',
  url: '🌐',
  folder: '📁',
  file: '📄',
  appx: '🧩',
  system: '⚙️',
}
function typeOf(t: string): string {
  return typeIcon[t] || '❓'
}
function statusClass(s: RowStatus): string {
  return `status-${s}`
}
</script>

<template>
  <div class="dialog-overlay" @click.self="close">
    <div class="dialog batch-icon-dialog">
      <div class="dialog-header">
        <span>批量重置图标{{ props.force ? '（强制）' : '' }}</span>
        <button class="btn btn-icon" :disabled="running" @click="close">✕</button>
      </div>

      <div class="dialog-body">
        <!-- 总进度 -->
        <div class="progress-summary">
          <div class="progress-bar-outer">
            <div class="progress-bar-inner" :style="{ width: percent + '%' }"></div>
          </div>
          <span class="progress-text">{{ running ? '处理中' : '已完成' }} {{ done }}/{{ total }}（{{ percent }}%）</span>
        </div>
        <div class="progress-stats">
          <span class="stat stat-success">成功 {{ successCount }}</span>
          <span class="stat stat-skipped">跳过 {{ skippedCount }}</span>
          <span class="stat stat-failed">失败 {{ failedCount }}</span>
        </div>

        <!-- 结果列表 -->
        <div ref="listEl" class="result-list">
          <div v-for="(r, i) in filteredRows" :key="r.entry_id" class="result-item">
            <span class="result-index">{{ i + 1 }}</span>
            <span class="result-type">{{ typeOf(r.entry_type) }}</span>
            <div class="result-info">
              <span class="result-name">{{ r.entry_name }}</span>
              <span v-if="r.message" class="result-message" :title="r.message">{{ r.message }}</span>
            </div>
            <span class="result-status" :class="statusClass(r.status)">{{ statusText[r.status] }}</span>
          </div>
        </div>

        <!-- 筛选 -->
        <div class="filter-row">
          <label class="filter-label">筛选：</label>
          <select v-model="statusFilter" class="select">
            <option value="all">全部（{{ total }}）</option>
            <option value="success">成功（{{ successCount }}）</option>
            <option value="skipped">跳过（{{ skippedCount }}）</option>
            <option value="failed">失败（{{ failedCount }}）</option>
          </select>
        </div>
      </div>

      <div class="dialog-footer">
        <button class="btn btn-primary" :disabled="running" @click="close">
          {{ running ? '处理中...' : '完成' }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.batch-icon-dialog {
  width: 620px;
  max-height: 80vh;
}

.progress-summary {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 8px;
}

.progress-bar-outer {
  flex: 1;
  height: 8px;
  background: var(--color-border-light);
  border-radius: 4px;
  overflow: hidden;
}

.progress-bar-inner {
  height: 100%;
  background: var(--color-primary);
  border-radius: 4px;
  transition: width 0.2s ease;
}

.progress-text {
  font-size: 12px;
  color: var(--color-text-secondary);
  white-space: nowrap;
}

.progress-stats {
  display: flex;
  gap: 12px;
  margin-bottom: 12px;
}

.stat {
  font-size: 12px;
  padding: 2px 8px;
  border-radius: 8px;
}

.stat-success {
  color: var(--color-success);
  background: rgba(16, 124, 16, 0.08);
}

.stat-skipped {
  color: var(--color-warning);
  background: rgba(247, 99, 12, 0.08);
}

.stat-failed {
  color: var(--color-danger);
  background: var(--color-danger-bg);
}

.result-list {
  max-height: 340px;
  overflow-y: auto;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm);
}

.result-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  border-bottom: 1px solid var(--color-border-light);
}

.result-item:last-child {
  border-bottom: none;
}

.result-index {
  width: 32px;
  font-size: 11px;
  color: var(--color-text-tertiary);
  text-align: right;
  flex-shrink: 0;
}

.result-type {
  font-size: 13px;
  flex-shrink: 0;
}

.result-info {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
  overflow: hidden;
}

.result-name {
  font-size: 13px;
  font-weight: 500;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.result-message {
  font-size: 11px;
  color: var(--color-text-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.result-status {
  font-size: 11px;
  padding: 2px 8px;
  border-radius: 8px;
  flex-shrink: 0;
}

.status-pending {
  color: var(--color-text-secondary);
  background: var(--color-bg-hover);
}

.status-success {
  color: var(--color-success);
  background: rgba(16, 124, 16, 0.08);
}

.status-skipped {
  color: var(--color-warning);
  background: rgba(247, 99, 12, 0.08);
}

.status-failed {
  color: var(--color-danger);
  background: var(--color-danger-bg);
}

.filter-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 12px;
}

.filter-label {
  font-size: 12px;
  color: var(--color-text-secondary);
  flex-shrink: 0;
}

.filter-row .select {
  flex: 1;
}
</style>
