<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { state, saveSettings, resolveAssetUrl } from '@/store'
import { scan_orphan_assets, clean_orphan_assets, set_autorun, get_autorun, list_plugins, open_plugins_dir } from '@/api'
import { useToast } from '@/composables/useToast'
import { useI18n } from '@/composables/useI18n'
import { setLocale } from '@/i18n'
import { list_languages, open_lang_dir } from '@/api'
import type { LangInfo, PluginInfo } from '@/types'
import IconPicker from '@/components/IconPicker.vue'

const emit = defineEmits<{
  (e: 'close'): void
}>()

// ─── Tab 切换 ───
type TabId = 'appearance' | 'behavior' | 'advanced' | 'maintenance' | 'plugins'
const activeTab = ref<TabId>('appearance')
const tabs: { id: TabId; labelKey: string; icon: string }[] = [
  { id: 'appearance', labelKey: 'settings.tab.appearance', icon: '◑' },
  { id: 'behavior', labelKey: 'settings.tab.behavior', icon: '⚙' },
  { id: 'advanced', labelKey: 'settings.tab.advanced', icon: '⚒' },
  { id: 'maintenance', labelKey: 'settings.tab.maintenance', icon: '◐' },
  { id: 'plugins', labelKey: 'settings.tab.plugins', icon: '◧' },
]

// 表单（从当前配置初始化）
const form = ref({
  logo_icon: state.config.logo_icon || '🪷',
  logo_icon_type: (state.config.logo_icon_type === 'custom' ? 'custom' : 'emoji') as 'emoji' | 'custom',
  logo_text: state.config.logo_text || 'MyLauncher',
  logo_image: state.config.logo_image || '',
  logo_image_enabled: state.config.logo_image_enabled || false,
  close_action: (state.config.close_action === 'close' ? 'close' : 'tray') as 'tray' | 'close',
  custom_emojis: [...(state.config.custom_emojis || [])] as string[],
  // Favicon 获取源（多行文本，textarea 用字符串编辑，保存时按行拆分）
  favicon_api_sources_text: (state.config.favicon_api_sources || []).join('\n'),
  // 开机自启（保存时写入注册表）
  autorun: false,
  autorun_silent: false,
  // 竖向卡片封面宽高比例
  card_cover_ratio: state.config.card_cover_ratio || '16:9',
  // 瀑布流封面最大高度（px）
  masonry_max_height: typeof state.config.masonry_max_height === 'number' && state.config.masonry_max_height > 0 ? state.config.masonry_max_height : 480,
})

// 竖向卡片封面比例档位
const coverRatioOptions = ['16:9', '4:3', '1:1', '3:4', '9:16']

// 加载时从注册表读实际自启状态（非前端内存）
onMounted(async () => {
  try {
    const st = await get_autorun()
    form.value.autorun = st.enabled
    form.value.autorun_silent = st.silent
  } catch {
    // 读取失败保持默认关闭
  }
  // 拉取已安装语言列表（实时扫描 lang/ 目录）
  await loadLanguages()
  // 拉取插件列表
  await loadPlugins()
})

// ─── 插件面板 ───
const plugins = ref<PluginInfo[]>([])

async function loadPlugins() {
  try {
    plugins.value = await list_plugins()
  } catch {
    // 读取失败按无插件处理
  }
}

function openPluginsDir() {
  open_plugins_dir().catch(() => {})
}

/**
 * 启用/停用插件：写入 config.disabled_plugins 后立刻持久化。
 * 停用只影响「扫描目录 / 按 ID 查询 / 平台下拉」，不动插件文件与已导入条目，
 * 因此随时可逆，也不会把已有条目改坏。
 */
async function togglePlugin(p: PluginInfo) {
  const disabled = new Set(state.config.disabled_plugins || [])
  if (p.enabled) disabled.add(p.id)
  else disabled.delete(p.id)
  try {
    await saveSettings({ disabled_plugins: Array.from(disabled) })
    await loadPlugins()
    showToast(p.enabled
      ? t('settings.pluginDisabledToast', { name: p.name })
      : t('settings.pluginEnabledToast', { name: p.name }))
  } catch (e) {
    showToast(t('settings.pluginToggleFailed', { error: String(e) }))
  }
}

