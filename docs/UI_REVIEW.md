# DM 界面审查（2026-11）

审查对象：`src/App.svelte`、`src/app.css`、`src/lib/components/*`、`browser-extension/popup.html`。
参照：`docs/UI_DESIGN_PLAN.md` v2（审查后升级为 v2.1）。
方式：逐文件静态审查 + 设计 token 对照 + 对照 `docs/ui/` 下的审查前截图 + 编译/构建验证。当前沙箱无法启动 Chromium/Firefox（进程被系统终止），因此没有**改动后**的截图级回归；所有结论均来自实现代码、审查前截图与渲染规则，最终仍建议在真实 WebView 中做一次双主题目检。

## 总体判断

v2 的设计系统本身是成立的：扁平表面、发丝线、单一强调色、表格化数字。问题不在“设计方向”，而在**实现漂移**：

1. **浅色主题没有真正实现**。大量 `rgba(255,255,255,…)` 悬停底色和 `rgba(0,0,0,…)` 底衬只在深色下成立，切到浅色后要么不可见，要么发灰。
2. **信息层有事实性错误**。筛选条只给 All/Active 算了数，其余计数永远是 0；“Active”把暂停/排队也算进去；状态栏重启后仍显示“Unlimited”；添加单个链接的“Speed cap”实际改的是全局限速。
3. **层级互相打架**。批量操作条与列头使用同一个 sticky 偏移（86 px），滚动时互相覆盖；状态栏整条用等宽字体，和账本争夺注意力；行内同时有 meter、百分比、状态标签、状态点四处强调色。
4. **浮层规范各自为政**。圆角 8/10/12/14/16 混用，白色 alpha 悬停、渐变开关、发光状态点、无效的 `box-shadow` 声明并存。

一句话：**“简约”已经有了，“不简单”还差在系统性和数据诚实度上。**

## 问题清单

严重度：**S1** 数据错误或主题下明显破损；**S2** 高频路径上的明显质量回退；**S3** 一致性问题；**S4** 打磨项。
优先级：**P0** 本轮必修；**P1** 应修；**P2** 机会性。

