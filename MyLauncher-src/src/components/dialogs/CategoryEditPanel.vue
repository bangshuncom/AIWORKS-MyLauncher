<script setup lang="ts">
import { computed } from 'vue'
import type { Category, ViewMode } from '@/types'
import IconPicker from '@/components/IconPicker.vue'
import { state } from '@/store'
import { useI18n } from '@/composables/useI18n'

const { t } = useI18n()

const props = defineProps<{
  editing: Category | null
  isNew: boolean
  form: { name: string; parent_id: string | null; view_mode: ViewMode; hidden: boolean }
  categories: Category[]
  pendingIcon: { type: 'emoji' | 'custom'; source: string } | null
}>()

// 统一获取当前图标状态：编辑已有分类用 editing.icon，新建用 pendingIcon
const currentIcon = computed(() => {
  if (props.editing?.icon) return props.editing.icon
  if (props.pendingIcon) return props.pendingIcon
  return null
})
const currentIconType = computed(() => currentIcon.value?.type ?? '')
const currentIconSource = computed(() => currentIcon.value?.source ?? '')

// 构建层级分类列表，用于父分类下拉选择
// 排除当前编辑的分类及其所有子孙（防止循环引用）
const hierarchicalCategories = computed(() => {
  const excludedIds = new Set<string>()
  if (props.editing) {
    excludedIds.add(props.editing.id)
    const collectDescendants = (parentId: string) => {
      props.categories
        .filter(c => c.parent_id === parentId)
        .forEach(c => {
          excludedIds.add(c.id)
          collectDescendants(c.id)
        })
    }
    collectDescendants(props.editing.id)
  }

  const result: { id: string; name: string; depth: number }[] = []
  const visit = (parentId: string | null, depth: number) => {
    props.categories
      .filter(c => c.parent_id === parentId && !excludedIds.has(c.id))
      .sort((a, b) => a.sort_order - b.sort_order)
      .forEach(c => {
        result.push({ id: c.id, name: c.name, depth })
        visit(c.id, depth + 1)
      })
  }
  visit(null, 0)
  return result
})

const emit = defineEmits<{
  (e: 'save'): void
  (e: 'cancel'): void
  (e: 'set-emoji', emoji: string): void
  (e: 'upload-icon', relPath: string): void
}>()

const syncChildren = defineModel<boolean>('syncChildren', { default: false })
</script>

<template>
  <div class="cat-edit-panel" @click.stop>
    <!-- 图标选择器 -->
    <div class="cat-icon-picker">
      <div class="section-title">{{ t('cat.edit.icon') }}</div>
      <IconPicker
        :current-type="currentIconType"
        :current-source="currentIconSource"
        upload-dir="icons/custom"
        :custom-emojis="state.config.custom_emojis || []"
        @select-emoji="(e: string) => emit('set-emoji', e)"
        @upload="(rel: string) => emit('upload-icon', rel)"
      />
    </div>

    <!-- 编辑表单 -->
    <div class="cat-edit-form">
      <div class="section-title">{{ isNew ? t('cat.edit.new') : t('cat.edit.edit') }}</div>
      <div class="form-row">
        <label class="form-label">{{ t('cat.edit.name') }}</label>
        <input v-model="form.name" class="input" :placeholder="t('cat.edit.namePlaceholder')" />
      </div>
      <div class="form-row">
        <label class="form-label">{{ t('cat.edit.parent') }}</label>
        <select v-model="form.parent_id" class="select">
          <option :value="null">{{ t('cat.edit.parentNone') }}</option>
          <option
            v-for="cat in hierarchicalCategories"
            :key="cat.id"
            :value="cat.id"
          >{{ '  '.repeat(cat.depth) }}{{ cat.depth > 0 ? '└ ' : '' }}{{ cat.name }}</option>
        </select>
      </div>
      <div class="form-row">
        <label class="form-label">{{ t('cat.edit.viewMode') }}</label>
        <select v-model="form.view_mode" class="select">
          <option value="vertical_card">{{ t('cat.manager.viewMode.vertical_card') }}</option>
          <option value="horizontal_card">{{ t('cat.manager.viewMode.horizontal_card') }}</option>
          <option value="icon_grid">{{ t('cat.manager.viewMode.icon_grid') }}</option>
          <option value="table">{{ t('cat.manager.viewMode.table') }}</option>
          <option value="waterfall">{{ t('cat.manager.viewMode.waterfall') }}</option>
        </select>
      </div>
      <div class="form-row sync-row" v-if="!isNew">
        <label class="form-label"></label>
        <label class="sync-check">
          <input type="checkbox" v-model="syncChildren" />
          <span>{{ t('cat.edit.syncChildren') }}</span>
        </label>
      </div>
      <div class="form-row">
        <label class="form-label"></label>
        <label class="sync-check" :title="t('cat.edit.hiddenTitle')">
          <input type="checkbox" v-model="form.hidden" />
          <span>{{ t('cat.edit.hidden') }}</span>
        </label>
      </div>
      <div class="form-actions">
        <button class="btn" @click="emit('cancel')">{{ t('common.cancel') }}</button>
        <button class="btn btn-primary" @click="emit('save')">{{ t('common.save') }}</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.cat-edit-panel {
  margin: 4px 0 8px;
  padding: 12px;
  background: var(--color-bg-sidebar);
  border: 1px solid var(--color-primary);
  border-radius: var(--radius-sm);
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.08);
}

.cat-icon-picker {
  margin-bottom: 12px;
}

.cat-edit-form {
  border-top: 1px solid var(--color-border-light);
  padding-top: 12px;
}

.sync-row {
  margin-bottom: 4px;
}

.sync-check {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--color-text-secondary);
  cursor: pointer;
  padding: 4px 0;
  user-select: none;
}

.sync-check input[type='checkbox'] {
  accent-color: var(--color-primary);
  cursor: pointer;
}
</style>
