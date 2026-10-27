# 平台扫描插件架构

目标：Steam 扫描从主程序剥离成插件（独立 exe），以后平台规则变化只换插件，不改主程序；
新增 Epic / GOG 等平台照抄 Steam 插件即可。

三条硬约束：

1. 主程序不认识任何平台。代码里不出现 `steam`、`epic`、注册表键名、库目录特征。
2. 插件失效不拖垮主程序。超时、崩溃、输出非法，只让该插件标记不可用。
3. 协议只增不改。新字段旧主程序忽略，旧字段新主程序继续兼容。

## 一、目录与产物

```
<exe目录>/plugins/
  steam/
    plugin.json
    steam-scanner.exe
  epic/
    plugin.json
    epic-scanner.exe
```

- 插件目录与 exe 同级，绿色便携，拷贝整个文件夹即可带走。
- 主程序启动时扫描 `plugins/*/plugin.json`，没有插件就是"没装该平台扫描器"，按普通目录处理。
- 不用 Tauri externalBin：那是打包进安装包的资源，替换/新增插件得重新打包，与"改插件不改程序"的要求冲突。

源码与构建产物分开：

```
plugins-src/steam/        # 插件 Rust 源码（独立 crate，不依赖 tauri）
plugins/steam/            # 构建产物：plugin.json + exe
build-plugins.bat         # 编译所有插件并拷到 plugins/
```

`build-portable.bat` 增加一步：把 `plugins/` 整个复制进 `MyLauncherPortable/`。

## 二、plugin.json 契约

```json
{
  "api_version": 1,
  "id": "steam",
  "name": "Steam",
  "version": "1.0.0",
  "entry": "steam-scanner.exe",
  "priority": 0,
  "timeout_ms": 30000,
  "probe": {
    "dir_markers": ["steamapps"]
  },
  "installed_check": {
    "hive": "HKCU",
    "key": "Software\\Valve\\Steam",
    "value": "SteamPath"
  },
  "capabilities": ["scan_dir", "scan_id"]
}
```

| 字段 | 必需 | 说明 |
|---|---|---|
| `api_version` | 是 | 协议版本，当前只支持 1；不匹配则跳过并在插件面板标红 |
| `id` | 是 | 小写字母数字连字符，决定目录名与条目里的 `provider_id` |
| `entry` | 是 | 插件可执行文件名，相对插件目录 |
| `priority` | 否 | 目录认领冲突时高者优先，默认 0，同值按 id 排序 |
| `timeout_ms` | 否 | 单次扫描超时，默认 30000，上限 120000 |
| `probe.dir_markers` | 否 | 这些子项**全部存在**时认领该目录。Steam = `steamapps` |
| `installed_check` | 否 | 只用于插件面板显示"已检测到/未安装"，不参与扫描 |
| `capabilities` | 是 | 支持的命令子集：`scan_dir`（扫目录）、`scan_id`（按平台 ID 查单个） |

未知字段一律忽略（不要开 `deny_unknown_fields`），方便协议只增不改。

## 三、CLI 契约

```
<entry> scan-dir --dir <绝对路径>
<entry> scan-id  --id <平台条目ID>
```

- 只认 `--dir` / `--id`，可加其他参数但主程序不会传。
- 参数路径可能带空格，插件按单参数接收（宿主用进程参数数组传，不拼命令行）。
- stdout **只放 JSON**，任何日志走 stderr。宿主只解析 stdout。
- 退出码：0 = 成功（`items` 可以为空），非 0 = 失败，stderr 里放一句人能看懂的原因。

## 四、输出 JSON 契约（api_version 1）

```json
{
  "api_version": 1,
  "items": [
    {
      "name": "Counter-Strike 2",
      "provider_item_id": "730",
      "exe_path": "D:\\SteamLibrary\\steamapps\\common\\Counter-Strike Global Offensive\\game\\bin\\win64\\cs2.exe",
      "candidates": [
        { "exe_path": "D:\\...\\cs2.exe", "label": "cs2" },
        { "exe_path": "D:\\...\\cs2launcher.exe", "label": "cs2launcher" }
      ],
      "launch_url": "steam://rungameid/730",
      "cover": {
        "local_path": "D:\\Steam\\appcache\\librarycache\\730_header.jpg",
        "url": "https://cdn.cloudflare.steamstatic.com/steam/apps/730/header.jpg"
      },
      "meta": { "install_dir": "Counter-Strike Global Offensive" }
    }
  ],
  "warnings": ["3 个库目录无法访问"]
}
```