| # | 区域 | 问题 | 影响 | 严重度 | 优先级 | 状态 |
|---|---|---|---|---|---|---|
| 1 | 主题 | BulkActionBar / Toast / CommandPalette / Settings / InlineSpeedLimit / AuthDialog / CaptureDialog 使用固定白色 alpha 悬停与黑色底衬 | 浅色主题下按钮“没有反馈”、代码块发灰、浮层脏 | S2 | P0 | 已修 |
| 2 | 筛选 | `SmartFilters` 只为 All/Active 赋值；Queued/Paused/Done/Errors 的 `count` 恒为 0；“Active”由前 5 个分类桶相加，依赖对象顺序 | 概览数字错误且不稳定，筛选条像半成品 | S1 | P0 | 已修 |
| 3 | 语义 | `counts.active` 与 `matchFilter("active")` 都包含 paused/queued/scheduled | 状态栏声称 4 个活跃任务，实际可能 0 个在传输 | S1 | P0 | 已修 |
| 4 | 添加流程 | “More → Speed cap”调用 `setGlobalSpeedLimit`，改的是全局限速；全局限速在 Settings 中没有入口 | 用户为单个文件设限，却静默限速整个应用 | S1 | P0 | 已修 |
| 5 | 布局 | 批量条与列头同为 `sticky; top: 48+38`，z-index 170 vs 180 | 选中行后滚动，列头压住批量条；列表被顶下去 | S2 | P0 | 已修 |
| 6 | 状态栏 | 整条 `font-family: mono`，包括 active/downloaded/done 等标签；`role="status"` 每秒播报速率 | 读起来像终端；屏幕阅读器被每秒刷新骚扰 | S2 | P0 | 已修 |
| 7 | 文件图标 | 28 px 宽的 tile 塞进 26 px 网格列；高度只有 19 px，字号 8.4 px；PDF 用 danger 红，ISO/DMG 用 image 紫 | 图标列 2 px 溢出；9 px 以下三字母不可读；类型语义错误 | S2 | P1 | 已修 |
| 8 | 颜色 | video≈warning 琥珀、archive≈success 绿、document≈info 蓝、PDF=danger 红 | 文件类型看起来像状态，语义色被稀释 | S2 | P1 | 已修 |
| 9 | 行层级 | 活跃行同时把 meter、百分比、状态标签、状态点都染成 accent | 一行四个高亮，眼睛没有落点 | S3 | P1 | 已修 |
| 10 | 字号 | Settings/Auth/Capture 的 `text-transform: uppercase; letter-spacing: .04–.08em`；列头 10 px；Settings 标题 22 px | 违反自家 11–20 px 字阶与“无大写标签”规则，观感像两套产品 | S3 | P1 | 已修 |
| 11 | 设置 | 开关用 `linear-gradient`；状态点 `box-shadow: 0 0 6px`；proxy 卡 hover `translateY(-1px)`；圆角 6/8/10/12 混用 | 与“无渐变、无发光、浮层才用阴影”的契约冲突 | S2 | P1 | 已修 |
| 12 | 设置 | `.drawer` 的 `box-shadow: -30px 0 80px -30px var(--shadow-modal)` 把阴影 token 当颜色用 | 整条声明无效，抽屉没有阴影 | S2 | P1 | 已修 |
| 13 | 命令面板 | `select-download` 事件无人监听；模式切换是 `⌘`/`⬇` 两个符号；radius 16；深色 footer | 选中下载无反应；模式不可读；浅色下发黑 | S2 | P1 | 已修 |
| 14 | Toast | `bottom: 24px` 与 32 px 固定状态栏重叠；radius 14；`fly` 入场 | 遮住右下角指标；动效与规范相反 | S3 | P1 | 已修 |
| 15 | 设置 | Export / Import / Reset 三个按钮没有 handler | 点了没反应的“假按钮”，Reset 尤其危险 | S2 | P1 | 已禁用并标注 |
| 16 | 进度条 | 分段路径完全忽略 `status`，已完成的多连接行仍是 accent 色 | 完成行出现“绿点 + 玉色进度条”的自相矛盾 | S2 | P1 | 已修 |
| 16b | 进度条 | 每个 chunk 各画一条独立 bar，4 连接行看起来像 4 段断开的下载 | 与“连续 meter + hairline ticks”的契约相反，扫一眼会误判进度 | S2 | P1 | 已修 |
| 17 | 状态栏 | `globalSpeedLimitLabel` 初始恒为 “Unlimited”，不从 settings 回填 | 重启后限速值显示错误 | S2 | P1 | 已修 |
| 18 | 可用性 | Add 按钮在空输入时仍可点；非法 URL 静默失败；失败后 URL 已被清空 | 用户不知道发生了什么，还要重新粘贴 | S3 | P1 | 已修 |
| 19 | 键盘 | 行内按钮上的 Enter 冒泡到 row 的 `onKeydown`，同时触发按钮与展开 | 键盘用户按一次键执行两个动作 | S2 | P2 | 已修 |
| 20 | 冗余装饰 | 筛选 chip 的 12 px 图标重复标签；错误行额外一个“!”气泡；列头重复 “N shown / N total”；搜索键帽画了边框 | 每处都在抢信息，密度反而下降 | S3 | P2 | 已修 |
| 21 | 外壳 | `.app` 限宽 1480，而固定状态栏全宽；顶栏与筛选条半透明 + blur | 超宽屏下账本与 chrome 脱节；滚动时文字被糊 | S3 | P2 | 已修 |
| 22 | 死代码 | `⌘[` 切换已删除的 sidebar；`StatusBadge` 无引用；`sortBy`/`viewMode`/`showAddDialog`/`showOnboarding` 无引用 | 交互承诺了不存在的东西，后续维护误导 | S3 | P2 | 已清理 |
| 23 | 空状态 | 硬编码 `⌘K` / `⌘N` | Windows/Linux 用户看到错误快捷键 | S4 | P2 | 已修 |
| 24 | 扩展弹窗 | 浅色 `--faint` 对比不足；logo 是纯色方块；字号 12.5/11.5/10.5 | 弹窗像未完成的占位稿 | S3 | P2 | 已修 |
| 25 | 根节点 | `role="application"` | 屏幕阅读器进入应用模式，浏览语义被削弱 | S3 | P2 | 已改 `<main>` |
| 26 | 对话框 | AuthDialog 的 `<label>` 未关联控件（svelte-check warning）；正文/按钮在 accent 上用 `#fff` | 检查不干净；深色下白字对比不足 | S3 | P2 | 已修 |
| 27 | Sparkline | uPlot 颜色只在 mount 时读取 CSS 变量 | 切换主题后折线仍是旧主题色 | S4 | P2 | 已修 |
| 28 | 扩展弹窗 | host 状态仍在底部，而非文档所述“标题行 + 状态点” | 与文档不一致，但不影响功能 | S4 | P2 | 已修 |
| 29 | 展开面板 | `ex-grid` 没有最大宽度，URL 与 Started 被 auto-fit 拉到面板两端 | 两个字段隔着一千像素，读不成一组 | S3 | P2 | 已修 |
| 30 | 展开面板 | 无限速时 `InlineSpeedLimit` 只渲染一个图标空壳，没有任何文字 | 看起来像坏掉的控件，而不是“无限制” | S3 | P2 | 已修（现在显示 `No limit` / `5 MB/s`） |