// Favicon 获取源默认模板（与 Rust default_favicon_api_sources 保持一致）
const DEFAULT_FAVICON_API_SOURCES = [
  'https://a.favicon.im/{url}?larger=true',
  'https://faviconsnap.com/api/favicon?url={url}',
  '{scheme}://{domain}/favicon.ico',
]

// 恢复默认获取源
function resetFaviconApiSources() {
  form.value.favicon_api_sources_text = DEFAULT_FAVICON_API_SOURCES.join('\n')
}

// 自定义 emoji 输入框
const customEmojiInput = ref('')

// 添加自定义 emoji（从输入框粘贴/输入）
function addCustomEmoji() {
  const text = customEmojiInput.value.trim()
  if (!text) return
  // 支持粘贴多个 emoji（连续粘贴也逐个添加）
  // 用 Array.from 拆分多字符 emoji（如旗帜 emoji 由多个码点组成）
  const chars = Array.from(text)
  let added = 0
  for (const ch of chars) {
    // 去重：不在内置库也不在已有自定义列表中
    if (!form.value.custom_emojis.includes(ch)) {
      form.value.custom_emojis.push(ch)
      added++
    }
  }
  if (added > 0) {
    customEmojiInput.value = ''
  } else {
    // 已存在，清空输入
    customEmojiInput.value = ''
  }
}

// 删除自定义 emoji
function removeCustomEmoji(index: number) {
  form.value.custom_emojis.splice(index, 1)
}

// 从剪贴板粘贴 emoji
async function pasteFromClipboard() {
  try {
    const text = await navigator.clipboard.readText()
    if (text) {
      customEmojiInput.value = text.trim()
      addCustomEmoji()
    }
  } catch {
    // 剪贴板权限被拒绝，用户可手动 Ctrl+V 到输入框
  }
}

const saving = ref(false)
const error = ref('')

// 选择 emoji
function selectEmoji(emoji: string) {
  form.value.logo_icon_type = 'emoji'
  form.value.logo_icon = emoji
}

// 上传图标图片
function onIconUploaded(relPath: string) {
  form.value.logo_icon_type = 'custom'
  form.value.logo_icon = relPath
}

// LOGO 图片预览 URL
const logoImagePreview = computed(() => {
  if (!form.value.logo_image) return ''
  return resolveAssetUrl(form.value.logo_image)
})

// 选择 LOGO 图片（复制到 assets/logo/）
async function pickLogoImage() {
  error.value = ''
  try {
    const { pickAndCopyImage } = await import('@/composables/useAssetImagePicker')
    const rel = await pickAndCopyImage('logo')
    if (!rel) return
    form.value.logo_image = rel
    if (!form.value.logo_image_enabled) form.value.logo_image_enabled = true
  } catch (e) {
    error.value = t('settings.logoImportFailed', { error: String(e) })
  }
}

// 移除 LOGO 图片
async function removeLogoImage() {
  form.value.logo_image = ''
  form.value.logo_image_enabled = false
}

// 保存
async function save() {
  error.value = ''
  // 启用图片但没有图片时，自动回退为不启用
  if (form.value.logo_image_enabled && !form.value.logo_image) {
    form.value.logo_image_enabled = false
  }
  // LOGO 文字为空时回退默认
  if (!form.value.logo_text.trim()) {
    form.value.logo_text = 'MyLauncher'
  }
  // LOGO 图标为空时回退默认
  if (!form.value.logo_icon.trim()) {
    form.value.logo_icon = '🪷'
    form.value.logo_icon_type = 'emoji'
  }
  // Favicon 获取源：按行拆分、去空白；全空时回落默认
  let favicon_sources = form.value.favicon_api_sources_text
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
  if (favicon_sources.length === 0) {
    favicon_sources = [...DEFAULT_FAVICON_API_SOURCES]
    form.value.favicon_api_sources_text = favicon_sources.join('\n')
  }

  saving.value = true
  try {
    // 先写自启注册表（失败则中断，避免配置显示已开但实际未生效）
    try {
      await set_autorun(form.value.autorun, form.value.autorun_silent)
    } catch (e) {
      error.value = t('settings.autorunFailed', { error: String(e) })
      saving.value = false
      return
    }
    await saveSettings({
      logo_icon: form.value.logo_icon.trim(),
      logo_icon_type: form.value.logo_icon_type,
      logo_text: form.value.logo_text.trim(),
      logo_image: form.value.logo_image,
      logo_image_enabled: form.value.logo_image_enabled,
      close_action: form.value.close_action,
      custom_emojis: form.value.custom_emojis,
      favicon_api_sources: favicon_sources,
      card_cover_ratio: form.value.card_cover_ratio,
      masonry_max_height: form.value.masonry_max_height,
    })
    emit('close')
  } catch (e) {
    error.value = t('settings.saveFailed', { error: String(e) })
  } finally {
    saving.value = false
  }
}