| 字段 | 必需 | 宿主处理 |
|---|---|---|
| `name` | 是 | 条目名；`candidates` > 1 时宿主追加 `[label]` 区分 |
| `provider_item_id` | 是 | 存进 `entry.provider.item_id`，编辑页"按 ID 重扫"用 |
| `exe_path` | 否 | 主程序；空表示只能靠 `launch_url` 启动 |
| `candidates` | 否 | 多个可执行文件，宿主展开成多条扫描结果，第一个为主（前端红色提示勾选） |
| `launch_url` | 否 | 启动链接，存进 `entry.url`。有它就能正常启动（协议由系统处理） |
| `cover.local_path` | 否 | 本地图片，存在则优先复制到 `assets/covers/<provider_id>/` |
| `cover.url` | 否 | 本地缺失时下载，2 秒超时，失败静默 |
| `meta` | 否 | 原样透传到 `entry.provider.meta`，仅供 UI 显示与插件自身语义 |
| `warnings` | 否 | 汇总进扫描结果提示区 |

插件**不碰** `data/`、`assets/`、主程序配置。封面只给来源，复制/下载由宿主做。
这条是插件简单的关键：插件是纯函数，输入路径、输出 JSON。

## 五、宿主处理流程

```
用户选目录
  → 遍历 plugins/*/plugin.json（缓存一次，磁盘变更时重扫）
  → probe：dir_markers 全部存在 → 候选插件
  → 多插件命中：按 priority 降序，同值按 id 字典序，取第一个
  → 命中：spawn entry scan-dir --dir <path>
       ├─ 超时（timeout_ms）→ kill，warning「Steam 插件超时」
       ├─ 退出码非 0 → warning + stderr 首行
       ├─ stdout > 8MB → 丢弃，warning
       └─ JSON 解析失败 → warning「插件输出格式错误」
  → 未命中：走原有递归扫 exe 逻辑
  → items → ScanItem 归一化 → 前端
```

归一化时宿主负责：

- `suggested_path_mode` / `relative_path`：同盘（exe 所在盘 == 程序所在盘）用 relative，否则 absolute。规则与现在一致。
- `file_exists`：`exe_path` 存在与否。
- 图标：**不在扫描阶段提取**，沿用现有"导入时批量提取"流程，按 `exe_path` 提取。
- 封面：扫描阶段不下载，导入时按 `cover.local_path` → `cover.url` 顺序取。

### 插件启停

`config.json` 增加 `disabled_plugins: ["<id>", ...]`，设置页每个插件右侧的开关直接改这个列表并立即落盘。语义：

| 场景 | 停用后的行为 |
|---|---|
| 扫目录 | 该插件不参与 `dir_markers` 探测，目录回到普通递归扫 exe |
| 按 ID 查询 | `scan_provider_item` 报「未安装或已停用平台扫描插件」 |
| EntryEditor 平台下拉 | 不再列出该平台（新增条目选不到） |
| 已导入的条目 | **完全不受影响**：启动只看条目的 `url` / exe 路径 |
| 插件文件 | 不动，随时可重新启用 |

宿主只读配置、不改写配置——写盘统一由前端保存流程完成，避免两端互相覆盖。宿主不保留任何内置平台回退，所以停用后不会有"绕过开关的内置实现"把目录抢回去。


## 六、数据模型变更

### ScanItem（扫描结果，替代 ExeInfo 里的 steam 字段）

```rust
pub struct ScanItem {
    pub name: String,
    pub exe_path: String,
    pub suggested_path_mode: String,
    pub relative_path: String,
    pub absolute_path: String,
    pub file_exists: bool,
    pub provider: Option<ProviderRef>,   // id + item_id + launch_url + meta
    pub cover: Option<CoverSource>,      // local_path + url
    pub multi_exe: bool,
    pub is_primary: bool,
}
```

`ProviderRef { id, item_id, launch_url, meta }`。

### Entry

```ts
export type EntryType = 'program' | 'url' | 'folder' | 'file' | 'system' | 'appx' | 'provider'

export interface ProviderInfo {
  id: string          // 插件 id，如 "steam"
  item_id: string     // 平台条目 id，如 "730"
  meta?: Record<string, string>
}

export interface Entry {
  // ...原有字段
  provider?: ProviderInfo
}
```

改动点：

- 删除 `type: 'steam'`，新增 `'provider'`。
- 启动链接继续用现有 `url` 字段，不新增。
- 旧数据迁移：加载时 `type == 'steam'` → `type = 'provider'`、`provider = { id: 'steam', item_id: 从 url 提取 }`。内存里改，下次保存落盘。

### 启动逻辑（launch.rs）

删除 `entry_type == "steam"` 分支，改为：

```
type == 'provider':
    url 非空 → ShellExecute(url)          // steam://、epic://、goggalaxy:// 都一样
    url 为空 → 按 program 的 exe 路径启动
```