## 优先级与处理顺序

**P0（已完成）**：#1–#6。先保证浅色主题可用、筛选数字可信、限速语义正确、选中态不与列头打架、状态栏可读。

**P1（已完成）**：#7–#18（含 #16b）。视觉系统的“同一套语言”：类型图标、分类色、字号、开关、浮层圆角、命令面板、Toast、进度条状态。顺带修掉三个无 handler 的数据按钮（禁用而非留假入口）。

**P2（已完成）**：#19–#30。清理死代码与装饰噪音，补键盘与 a11y 细节，统一空状态、展开面板和扩展弹窗；最后两项（Sparkline 主题响应、扩展弹窗状态行重排）在本轮收尾，清单已无未处理项。

## 主要变更

### 1. 设计 token 与外壳（`src/app.css`、`src/lib/design-tokens.ts`）

- 新增 `--color-track`、`--select-chevron`、`--size-filterbar`、`--size-row(-compact)`；圆角收敛为 3/6/8/12/16；删除 ambient/grain/shimmer/sidebar/kpi/stagger 等 12 个遗留 token。
- 分类色改为低彩度、与语义色错位的六色盘：video 紫、audio 青、archive 赭、document 蓝灰、image 藕、binary 灰。PDF 归 document，ISO/DMG 归 binary。
- 顶栏/筛选条改为**实色 + 发丝线**，去掉 `backdrop-filter`；`.app` 改为全宽；列头 sticky 偏移改用 token；行高/列宽与 28 px 图标列对齐。
- 全局补齐 `.btn:disabled` 状态；`select` 箭头改成随主题切换的 `--select-chevron`（浅色下不再用深色主题的灰箭头）。

### 2. 数据与语义（`stores/downloads.ts`、`stores/settings.ts`、`App.svelte`）

- `counts.active` 与 `active` 筛选统一为 `downloading | connecting`；queued/paused/scheduled 不再冒充活跃。
- `SmartFilters` 的六个状态计数全部来自 `counts`；分类计数仍来自 `smartFolders`，且只在 >0 时出现。
- 添加行的 “Speed cap” 改为 **per-download limit**，通过 `addDownload({ speedLimit })` 下发；成功后重置。
- 全局限速迁入 Settings → Network → Bandwidth（∞/5/20/50 + 自定义），并回填状态栏的 `limit`；重启不再显示错误的 Unlimited。
- Add 按钮在输入不是完整 http(s) URL 时禁用；非法输入给出明确错误；失败时保留 URL 以便重试。

### 3. 列表与状态栏（`DownloadRow`、`FileIcon`、`ProgressBar`、`StatusBar`）

- 行保持 40 px 账本结构；图标 tile 28×28、9.5 px 三字母；去掉错误行重复的“!”气泡；展开面板左缘对齐文件名列（78 px），虚线改实线，`ex-grid` 最大 860 px（URL / Started 不再被拉到两端）。
- 活跃行只保留 meter + 状态标签 + 状态点三处 accent，百分比改为 primary text。
- 分段 meter 现在是一条**连续填充**，chunk 边界以 1 px 刻线叠加（旧实现每 chunk 一条独立 bar，4 连接看起来像 4 段断开下载）；填充色响应 `status`：完成=success 55%，错误=danger 60%，暂停=faint。
- 状态栏标签用 UI 字体 11 px，数值用 mono tabular；速度不再染绿；零活跃时速率显示 `—`；移除 ping（迁到 Settings → About）与 `role="status"`。

### 4. 浮层与设置（`BulkActionBar`、`Toast`、`CommandPalette`、`SettingsTabs`、`AuthDialog`、`CaptureDialog`）