// ─── 过期资源清理 ───

const { showToast } = useToast()

// ─── 界面语言 ───
const { t } = useI18n()
const languages = ref<LangInfo[]>([])
const currentLangCode = ref(state.config.language || 'zh-CN')
const switchingLang = ref(false)

/** 拉取已安装语言列表（每次打开设置页/点击刷新时实时扫描） */
async function loadLanguages() {
  try {
    languages.value = await list_languages()
  } catch {
    languages.value = []
  }
}

/** 切换语言：加载新消息表 → 整体替换 → 持久化到 config.json */
async function changeLanguage(code: string) {
  if (!code || code === currentLangCode.value) return
  switchingLang.value = true
  try {
    const ok = await setLocale(code)
    if (ok) {
      await saveSettings({ language: code })
      currentLangCode.value = code
    } else {
      error.value = t('settings.language.switchFailed', { error: code })
    }
  } catch (e) {
    error.value = t('settings.language.switchFailed', { error: String(e) })
  } finally {
    switchingLang.value = false
  }
}

// 扫描/清理进行中
const cleaning = ref(false)
// 扫描结果（null 表示尚未扫描）
const scanResult = ref<{ count: number; sizeText: string } | null>(null)

// 字节数转可读大小
function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

// 扫描过期资源（无副作用）
async function scanAssets() {
  error.value = ''
  cleaning.value = true
  scanResult.value = null
  try {
    const r = await scan_orphan_assets()
    scanResult.value = { count: r.orphan_count, sizeText: formatBytes(r.orphan_bytes) }
  } catch (e) {
    error.value = t('settings.scanFailed', { error: String(e) })
  } finally {
    cleaning.value = false
  }
}

// 清理过期资源（先确认再执行，移入回收站）
async function cleanAssets() {
  if (!scanResult.value || scanResult.value.count === 0) return
  if (!confirm(t('settings.cleanConfirm', { count: scanResult.value.count, size: scanResult.value.sizeText }))) return
  error.value = ''
  cleaning.value = true
  try {
    const r = await clean_orphan_assets()
    showToast(t('settings.cleanDone', { count: r.deleted_count, size: formatBytes(r.deleted_bytes) }))
    // 清理后重新扫描显示最新状态
    const s = await scan_orphan_assets()
    scanResult.value = { count: s.orphan_count, sizeText: formatBytes(s.orphan_bytes) }
  } catch (e) {
    error.value = t('settings.cleanFailed', { error: String(e) })
  } finally {
    cleaning.value = false
  }
}

</script>