主程序从此不认识任何平台协议。

## 七、现有代码改造清单

| 文件 | 改动 |
|---|---|
| `src-tauri/src/steam.rs` | 已删除；目录扫描入口保留为 `scan.rs`，算法搬进 `plugins-src/steam/` |
| `src-tauri/src/plugin_host.rs` | 新增：发现、探测、调度、超时、JSON 解析、ScanItem 归一化 |
| `lib.rs` | `scan_directory` 改为先问插件、未命中走原逻辑；删除 `scan_steam_app` 注册，换 `scan_provider_item` |
| `models.rs` | `ExeInfo` 删 `steam`，加 `provider` / `cover`；新增 `ProviderRef` |
| `import.rs` | 删 steam 分支，改按 `provider` 处理：entry_type、url、封面来源 |
| `launch.rs` | 删 steam 两处分支，改 `provider` + url 优先 |
| `icons_refresh.rs` | 删 steam 分支（按 exe 提取已通用） |
| `assets.rs` | `covers/steam` 改 `covers/<provider_id>`，清理逻辑按目录扫描，不改 |
| 前端 `types/index.ts` | `EntryType` 加 `provider`，`Entry` 加 `provider` |
| 前端 `EntryEditor.vue` | Steam 类型 → "平台游戏"；appid 输入改平台下拉（来自 `list_plugins`）+ ID 输入 |
| 前端 `App.vue` | steam 启动选择弹窗改成"通过 <平台名> 启动 / 直接运行程序"，判断条件改 `entry.provider` |
| 前端 `ScanDialog.vue` | 显示 `warnings` |
| 前端设置 | 新增"插件"分组：列表（名称/版本/状态）、打开插件目录、刷新 |

## 八、分阶段计划

> 进度（2026-10-20）：阶段 1～4 全部完成。主程序里已无任何平台特征代码：
> `steam.rs` 已删除，目录扫描入口改名 `scan.rs` 且只剩「插件认领 / 普通递归扫 exe」两条路径。

**阶段 1：宿主骨架 + 模型就位（不动 Steam 算法）** ✅

- 新增 `plugin_host.rs`、`ProviderRef`、`ScanItem`、`list_plugins` / `scan_provider_item` / `open_plugins_dir` 命令
- `scan_directory` 增加插件探测分支；没有插件时行为与现在完全一致
- 数据模型加 `provider` 字段，`type='provider'` 进类型定义；迁移代码写好但先不启用
- 验收：不放任何插件时，现有功能零变化

**阶段 2：Steam 插件 + 双轨对比** ✅

- 新建 `plugins-src/steam/`，把 `steam.rs` 的算法搬过去（注册表、libraryfolders.vdf、acf、exe 启发式）
- 主程序优先走插件；插件缺失或报错时回退内置实现（这一版双轨）
- 验收：同一台机器上，内置扫描与插件扫描结果逐字段一致（名称、exe、封面来源、appid）

**阶段 3：删内置 Steam 代码** ✅

- 删除 `steam.rs`（目录扫描入口保留并改名 `scan.rs`）、`ExeInfo.steam`、import/launch/icons 里的全部分支、`scan_steam_app`
- 迁移旧数据 `type='steam'` → `provider`
- 不做过渡回退：插件缺失或停用时，平台库目录就是普通目录，按普通递归扫 exe，不提示也不特判
- 验收：删掉 `plugins/steam/` 后，选 Steam 目录按普通目录递归扫描，不做任何提示或特判；
  放回插件后立刻恢复为游戏列表；旧条目仍能启动

**阶段 4：UI** ✅

- 设置 → 插件面板（状态、版本、打开目录、刷新）
- EntryEditor 平台化（平台下拉 + ID）
- 验收：新增一个假插件（比如扫某个自定义目录格式），不改主程序即可出现在扫描路径里

## 九、明确不做

- **插件市场 / 自动更新**：手动替换目录。需要时再加 manifest 的 `update_url`。
- **插件配置 UI**：插件要配置就自己在自己目录放 json，宿主不解析。
- **脚本插件（JS/Lua）**：只支持独立 exe。将来真需要再叠加，协议输出不变即可复用。
- **签名校验**：插件是本机文件，信任边界与主程序一致；不做签名，但插件面板显示来源路径。
- **插件注册新条目类型 / 自定义表单**：插件只扩展"数据来源"，不扩展"UI 结构"。
  条目类型固定为现有 7 种（含 provider），表单由主程序统一提供。

## 十、待定

- 多插件同时认领同一目录（比如 Steam 与某个"通用 Steam 库"插件）：
  当前按 priority 取第一个。是否需要"全部执行并合并"？建议不做，等真出现再改。