- 批量条改为**状态栏上方的浮动条**（`bottom: 32+10`，radius 12，floating shadow）；App 在选中时预留 104 px 滚动空间；不再与列头共用 sticky 偏移。
- Toast 抬到状态栏上方，radius 12，只做 opacity；标题/正文降到 13/12。
- 命令面板 radius 12、footer 用 `bg`、模式改成 `Commands / Downloads` 文本切换；新增 `on:select-download`：清空筛选并把搜索设为该文件名，确保选中项可见。
- Settings：标题 16 px；字段标签取消大写与 tracking；开关改平涂 accent；去掉状态点发光、卡片 hover 位移、混用圆角；修复无效阴影；新增 Bandwidth 与 Engine health；Data 三键禁用并标注未实现。
- Auth/Capture：统一 12 px 圆角、`--color-backdrop`；accent 按钮文字改 `--color-inverse`；Auth 的 `.warn` 从 `warning-strong`（0.28 alpha）改回 `warning`；修掉 label 关联 warning。
- 扩展弹窗：补真正的下拉箭头品牌标、浅色 faint 提到 `#676E7C`、字号收敛到 12/11。

### 5. 文档

- `docs/UI_DESIGN_PLAN.md` 升级为 v2.1：修正行高（40 px 而非 52–56）、状态栏字体规则、ping 位置、modal/drawer 动效、选择条位置、添加行的 per-download 限速；补充分类色表与“readout 只用于数值”的约束；更新实现映射与验证命令。
- 本文件为新增加的设计审查记录。

### 6. 收尾两项（#27、#28）

- **Sparkline 主题响应**：`SpeedGraph.svelte` 不再在 `onMount` 里缓存颜色。uPlot 的 `series.stroke` / `axes[].grid.stroke` 改为函数，每次绘制时惰性读取 `--color-primary` / `--color-border-2`；`effectiveThemeStore` 改为由 `applyTheme` 驱动的 writable，`system` 模式下 OS 偏好翻转也会通知订阅者，切换主题立即 `redraw()`，无需重建图表。
- **扩展弹窗状态行**：`popup.html` 标题行新增状态点 + 文字标签（`Connected` / `Disconnected` / `Checking…`），host 状态与错误详情移到顶部，底部不再有状态块；`#status` 收敛为单行（超长省略，完整文本进 `title`），原有诊断文案与 transport 信息全部保留。`popup.js` 新增 `setHostState()`，新元素均做空值保护。
- **回归约束**：`integration.test.mjs` 新增断开 / 连接两条状态点测试，`mutation-check.mjs` 新增“把状态点钉成 `ok`”的变异；测试的元素 id 清单同步补齐。

## 验证

| 检查 | 结果 |
|---|---|
| `svelte-check --tsconfig ./tsconfig.json` | **0 errors / 0 warnings**（审查前为 0/1）。沙箱 WASM 受限，`npm run check` 需加 `NODE_OPTIONS=--unhandled-rejections=warn` 绕开 undici 后台 fetch 的未处理拒绝 |
| `node scripts/check-css-vars.mjs` | **CSS vars OK**，715 处 `var()`、136 个属性，双主题均可解析 |
| `vite build` | **成功**（82 modules；CSS 72.62 kB / JS 246.16 kB）。沙箱内 WASM 不可用，用临时配置跳过 `vite:build-import-analysis` / `vite:dynamic-import-vars` 两个仅做 import 分析的插件，Svelte/TS/CSS 全量编译与 rollup 打包照常执行；临时配置未入库 |
| Svelte 组件全量预编译 | **16 个组件 / 0 errors**（`vitePreprocess` + `svelte.compile`，作为构建被截断时的补充验证） |
| 扩展回归 `node --test test/*.test.mjs` | **106 passed / 0 failed**（popup 结构改动未破坏选择器，新增 2 条状态点测试） |
| `node test/mutation-check.mjs` | **25/25 caught**（新增 1 条变异） |
| `cargo test -p dm-engine` | **全绿**。沙箱 `/tmp` 只有 10 MB，需把 `TMPDIR` 指向仓库内 gitignored 目录并限制 `--test-threads=2` |
| 截图级回归 | **未执行**：沙箱内 Chromium/Firefox 均被系统终止；建议在真实 Tauri WebView 中双主题目检一次 |

## 建议的下一步（不改代码也能做）

1. 在 1440×900 与 1024×768 两个尺寸、深浅两个主题下各走一遍：空列表、6 条混合状态、选中 2 条、展开一条多连接行、打开设置、打开命令面板；另外确认切换主题时状态栏 sparkline 的折线/网格线立刻换色（#27），以及扩展弹窗浅/深主题下标题行状态点可读（#28）。
2. 确认 `--color-category-*` 在真实屏幕上 9.5 px 仍可辨；若偏弱，优先提高 tile 对比而不是加粗字重。
3. 若要继续收敛，下一刀应砍 **Settings 的信息架构**（8 个 tab 偏多，Extensions 的长文档可以折叠成“安装指南”），而不是继续调颜色。