<template>
  <div class="dialog-overlay settings-overlay" @click.self="!saving && $emit('close')">
    <div class="dialog settings-dialog">
      <div class="dialog-header">
        <span>{{ t('settings.title') }}</span>
        <button class="btn btn-icon" @click="$emit('close')">✕</button>
      </div>

      <!-- Tab 导航 -->
      <div class="settings-tabs">
        <button
          v-for="tab in tabs"
          :key="tab.id"
          class="settings-tab"
          :class="{ active: activeTab === tab.id }"
          @click="activeTab = tab.id"
        >
          <span class="tab-icon">{{ tab.icon }}</span>
          <span class="tab-label">{{ t(tab.labelKey) }}</span>
        </button>
      </div>

      <div class="dialog-body">
        <!-- ══════════ 外观 ══════════ -->
        <template v-if="activeTab === 'appearance'">
          <!-- 界面语言 -->
          <div class="section-title">{{ t('settings.language') }}</div>
          <div class="setting-group">
            <div class="form-row">
              <label class="form-label">{{ t('settings.language') }}</label>
              <div class="form-control lang-row">
                <select
                  class="select"
                  :value="currentLangCode"
                  :disabled="switchingLang"
                  @change="changeLanguage(($event.target as HTMLSelectElement).value)"
                >
                  <option v-for="lang in languages" :key="lang.code" :value="lang.code" :disabled="!lang.valid">
                    {{ lang.name }}{{ lang.is_default ? `（${t('settings.language.default')}）` : '' }}{{ lang.valid ? ` · ${t('settings.language.completeness', { percent: lang.completeness })}` : t('settings.language.corrupted') }}
                  </option>
                </select>
                <button class="btn" :disabled="switchingLang" @click="loadLanguages">{{ t('settings.language.refresh') }}</button>
                <button class="btn" @click="open_lang_dir()">{{ t('settings.language.openFolder') }}</button>
              </div>
            </div>
            <div class="form-hint">{{ t('settings.language.hint') }}</div>
          </div>

          <!-- LOGO 设置 -->
          <div class="section-title">{{ t('settings.logo') }}</div>
          <div class="setting-group">
            <div class="form-row">
              <label class="form-label">{{ t('settings.logoIcon') }}</label>
              <div class="form-control">
                <IconPicker
                  :current-type="form.logo_icon_type"
                  :current-source="form.logo_icon"
                  upload-dir="icons/custom"
                  :custom-emojis="form.custom_emojis"
                  @select-emoji="selectEmoji"
                  @upload="onIconUploaded"
                />
              </div>
            </div>
            <div class="form-row">
              <label class="form-label">{{ t('settings.logoText') }}</label>
              <div class="form-control">
                <input v-model="form.logo_text" class="input" placeholder="MyLauncher" maxlength="20" />
                <div class="form-hint">{{ t('settings.logoTextHint') }}</div>
              </div>
            </div>
            <div class="form-row">
              <label class="form-label">{{ t('settings.logoImage') }}</label>
              <div class="form-control">
                <div class="logo-image-row">
                  <button class="btn" @click="pickLogoImage">📁 {{ t('settings.pickImage') }}</button>
                  <button v-if="form.logo_image" class="btn btn-danger" @click="removeLogoImage">{{ t('common.remove') }}</button>
                  <label class="checkbox-label">
                    <input v-model="form.logo_image_enabled" type="checkbox" :disabled="!form.logo_image" />
                    {{ t('settings.logoImageEnabled') }}
                  </label>
                </div>
                <div class="form-hint">{{ t('settings.logoImageHint') }}</div>
                <!-- 预览（单图铺满，模拟 LOGO 区域实际效果） -->
                <div v-if="form.logo_image" class="logo-preview" :class="{ disabled: !form.logo_image_enabled }">
                  <div class="logo-preview-box">
                    <img v-if="logoImagePreview" :src="logoImagePreview" class="logo-preview-img" alt="LOGO 预览" />
                  </div>
                  <div class="logo-preview-filename">{{ form.logo_image }}</div>
                </div>
              </div>
            </div>
          </div>

          <!-- 自定义 emoji 图标库 -->
          <div class="section-title">{{ t('settings.customEmojiLib') }}</div>
          <div class="setting-group">
            <div class="custom-emoji-hint">{{ t('settings.customEmojiHint') }}</div>
            <div class="custom-emoji-input-row">
              <input
                v-model="customEmojiInput"
                class="input"
                :placeholder="t('settings.emojiPlaceholder')"
                @keydown.enter.prevent="addCustomEmoji"
              />
              <button class="btn" @click="pasteFromClipboard">📋 {{ t('settings.paste') }}</button>
              <button class="btn btn-primary" @click="addCustomEmoji">{{ t('settings.add') }}</button>
            </div>
            <div v-if="form.custom_emojis.length > 0" class="custom-emoji-list">
              <div
                v-for="(emoji, i) in form.custom_emojis"
                :key="i"
                class="custom-emoji-item"
                @click="selectEmoji(emoji)"
              >
                <span class="custom-emoji-char">{{ emoji }}</span>
                <button class="custom-emoji-remove" @click.stop="removeCustomEmoji(i)">✕</button>
              </div>
            </div>
            <div v-else class="custom-emoji-empty">{{ t('settings.noCustomEmoji') }}</div>
          </div>

          <!-- 列表展示 -->
          <div class="section-title">{{ t('settings.listDisplay') }}</div>
          <div class="setting-group">
            <div class="form-row">
              <label class="form-label">{{ t('settings.coverRatio') }}</label>
              <select v-model="form.card_cover_ratio" class="select">
                <option v-for="r in coverRatioOptions" :key="r" :value="r">{{ r }}</option>
              </select>
            </div>
            <div class="form-row">
              <label class="form-label">{{ t('settings.masonryMaxHeight') }}</label>
              <input v-model.number="form.masonry_max_height" type="number" min="120" max="960" step="10" class="input" />
            </div>
            <div class="form-hint">{{ t('settings.listDisplayHint') }}</div>
          </div>
        </template>

        <!-- ══════════ 行为 ══════════ -->
        <template v-if="activeTab === 'behavior'">
          <!-- 关闭按钮行为 -->
          <div class="section-title">{{ t('settings.closeAction') }}</div>
          <div class="setting-group">
            <div class="radio-group">
              <label class="radio-option" :class="{ active: form.close_action === 'tray' }">
                <input v-model="form.close_action" type="radio" value="tray" />
                <span class="radio-text">
                  <span class="radio-title">{{ t('settings.closeToTray') }}</span>
                  <span class="radio-desc">{{ t('settings.closeToTrayDesc') }}</span>
                </span>
              </label>
              <label class="radio-option" :class="{ active: form.close_action === 'close' }">
                <input v-model="form.close_action" type="radio" value="close" />
                <span class="radio-text">
                  <span class="radio-title">{{ t('settings.closeDirectly') }}</span>
                  <span class="radio-desc">{{ t('settings.closeDirectlyDesc') }}</span>
                </span>
              </label>
            </div>
          </div>

          <!-- 开机自启 -->
          <div class="section-title">{{ t('settings.autorun') }}</div>
          <div class="setting-group">
            <label class="checkbox-label">
              <input v-model="form.autorun" type="checkbox" />
              {{ t('settings.autorunItem') }}
            </label>
            <label class="checkbox-label autorun-sub" :class="{ disabled: !form.autorun }">
              <input v-model="form.autorun_silent" type="checkbox" :disabled="!form.autorun" />
              {{ t('settings.autorunSilent') }}
            </label>
            <div class="form-hint">{{ t('settings.autorunHint') }}</div>
          </div>
        </template>

        <!-- ══════════ 高级 ══════════ -->
        <template v-if="activeTab === 'advanced'">
          <!-- Favicon 获取源 -->
          <div class="section-title">{{ t('settings.favicon') }}</div>
          <div class="setting-group">
            <div class="form-hint favicon-hint">
              {{ t('settings.faviconHint') }}
            </div>
            <textarea
              v-model="form.favicon_api_sources_text"
              class="textarea favicon-textarea"
              rows="4"
              spellcheck="false"
              placeholder="https://a.favicon.im/{url}?larger=true"
            ></textarea>
            <div class="favicon-actions">
              <button class="btn" @click="resetFaviconApiSources">{{ t('settings.faviconReset') }}</button>
            </div>
          </div>
        </template>

        <!-- ══════════ 维护 ══════════ -->
        <template v-if="activeTab === 'maintenance'">
          <!-- 过期资源清理 -->
          <div class="section-title">{{ t('settings.maintenance') }}</div>
          <div class="setting-group">
            <div class="form-hint clean-hint">
              {{ t('settings.cleanHint') }}
            </div>
            <div class="clean-actions">
              <button class="btn" :disabled="cleaning" @click="scanAssets">
                {{ cleaning ? t('common.processing') : '🔍 ' + t('settings.scan') }}
              </button>
              <button
                v-if="scanResult && scanResult.count > 0"
                class="btn btn-danger"
                :disabled="cleaning"
                @click="cleanAssets"
              >
                🗑 {{ t('settings.clean') }}
              </button>
            </div>
            <div v-if="scanResult" class="clean-result" :class="{ empty: scanResult.count === 0 }">
              <template v-if="scanResult.count > 0">
                {{ t('settings.cleanResult', { count: scanResult.count, size: scanResult.sizeText }) }}
              </template>
              <template v-else>
                {{ t('settings.cleanEmpty') }}
              </template>
            </div>
          </div>

        </template>

        <!-- ══════════ 插件 ══════════ -->
        <template v-if="activeTab === 'plugins'">
          <div class="section-title">{{ t('settings.plugins') }}</div>
          <div class="setting-group">
            <div class="form-hint">{{ t('settings.pluginsHint') }}</div>
            <div v-if="plugins.length === 0" class="plugins-empty">{{ t('settings.pluginsEmpty') }}</div>
            <div v-else class="plugins-list">
              <div
                v-for="p in plugins"
                :key="p.id + p.dir"
                class="plugin-item"
                :class="{ 'has-error': p.status === 'error', 'is-disabled': p.status === 'ok' && !p.enabled }"
              >
                <div class="plugin-body">
                  <div class="plugin-main">
                    <span class="plugin-name">{{ p.name }}</span>
                    <span v-if="p.version" class="plugin-version">v{{ p.version }}</span>
                    <span class="plugin-status" :class="p.status">
                      {{ p.status === 'ok' ? t('settings.pluginOk') : t('settings.pluginError') }}
                    </span>
                    <span v-if="p.status === 'ok' && !p.enabled" class="plugin-status disabled">
                      {{ t('settings.pluginDisabledBadge') }}
                    </span>
                    <span v-if="p.status === 'ok' && p.installed !== null" class="plugin-installed" :class="{ yes: p.installed }">
                      {{ p.installed ? t('settings.pluginDetected') : t('settings.pluginNotInstalled') }}
                    </span>
                  </div>
                  <div v-if="p.status === 'error'" class="plugin-error-msg">{{ p.message }}</div>
                  <div class="plugin-meta">{{ p.dir }}<template v-if="p.entry_mtime"> · {{ p.entry_mtime }}</template></div>
                </div>
                <!-- 启用开关：只对加载成功的插件显示（损坏插件无启停意义） -->
                <button
                  v-if="p.status === 'ok'"
                  class="plugin-switch"
                  :class="{ on: p.enabled }"
                  type="button"
                  role="switch"
                  :aria-checked="p.enabled"
                  :title="p.enabled ? t('settings.pluginDisableHint') : t('settings.pluginEnableHint')"
                  @click="togglePlugin(p)"
                >
                  <span class="plugin-switch-knob"></span>
                </button>
              </div>
            </div>
            <div class="plugins-actions">
              <button class="btn" @click="loadPlugins">🔄 {{ t('settings.pluginsRefresh') }}</button>
              <button class="btn" @click="openPluginsDir">📂 {{ t('settings.pluginsOpenDir') }}</button>
            </div>
            <div class="form-hint plugins-trust-hint">{{ t('settings.pluginsTrustHint') }}</div>
          </div>
        </template>
      </div>

      <div class="dialog-footer">
        <span v-if="error" class="error-text">{{ error }}</span>
        <button class="btn" :disabled="saving" @click="$emit('close')">{{ t('common.cancel') }}</button>
        <button class="btn btn-primary" :disabled="saving" @click="save">{{ saving ? t('common.saving') : t('common.save') }}</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.settings-dialog {
  width: 560px;
}