## 十一、对外发布：插件作者规范

插件协议要给别人写，下面这些是硬性校验，宿主实现时逐条落地。

**标识与目录**

- `id` 必须匹配 `^[a-z][a-z0-9-]{0,31}$`。宿主校验后才拼目录路径，防路径穿越。
- 插件目录内 `plugin.json` 与 `entry` 指向的文件都必须真实存在，`entry` 不允许带路径分隔符。

**版本兼容**

- 宿主声明支持的 `api_version` 区间（当前只支持 1）。
- 不匹配：**拒绝加载**，插件面板显示「需要更新主程序」或「插件版本过低」，不静默跳过。
- manifest 里可加 `min_host_version`，新插件在旧主程序上直接提示而不是跑出怪结果。

**输出限制（防失控）**

| 项 | 上限 |
|---|---|
| stdout 总量 | 8 MB |
| `items` 条数 | 5000 |
| 单个 `name` | 200 字符 |
| 单个路径 | 4096 字符 |
| `meta` 单个值 | 1024 字符 |
| `warnings` 总量 | 64 KB |

超限按"输出非法"处理，插件标记不可用，不尝试部分解析。

**进程与错误**

- 默认超时 30 秒，manifest `timeout_ms` 可覆盖，上限 120 秒。
- stdout 只放 JSON；stderr 保留前 4 KB 作为错误详情显示在插件面板。
- 退出码非 0 视为失败，即使 stdout 有内容也不采信。
- 宿主在 Windows 下用 `CREATE_NO_WINDOW` 启动插件进程，插件按控制台程序编译也不会闪黑框；
  因此插件**不能**依赖已附加的控制台（不要用 `print` 到控制台缓冲区、不要读 stdin），
  所有输出一律写 stdout 管道。插件自己再 spawn 子进程时需自行加 `CREATE_NO_WINDOW`。

**语言无关**

任何能产出 exe 的语言都行（Rust / Go / C# / Python + PyInstaller / Node + pkg）。
第一版只支持单个可执行文件；`entry` 将来可扩展为数组以支持脚本，暂不做。

**边界**

- 插件不碰 `data/`、`assets/`、主程序配置，只能读自己要扫的平台数据。
- 宿主不做签名校验：插件面板展示路径、版本、文件修改时间；设置页一次性提示
  「插件是可执行文件，只安装可信来源」。

## 十二、添加单个条目：平台类型怎么落地

现状：EntryEditor 的类型下拉里硬编码一项「Steam 游戏」，选中后输入 appid、
点扫描回填名称/封面/exe，保存 `type='steam'`。

改为：**取消硬编码的「Steam 游戏」，收编为通用的「平台游戏」（`type='provider'`）**。
平台列表由已装插件动态提供，不写死任何平台。

**表单行为**

```
类型：平台游戏
  ┌ 平台   [ Steam ▾ ]        ← 已装且 capabilities 含 scan_id 的插件
  └ 条目 ID [ 730    ] [查询]
      查询成功 → 回填名称、exe 路径、封面预览（均可手动改）
      保存 → type='provider'，provider={id, item_id}，url=launch_url
```

- 只装一个插件时平台下拉自动选中它，体验与原「Steam 游戏」一致。
- 没装任何支持 `scan_id` 的插件：类型下拉里不出现「平台游戏」。
- 编辑旧条目时若对应插件已卸载（比如换机器没带 `plugins/`）：下拉里保留该 id，
  标注「未安装」，允许保存但不改平台——不能因为插件缺失就把条目改坏。

**启动方式弹窗（App.vue）**

条件从 `type=='steam'` 改为 `entry.provider && entry.url`；文案用插件名：
「通过 Steam 启动」/「直接运行程序」。平台名从 `list_plugins` 查，查不到用 id。

**为什么不放开"插件注册新类型"**

1. 条目类型的本质只有两件事：怎么启动、怎么显示。现有 7 种已覆盖全部场景，
   平台游戏的差异只在"ID + 查询接口 + 启动协议"，一套通用表单足够。
2. 一旦允许插件定义类型和表单字段，主程序就得做通用表单引擎（schema → 控件 →
   校验 → 布局），维护成本顶半个低代码平台，与"稳定底层"直接冲突。
3. 插件生态早期最怕协议反复。收敛成 provider 后，加 Epic/GOG 只需写插件，
   前端零改动；放开类型注册则每加一个插件都可能要求主程序改渲染逻辑。

需要新类型时的替代路径：用 provider 表达。比如 Docker 容器插件可以给
`provider_item_id=容器名`、`launch_url=docker://...`，条目照常显示与启动。