/* 设置对话框顶部对齐，避免切换 Tab 时高度变化导致垂直跳动 */
.settings-overlay {
  align-items: flex-start;
  padding-top: 40px;
}

/* ─── Tab 导航 ─── */
.settings-tabs {
  display: flex;
  border-bottom: 1px solid var(--color-border);
  padding: 0 4px;
}

.settings-tab {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 10px 4px;
  border: none;
  background: none;
  cursor: pointer;
  font-size: 13px;
  color: var(--color-text-secondary);
  border-bottom: 2px solid transparent;
  transition: all var(--transition);
  border-radius: 0;
}

.settings-tab:hover {
  color: var(--color-text);
  background: var(--color-bg-hover);
}

.settings-tab.active {
  color: var(--color-primary);
  border-bottom-color: var(--color-primary);
  font-weight: 600;
}

.tab-icon {
  font-size: 14px;
  line-height: 1;
}

.tab-label {
  font-size: 13px;
}

.setting-group {
  margin-bottom: 20px;
}

/* ─── 界面语言行 ─── */
.lang-row {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.lang-row .select {
  flex: 1;
  min-width: 160px;
}

.form-control {
  flex: 1;
  min-width: 0;
}

/* ─── LOGO 图片行 ─── */
.logo-image-row {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.checkbox-label {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  cursor: pointer;
  color: var(--color-text);
}

.checkbox-label input[type='checkbox'] {
  cursor: pointer;
  accent-color: var(--color-primary);
}

.checkbox-label input[type='checkbox']:disabled {
  cursor: not-allowed;
}

/* ─── LOGO 预览 ─── */
.logo-preview {
  margin-top: 10px;
  display: flex;
  align-items: center;
  gap: 12px;
}

.logo-preview.disabled {
  opacity: 0.5;
}

.logo-preview-box {
  width: 180px;
  height: 48px;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm);
  background: var(--gradient-top);
  display: flex;
  align-items: center;
  flex-shrink: 0;
  position: relative;
  overflow: hidden;
}

.logo-preview-img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

.logo-preview-filename {
  font-size: 11.5px;
  color: var(--color-text-tertiary);
  word-break: break-all;
}

/* ─── 开机自启 ─── */
.autorun-sub {
  margin-top: 8px;
  margin-left: 22px;
}

.autorun-sub.disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

/* ─── 单选组 ─── */
.radio-group {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.radio-option {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 10px 14px;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition);
}

.radio-option:hover {
  background: var(--color-bg-hover);
}

.radio-option.active {
  border-color: var(--color-primary);
  background: var(--color-primary-bg);
}

.radio-option input[type='radio'] {
  margin-top: 3px;
  cursor: pointer;
  accent-color: var(--color-primary);
}

.radio-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.radio-title {
  font-size: 13px;
  font-weight: 500;
  color: var(--color-text);
}

.radio-desc {
  font-size: 11.5px;
  color: var(--color-text-tertiary);
}

.error-text {
  margin-right: auto;
  font-size: 12px;
  color: var(--color-danger);
}

/* ─── 自定义 emoji 图标库 ─── */
.custom-emoji-hint {
  font-size: 12px;
  color: var(--color-text-tertiary);
  margin-bottom: 8px;
}

.custom-emoji-input-row {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 10px;
}

.custom-emoji-input-row .input {
  flex: 1;
  min-width: 0;
}

.custom-emoji-list {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.custom-emoji-item {
  position: relative;
  width: 36px;
  height: 36px;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm);
  background: var(--color-bg-card);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all var(--transition);
}

.custom-emoji-item:hover {
  border-color: var(--color-primary);
  background: var(--color-primary-bg);
}

.custom-emoji-char {
  font-size: 18px;
  line-height: 1;
}

.custom-emoji-remove {
  position: absolute;
  top: -4px;
  right: -4px;
  width: 16px;
  height: 16px;
  border: none;
  border-radius: 50%;
  background: var(--color-danger);
  color: #fff;
  font-size: 9px;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  padding: 0;
  opacity: 0;
  transition: opacity var(--transition);
}

.custom-emoji-item:hover .custom-emoji-remove {
  opacity: 1;
}

.custom-emoji-empty {
  font-size: 12px;
  color: var(--color-text-tertiary);
  padding: 8px 0;
}

/* ─── Favicon 获取源 ─── */
.favicon-hint {
  margin-bottom: 8px;
  line-height: 1.6;
}

.favicon-textarea {
  width: 100%;
  font-size: 12.5px;
  font-family: 'Cascadia Code', 'Consolas', monospace;
  line-height: 1.7;
  resize: vertical;
  min-height: 92px;
}

.favicon-actions {
  margin-top: 8px;
  display: flex;
  align-items: center;
  gap: 8px;
}

/* ─── 过期资源清理 ─── */
.clean-hint {
  margin-bottom: 10px;
}

.clean-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.clean-result {
  margin-top: 10px;
  padding: 8px 12px;
  border-radius: var(--radius-sm);
  background: var(--color-primary-bg);
  border: 1px solid var(--color-primary);
  font-size: 12.5px;
  color: var(--color-text);
}

.clean-result.empty {
  background: var(--color-bg-hover);
  border-color: var(--color-border);
  color: var(--color-text-tertiary);
}

.clean-result b {
  color: var(--color-primary);
  font-weight: 600;
}

.clean-result.empty b {
  color: var(--color-text-tertiary);
}

/* ─── 插件面板 ─── */
.plugins-empty {
  font-size: 12px;
  color: var(--color-text-tertiary);
  padding: 10px 12px;
  background: var(--color-bg-hover);
  border-radius: var(--radius-sm);
}

.plugins-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.plugin-item {
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm);
  padding: 8px 12px;
  display: flex;
  flex-direction: row;
  align-items: flex-start;
  gap: 10px;
}

.plugin-body {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

/* 停用态：整块降透明度，开关保持在右侧可点击 */
.plugin-item.is-disabled {
  opacity: 0.62;
}

.plugin-item.is-disabled:hover {
  opacity: 0.92;
}

.plugin-item.has-error {
  border-color: var(--color-danger);
}

/* 启用开关（Win11 风格小滑块） */
.plugin-switch {
  flex-shrink: 0;
  width: 34px;
  height: 18px;
  margin-top: 2px;
  padding: 0;
  border: 1px solid var(--color-border);
  border-radius: 9px;
  background: var(--color-bg-hover);
  cursor: pointer;
  position: relative;
  transition: background var(--transition), border-color var(--transition);
}

.plugin-switch:hover {
  border-color: var(--color-primary);
}

.plugin-switch .plugin-switch-knob {
  position: absolute;
  top: 2px;
  left: 2px;
  width: 12px;
  height: 12px;
  border-radius: 50%;
  background: var(--color-text-secondary);
  transition: transform var(--transition), background var(--transition);
}

.plugin-switch.on {
  background: var(--color-primary);
  border-color: var(--color-primary);
}

.plugin-switch.on .plugin-switch-knob {
  transform: translateX(16px);
  background: #fff;
}

.plugin-switch:focus-visible {
  outline: none;
  box-shadow: 0 0 0 2px var(--color-primary-bg);
}

.plugin-main {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.plugin-name {
  font-size: 13px;
  font-weight: 600;
}

.plugin-version {
  font-size: 11px;
  color: var(--color-text-tertiary);
}

.plugin-status {
  font-size: 11px;
  padding: 1px 8px;
  border-radius: 8px;
}

.plugin-status.ok {
  color: var(--color-primary);
  background: var(--color-primary-bg);
}

.plugin-status.error {
  color: var(--color-danger);
  background: var(--color-danger-bg);
}

.plugin-status.disabled {
  color: var(--color-text-tertiary);
  background: var(--color-bg-hover);
}

.plugin-installed {
  font-size: 11px;
  color: var(--color-text-tertiary);
}

.plugin-installed.yes {
  color: var(--color-primary);
}

.plugin-error-msg {
  font-size: 12px;
  color: var(--color-danger);
}

.plugin-meta {
  font-size: 11px;
  color: var(--color-text-tertiary);
  word-break: break-all;
  user-select: text;
}

.plugins-actions {
  display: flex;
  gap: 8px;
  margin-top: 4px;
}

.plugins-trust-hint {
  margin-top: 8px;
}

</style>