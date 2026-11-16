# DM 界面第二轮审查（2026-11-15）

**基线**：`a94937e fix(ui): finish the UI review pass, close the last two P2 items`（工作区干净）。
**范围**：`src/App.svelte`、`src/app.css`、`src/lib/**`、`docs/UI_DESIGN_PLAN.md` v2.1、`browser-extension/**`。
**方式**：逐文件静态审查 + 契约对照 + 可复现实验（扩展 harness 端到端注入、uPlot 源码调用点核对、WCAG 相对亮度计算）+ 验证命令复跑。
**结论**：上一轮 30 项"已修"的结论没有被推翻，但审查换到**失败路径、键盘路径与生命周期**后暴露了 2 个功能级缺陷、4 个数据/覆盖问题，以及一批契约一致性问题。本轮**只审查、未改实现代码**。

> 本文件是第二轮问题清单的唯一归属；第一轮清单在 [`UI_REVIEW.md`](UI_REVIEW.md)。设计契约在 [`UI_DESIGN_PLAN.md`](UI_DESIGN_PLAN.md)。

## 摘要

| 严重度 | 数量 | 编号 |
|---|---|---|
| S1 功能破坏 | 2 | R2-01, R2-02 |
| S2 数据错误 / 高频路径 | 4 | R2-03, R2-04, R2-05, R2-06 |
| S3 一致性与契约 | 9 | R2-07 – R2-15 |
| S4 死代码 / 打磨 | 2 | R2-16, R2-17 |

优先级：**P0** 本轮必修（R2-01、R2-02）；**P1** 应修（R2-03 – R2-06）；**P2** 机会性（其余）。
其中 **R2-07 / R2-08 / R2-09 直接关联上一轮刚提交的 #27 / #28**，是"修对了主机制、但周边契约没跟上"。

## 问题清单

| # | 区域 | 问题 | 严重度 | 优先级 | 与上一轮关系 |
|---|---|---|---|---|---|
| R2-01 | 扩展弹窗 | `hintForError` 四个分支返回裸字符串，`renderMarkup` 把它们当节点对象消费，Firefox 四类常见错误的修复指引整句丢失 | S1 | P0 | 未覆盖的既存缺陷 |
| R2-02 | 外壳 | 模态滚动锁是电平触发，`lockCount` 只增不减，关闭面板后页面永久不可滚 | S1 | P0 | 未覆盖的既存缺陷 |
| R2-03 | 设置 | `updateSettings` 失败时回滚 `settings`，却不回滚 `globalSpeedLimitLabel` | S2 | P1 | #17 的残留 |
| R2-04 | 选择集 | 全量 `refreshDownloads` 不 prune 选择集；筛选/搜索切换也不 prune，批量操作可打向幽灵 id | S2 | P1 | 未覆盖 |
| R2-05 | 扩展测试 | 106 条测试全绿，但四个坏分支零断言，且 fake DOM 对 `createElement(undefined)` 不抛错 | S2 | P1 | 覆盖盲区 |
| R2-06 | 概览数字 | `counts` 基于全库，搜索后 chip 数字不联动；`scheduled` 筛选与 `canceled` 计数是孤儿 | S2 | P1 | #2/#3 的边界 |
| R2-07 | Sparkline | `axes[1].grid.stroke` 因 `show:false` 永不执行，§7 声称的"网格线换色"从未发生 | S3 | P2 | **#27 的遗留** |
| R2-08 | 弹窗 a11y | `aria-label` 挂在无 role 的 `<span>`；`!state` 分支不清属性；单行状态全文只在 `title` | S3 | P2 | **#28 的遗留** |
| R2-09 | 对话框 | Auth/Capture 无焦点移入/恢复/陷阱，且 `fly` 200 ms 滑动违反 §8；backdrop 注释与实现不符 | S3 | P2 | 未覆盖 |
| R2-10 | 命令面板 | Tab 被劫持切模式，模式切换不重置 `selectedIndex`，关闭不恢复焦点 | S3 | P2 | 未覆盖 |
| R2-11 | 列表行 | `role="button"` 只处理 Enter，不处理 Space | S3 | P2 | #19 的边界 |
| R2-12 | 展开面板 | `InlineSpeedLimit` 的 Enter 会经 keydown + 隐式 submit 两次 `commit`；input 无可访问名称 | S3 | P2 | 未覆盖 |
| R2-13 | Toast | 外层 `aria-live="polite"` 嵌套内层 `role="status"`，双层 live region | S3 | P2 | 未覆盖 |
| R2-14 | 失败反馈 | `error` / `isLoading` store 无消费者，刷新与添加失败静默；校验错误不在 live region | S3 | P2 | #18 的边界 |
| R2-15 | 可访问性 | 输入/复选框边界对比度 1.2–1.6:1，§9 承诺 ≥3:1 | S3 | P2 | 契约未量化验证 |
| R2-16 | 死代码 | `design-tokens.ts` 零引用且已漂移；`.list-head` 等 CSS 与 5 个工具函数无消费者 | S4 | P2 | #22 收尾不彻底 |
| R2-17 | 生命周期 | `initKeyboardShortcuts()` 返回的注销函数被忽略；`theme.subscribe` 模块级无注销 | S4 | P2 | 未覆盖 |

---

## P0 复现记录

### R2-01 — 诊断提示整句丢失（S1）

`browser-extension/popup.js:52-67` 的 `renderMarkup` 假定 `parts` 全是 `{tag, text}` 对象：

```js
for (const part of parts) {
  const el = document.createElement(part.tag);   // 字符串 → undefined
  ...
  else if (typeof part.text === "string") appendText(el, part.text);
  parent.appendChild(el);
}
```

而 `hintForError`（`popup.js:156-247`）有四个分支把**裸字符串**混进数组：`:204-219` (`no such native application`)、`:221-227` (`not found`)、`:229-236` (`access/denied/allowed`)、`:238-244` (`manifest`)。例如：

```js
return [
  "Firefox can't find a valid host manifest for this extension. Make sure ",
  { tag: "code", text: "~/.mozilla/native-messaging-hosts/..." },
  " exists and lists ", ...
];
```

其余六个分支（`background` / `blocked` / `is not a function` / `1006` / `!msg` / `default`）全是对象数组，所以只在 Firefox 原生宿主缺失、路径不可读、权限被拒、manifest 损坏这四类**最常见的真实故障**上炸开。

**端到端复现**（用仓库自带 harness，只加载 `popup.js`，注入 `get-status` 回复 `lastError: "No such native application com.app.dm.native"`，推进时钟 3 s）：

```
status     : "✗ DM host not running."
err-detail : "Browser said: No such native application com.app.dm.native~/.mozilla/native-messaging-hosts/com.app.dm.native.jsondm-grabber@dm-projectallowed_extensionscom.app.dm.native.firefox.json"
```

解释性文字全部消失，只剩 `<code>` 片段无分隔拼接，另有 5 个空的 `<undefined>` 元素（`createElement(undefined)` 在浏览器里合法但无内容）；`.hint` 里的修复指引同样是空的。用户看到的是一串路径，不知道要做什么。

**建议**：统一 `parts` 元素类型 —— 要么让 `renderMarkup` 认识字符串并 `appendText(parent, part)`，要么把四个分支的文字包成 `{tag:"b", text}`；同时让 `renderMarkup` 在 `part == null || typeof part !== "object"` 时显式报错，避免同类静默失败。

### R2-02 — 滚动锁泄漏（S1）

`src/App.svelte:740-746` 把锁写成**电平触发**：

```svelte
$: if (typeof window !== "undefined") {
  if ($showSettings || $showCommandPalette) lockBodyScroll();
  else unlockBodyScroll();
}
```

`lockBodyScroll()` 每次调用 `lockCount++`（`:723`），`unlockBodyScroll()` 每次只 `--`（`:728`）。只要两个 store 中**任一发生变化而条件仍为真**，就多记一次锁且没有对应的解锁。可达路径不止"双模态"：

- 命令面板里选 `Open settings`（`:1039` 只把 `showSettings` 置 true，随后 `CommandPalette.close()` 把 palette 置 false）：条件从真→真，`lockCount` 多 +1。
- 设置抽屉打开时按 `Cmd/Ctrl+K`（`:782`，`useKeyboardShortcuts.ts:43` 明确允许带修饰键的全局组合键在输入聚焦时触发）：两个模态同时为真，之后两次 Esc 各按 `:810-811` 的优先级关闭，条件再次从真→真。

结果：关闭所有面板后 `lockCount > 0`，`body`/`html` 上的 `overflow: hidden !important` 不解除，整个列表**永久不能滚动**。根组件 `onDestroy` 的兜底（`:822-832`）在正常生命周期里不会执行。

**建议**：改成边沿触发，例如缓存 `let wasOpen = false`，仅当 `open !== wasOpen` 时调用一次 lock/unlock 并更新 `wasOpen`；或者干脆把 `lockCount` 换成单个布尔 `isLocked`，锁的语义本就是"是否有任意模态打开"，不需要引用计数。

---

## 逐条详述（S2 / S3 / S4）

- **R2-03**（`src/lib/stores/settings.ts:28-42`）：`globalSpeedLimitLabel.set(...)` 在 `api.updateSettings(next)` 之前，`catch` 只 `settings.set(current)`。引擎实际保留旧限速，状态栏却一直显示新值直到重启。#17 修的是"启动时回填"，没修"写入失败回滚"。**建议**：在 `catch` 里用 `speedLimitLabel(current.speedLimitGlobal)` 复位，或把标签改为从 `settings` 派生。
- **R2-04**（`src/lib/stores/downloads.ts:84-125` / `:701-703`、`src/lib/components/BulkActionBar.svelte:70`）：`refreshDownloads()` 用服务端列表整体替换 `downloads`（`:118`），只有 `Removed` 事件才调 `pruneSelection`。服务端侧清理、重启或跨端删除不会发该事件；`BulkActionBar.fire()` 直接把 `Array.from($selectedIds)` 交给命令。另外 `ui.ts:167-175` 的注释把选择集定义为"visible selection"，但 `App.svelte:200` 的 `visible` 变化（筛选/搜索）并不会 prune，语义与实现不符。**建议**：`refreshDownloads` 成功后统一 `pruneSelection(new Set(list.map(d => d.id)))`；明确选择是否跨筛选保留，并把结论写回注释。
- **R2-05**（`browser-extension/test/integration.test.mjs:326-345`、`harness.mjs:667-669` / `:303`）：现有断言只覆盖 `background.scripts`（对象数组分支）与 `check-host.mjs`（`1006` 分支）；`nativeFails` 这个 mock 开关在全套件零引用；`FakeDocument.createElement(tag)` 对 `undefined` 生成 `<UNDEFINED>` 而不抛错。因此 R2-01 的回归永远不会被 CI 发现。**建议**：四个分支各补一条断言（检查解释性文字），并让 fake DOM 对非法 tag 抛错。
- **R2-06**（`downloads.ts:783-799`、`SmartFilters.svelte:14-45`、`App.svelte:200-225`）：`counts` 派生自全量 `downloads`，chip 取 `$counts`，而 `filterDownloads` 在筛选之后再叠加搜索。搜索到 1 条时 `All 6 · Active 2` 不变 —— 数字本身没算错，但没有说明"这是全库数"，与 §5「计数只在此处」的信息职责相冲突。同时 `FilterKey` 含 `scheduled`、`matchFilter` 有对应 case，却没有任何入口能设置它（`activeFilter.set` 仅出现在 `SmartFilters:53` 与 `App:1065` 的 `"all"`）；`counts.canceled` 无任何读取方。**建议**：要么让 chip 计数随搜索联动并标注口径，要么在文档里写死"计数为全库口径"；删掉 `scheduled` 孤儿分支或补一个 chip。
- **R2-07**（`src/lib/SpeedGraph.svelte:50-63`、组件注释 `:25-27`、`docs/UI_DESIGN_PLAN.md:221-223`）：`axes: [{show:false},{show:false, grid:{show:true, stroke: cssColor(...)}}]`。`node_modules/uplot/dist/uPlot.esm.js:4548-4550` 的 `drawAxesGrid()` 首行就是 `if (!axis.show || !axis._show) continue;`，所以 `:4697` 的 `grid.stroke(self,i)` **永远不会执行**。折线 `series.stroke` 的函数式取值是有效的（`:4207` / `:4261` 每次 `drawSeries` 重新求值），#27 的核心目标达成；但"line/grid colours"里的 grid 是死配置，注释与 §7 都多说了一句。**建议**：删掉无效的 `grid` 配置并修正两处措辞，或确实要网格线就把 `show` 打开（但那会改变 sparkline 的视觉）。
- **R2-08**（`browser-extension/popup.html:214-219`、`popup.js:117-139` / `:313`）：`#host-state` 是无 role 的 `<span>`，ARIA 不允许给 generic 元素命名，`setAttribute("aria-label", …)` 基本被 AT 忽略；`!state` 分支（`:120-124`）只改 `title`，不清 `aria-label`。可见文字 `Connected` / `Disconnected` 仍在，所以不算"只靠颜色"，但 §7 承诺的"详情可取回"只靠 `title`（悬停），键盘与读屏都不可靠。**建议**：给状态块一个明确 role（如 `role="status"` 但需避免 1.5 s 轮询重复播报，更稳妥的是 `aria-describedby` 指向 `#status`），并把 `#status` 的完整文本放进可聚焦/可读的容器。
- **R2-09**（`AuthDialog.svelte:311-313` / `:325`、`CaptureDialog.svelte:287-292` / `:327-338`、`Toast.svelte:20-27`）：两个 `role="dialog" aria-modal="true"` 都没有焦点移入、焦点陷阱与关闭后焦点恢复（Auth 的 `onMount` 是显式 no-op）；`transition:fly={{y:12,duration:200}}` 违反 §8「modal: opacity 120 ms; no slide」；Capture 的注释声称 backdrop 有 `role="button"+tabindex`，实际只有 `aria-hidden="true"`（注释与实现不符）。**建议**：统一焦点管理，把两个 `fly` 换成 `fade`。
- **R2-10**（`CommandPalette.svelte:48` / `:56-61` / `:63-86`）：`Tab` 被 `preventDefault()` 用来切 `command/download` 模式，键盘用户无法 Tab 出面板；`close()` 会重置 `selectedIndex`，但通过 Tab 或 `search-downloads` 命令切模式时不会重置，结果列表变短后 `results[selectedIndex]` 越界，高亮消失、Enter 无响应；关闭也不把焦点还给触发元素。**建议**：模式切换时 `selectedIndex = 0`，Tab 保留原生导航（或提供显式键），关闭时恢复焦点。
- **R2-11**（`DownloadRow.svelte:184-206` / `:251-254`）：`onKeydown` 只处理 Enter 与 Alt+方向键。`role="button"` + `tabindex="0"` 承诺了按钮语义，Space 却只会滚动页面。#19 修好了"Enter 冒泡双动作"，但没补齐按钮的另一半键盘契约。**建议**：增加 `e.key === " "` 分支并 `preventDefault()`。
- **R2-12**（`InlineSpeedLimit.svelte:20-31` / `:37-40` / `:45-60`）：`<form on:submit|preventDefault={commit}>` 里单输入框按 Enter 会同时触发 `on:keydown` 的 `commit()` 与隐式 submit 的 `commit()`，`commit()` 又没有 `editing` 守卫（还会叠加 `on:blur`），`set-limit` 可能下发两次；`aria-label` 放在 `form[role=group]` 上，input 自身只有 placeholder，无可访问名称。**建议**：`commit()` 开头加 `if (!editing) return;`，keydown 只负责 Escape，把名称移到 input。
- **R2-13**（`Toast.svelte:20` / `:24`）：外层 `role="region" aria-live="polite"` 里再放 `role="status"`（隐含 `aria-live="polite"`），同一条通知可能被播报两次。§9 说"Toasts are the live region"，用一个即可。
- **R2-14**（`downloads.ts:50` / `:119-121`、`App.svelte:965-966`）：`error` 与 `isLoading` 两个 store 只有写入没有 `$` 订阅（App 的 `error` 是另一个局部变量），`refreshDownloads` 吞掉异常后界面没有任何原因反馈；添加行的校验错误是普通 `<div class="err">`，不带 `role="alert"` / `aria-live`，而 §9 规定 Toast 才是 live region。#18 修好了"非法 URL 静默失败 + 清空输入"，但没有覆盖读屏用户。**建议**：失败时 `showToast({kind:"error"})`，或给 `.err` 加 `role="alert"`；删除或接线两个死 store。
- **R2-15**（`src/app.css:19/21/97/99` / `:393` / `:519` / `:549-552`）：按 WCAG 相对亮度实算，`input/select/textarea` 的 `--color-border`（浅 0.10 / 深 0.08 alpha）对比度 1.23 / 1.27:1，复选框 `--color-border-strong`（0.20 / 0.15）1.53 / 1.59:1，均远低于 §9 自定的 ≥3:1。控件背景 `--color-surface-solid` 与窗口底色也几乎同色，无法靠填充区分边界。**需要说清的两点**：① 这些是装饰性发丝线，WCAG 1.4.11 对纯装饰分隔线不强制 3:1；但空输入框/未选中复选框的边界属于"识别控件所需信息"，通常应达标。② §9 要求的 focus-visible ring 本身是达标的 —— `:550` 用的是实心 `--color-primary`（浅 5.30:1 / 深 7.75:1），只有 `:focus` 的辅助 `--color-border-focus` 偏淡（浅 2.12:1），不影响主要焦点指示。**建议**：把控件边界单独提到 ≥3:1 的实色，装饰分割线保持发丝。
- **R2-16**（`src/lib/design-tokens.ts` 全文件、`app.css:76/151/208/808-829`、`ui.ts:124/130`、`utils/formatters.ts:62/74/115/126`、`fuzzySearch.ts:89`）：`design-tokens.ts` 在 `src/` 内零 import，却仍按 §10 被称作 "JS mirror"，且已漂移：断点 `lg 900px / xl 1280px`（`:167-168`）对应 `app.css:920/928` 的 `1180 / 980px`；导出 `rowHeight → --size-row-height`（`:189`）对应实际 `--size-row`（`:208`）；`shadow.glow` 硬编码深色 rgba（`:145`）而 `app.css:187` 已改成 `var(--color-primary-dim)`。此外 `.list-head`、`--divider-on-surface`、`globalSpeedLimit`、`clipboardMonitorEnabled`（只 set 不读）、`fmtRelativeTime` / `fmtDate` / `debounce` / `highlightMatches` 均无消费者。**建议**：要么删掉 mirror，要么让它成为真源并生成 `app.css`；顺手清掉死代码。
- **R2-17**（`App.svelte:818`、`ui.ts:39-54`）：`initKeyboardShortcuts()` 返回注销函数但调用处直接丢弃，全局 `keydown` 监听在组件销毁后仍挂着；`theme.subscribe` 与 `mediaQuery.addEventListener` 同为模块级、无注销（模块单例下通常无害，但 HMR 会叠加）。**建议**：`onDestroy` 里调用返回的注销函数。

---

## 量化核查：对比度（本轮实测）

按 WCAG 2.x 相对亮度公式计算（抽样 17 组文字/分类色，**全部 ≥4.5:1**）：

| 组合 | 对比度 | 组合 | 对比度 |
|---|---|---|---|
| light faint / bg | 4.70 | dark faint / panel | 5.33 |
| light faint / panel | 5.12 | dark muted / bg | 6.77 |
| light muted / bg | 5.30 | dark text / bg | 14.32 |
| light text / bg | 15.31 | dark accent / panel | 7.75 |
| light accent / panel | 5.30 | light category document | 6.04 |
| light warning / panel | **4.57**（最低，勉强达标） | light category binary | 6.36 |
| light danger / panel | 5.43 | dark category video | 5.34 |
| light success / panel | 5.32 | dark category binary | 5.74 |
| dark faint / bg | 5.82 | focus-visible outline（实心 accent） | 5.30 / 7.75 |

**结论**：§9 的"文字 ≥4.5:1"和"focus-visible 双主题可见"成立；修复前不达标的是 R2-15 所述的**控件边界**（1.2–1.6:1）。上一轮把浅色 `--faint` 提到 `#676E7C` 确实解决了对比不足。

**阶段 3 修复后复算**：控件边界改用 `--control-border`（别名指向 `--color-faint`），浅色 5.12:1（panel）/ 4.70:1（bg），深色 5.33:1 / 5.82:1；`:focus` 边框改为实心 accent，5.30:1 / 7.75:1。四组全部 ≥3:1，§9 的承诺现在成立。

## 已验证通过（本轮复跑）

| 检查 | 结果 |
|---|---|
| `npm run check`（沙箱需 `NODE_OPTIONS=--unhandled-rejections=warn`） | 0 errors / 0 warnings（三个阶段均复跑） |
| `node scripts/check-css-vars.mjs` | 阶段 3 后 OK，712 处 `var()` / 130 个属性（删掉死 token 后相应减少） |
| `node --test test/*.test.mjs` | 阶段 3 后 109 passed / 0 failed，十个 hint 分支全部有断言 |
| `node test/mutation-check.mjs` | 27/27 caught |
| `vite build` | 三个阶段均成功（沙箱配置跳过两个 WASM-only import-analysis 插件） |
| `cargo test -p dm-engine` | 三个阶段均 118 passed / 0 failed |
| `git status` | 每个阶段提交后干净；审查基线为 `a94937e` |
| #27 折线换色主机制 | 成立：uPlot `cacheStrokeFill` 每次 `drawSeries` 求值；`applyTheme` 先改 `data-theme` 再 `.set()`；`redraw` 有 `xs.length > 0` 守卫 |
| #28 状态点主机制 | 成立：两条新回归测试分别钉住断开 / 连接态的 `className`、文字与 `title` |
| 上一轮 30 项 | 未发现"标为已修但实际未修"的回退 |

三个阶段均按要求"改完 → 全套验证 → 单独提交"推进，每阶段一个独立 commit（P0 为 `37bc831`，P1 为 `c35184c`，P2 为 `cba20cf`）。`vite build` 在沙箱内使用 `.pi/` 下 gitignored 的临时配置（跳过两个需要 WASM 的 import-analysis 插件），不入库。**截图级双主题目检已完成**（2026-11-15 补 Blink/WebKitGTK，2026-11-16 补真实 `dm-tauri` 窗口），见下节「视觉验证」。

## 明确排除的疑点

- `setHostState(null, …)` 的 `aria-label` 残留：`lastState` 是单调 `null → 对象`，实际不会在设过属性后回到 `null`，属代码不对称隐患而非可达 bug（属性挂错 role 的问题仍成立，见 R2-08）。
- `theme.subscribe` / `mediaQuery` 无注销：模块级单例，正常生命周期内不会泄漏，仅 HMR 叠加（R2-17 降为 S4）。
- `trash` 动作（`App.svelte:397`）未显式 `refreshDownloads` / `clearSelection`：依赖后端是否发 `Removed` 事件，未读到证据前不判定为缺陷。
- Capture / Auth / dropveil 未纳入滚动锁条件：需确认这些组件是否自行锁背景，列为待确认；R2-02 的结论不依赖这一项。
- 列头 `top: calc(48+38)` 与筛选条 `top: var(--size-topbar)`：当前数值吻合，仅在筛选条高度偏离 38 px 时失效。

## 修复顺序建议

1. **R2-01**：统一 `hintForError` / `renderMarkup` 的节点协议，让非法节点显式报错。
2. **R2-02**：滚动锁改边沿触发或单布尔。
3. **R2-05**：补齐四个分支的回归断言 + fake DOM 对非法 tag 抛错；按 `AGENTS.md` 约定，修扩展 bug 时同步在 `test/mutation-check.mjs` 加变异。
4. **R2-03 / R2-04**：状态栏限速标签回滚、全量刷新后 prune 选择集。
5. **R2-07 / R2-08**：把 #27 / #28 的注释与 §7 措辞对齐实现（这是最小改动，可并入前四项一起做）。
6. **R2-09 – R2-15**：键盘与 a11y 批次（Space、对话框焦点、Tab、Toast、控件边界）。
7. **R2-16 / R2-17**：死代码与生命周期收尾。

修复后按 `AGENTS.md` 的门禁复跑：`npm run check`、`cargo test -p dm-engine`、`node --test test/*.test.mjs`、`node test/mutation-check.mjs`、`node scripts/check-css-vars.mjs`；真实 WebView 的双主题目检已在下节完成。

## 处理进度（分阶段，每阶段独立验证并提交）

| 阶段 | 范围 | 状态 | 阶段验证 |
|---|---|---|---|
| 1 | P0：R2-01、R2-02 | 已修 | `svelte-check` 0/0；CSS vars OK；扩展 107/107；变异 26/26；`vite build` 82 modules；`cargo test -p dm-engine` 118 passed / 0 failed |
| 2 | P1：R2-03 – R2-06 | 已修 | `svelte-check` 0/0；CSS vars OK；扩展 109/109；变异 27/27；`vite build`；`cargo test -p dm-engine` 118 passed；额外运行时验证计数/清理逻辑 |
| 3 | P2：R2-07 – R2-17 | 已修 | `svelte-check` 0/0；CSS vars OK（712/130）；扩展 109/109；变异 27/27；`vite build`；`cargo test -p dm-engine` 118 passed；控件边界对比度复算全部 ≥3:1 |

**阶段 1 实现要点**：`renderMarkup` 现在把字符串当文本节点、并对非法节点抛 `TypeError`；新增一个覆盖四个坏分支的回归测试与一条变异。`App.svelte` 的滚动锁从 `lockCount` 引用计数改为幂等的 `scrollLocked` 布尔，只有 false↔true 才动 DOM，`onDestroy` 用 `setBodyScrollLocked(false)` 兜底。

**阶段 2 实现要点**：`updateSettings` 失败时同步复位限速标签；`refreshDownloads` 全量替换后 prune 选择集，`ui.ts` 注释改为明确的"跨筛选保留、随权威列表清理"语义。chip 计数与 smart folders 改由 `searchScope`（仅按搜索文本收窄）派生，状态栏继续用全库 `counts`；删除不可达的 `scheduled` FilterKey/case 与无人读取的 `counts.canceled` / `counts.scheduled`，§5 补上口径说明。fake DOM 现在对非法 tag 抛错，十个 `hintForError` 出口全部有断言，并新增一条针对该防线的变异。运行时验证：库内 4 行、搜索 `pdf` 后 chip 为 `All 1 / Done 1`，状态栏仍为 `All 4`。

**阶段 3 实现要点**：
- R2-07：删掉 `SpeedGraph` 里因 `axes.show:false` 永不执行的 `grid` 配置，组件注释与 §7 改为只承诺折线换色。
- R2-08：`#host-state` 改为 `role="group"`（generic `<span>` 上的 `aria-label` 会被 AT 忽略），`aria-describedby` 指向可聚焦 `tabindex="0"` 的 `#status`；`setHostState` 统一处理 `!state` 分支，且只在文本真的变化时改写 ARIA 属性，避免 1.5 s 轮询重复播报。
- R2-09 / R2-10：新增 `src/lib/utils/focusTrap.ts`，两个对话框与命令面板共用（焦点移入、Tab 陷阱、关闭后焦点恢复）；Auth/Capture 的 `fly` 改 `fade` 120 ms，backdrop 注释与实现对齐；命令面板不再劫持 Tab，模式切换统一走 `setMode()` 重置 `selectedIndex`，输入框补 `aria-label`。
- R2-11 / R2-12 / R2-13 / R2-14：行支持 Space；`InlineSpeedLimit` 加 `editing` 守卫 + `preventDefault` + input 可访问名称；Toast 去掉嵌套 live region；`error` / `isLoading` 两个死 store 删除，刷新失败改为一次性 Toast，校验错误加 `role="alert"`。
- R2-15：新增别名 `--control-border: var(--color-faint)` 用于输入/复选框边界，`:focus` 边框改实心 accent；§9 区分"控件边界 ≥3:1"与"装饰发丝线豁免"。
- R2-16 / R2-17：删除零引用且已漂移的 `design-tokens.ts`、`.list-head`、`--divider-on-surface`、`--border-focus`、`globalSpeedLimit`、`clipboardMonitorEnabled`、`isMac` 及 6 个未使用工具函数；`initKeyboardShortcuts()` 的注销函数在 `onDestroy` 生效。模块级 `theme.subscribe` 属单例设计，保留。
- 因 `setHostState` 重构，同步更新了 `mutation-check.mjs` 中该变异的匹配串，避免 stale pattern。

**仍待确认（不影响已完成项）**：`trash` 动作是否依赖 `Removed` 事件、Capture/Auth/dropveil 的背景滚动、列头 sticky 偏移对 38 px 的硬编码依赖。截图级双主题目检已完成，见下节。

## 视觉验证（2026-11-15 起，2026-11-16 补完 Tauri 本体）

第一轮记录里"沙箱无法启动 Chromium/Firefox"的结论**已不成立**：当前环境有 `DISPLAY=:0`（X.Org 24.1.14，2560×1440）、`google-chrome 157`、`firefox`、`libwebkit2gtk-4.1`、ImageMagick 与 PyGObject/WebKit2。本轮在两种引擎里渲染了**最终构建产物**（`cba20cf` 之后的源码，含本轮修正）：

| 引擎 | 方式 | 结果 |
|---|---|---|
| Blink | `google-chrome --headless=new --screenshot` + `agent-browser`（交互） | 1440×900 深/浅、1024×768 浅色、选中 2 行、设置抽屉、命令面板、弹窗深/浅；控制台 0 error |
| WebKitGTK 4.1 | Tauri 同引擎：PyGObject `WebKit2.WebView` 窗口加载同一 URL，`convert x:<window-id>` 抓屏 | 1440×900 深/浅，与 Blink 布局一致 |
| Tauri 本体 | `target/release/dm-tauri` 在 `DISPLAY=:0` 下启动 | 初步结论见下节：用普通 `cargo build --release` 编出来的是 **dev 配置**的二进制，不读 `dist/`；补 `--features tauri/custom-protocol` 重编后，窗口渲染的是**当前**界面（深浅各一张），空库还会走到 "Nothing here yet" 空状态 |

**逐项核对（运行时证据，不只是截图）**
- 双主题：浅色 `bodyBg=rgb(244,245,247)`、深色 `#17191e`；两主题下 6 行账本、筛选计数、分类 chip、状态栏 sparkline、demo 标记均完整；1024×768 下 ETA 列按 §5 在 1180px 断点正确消失，Speed 列保留。
- #27（sparkline 主题响应）：`system` 模式下用 `prefers-color-scheme` 翻转 OS 偏好，`data-theme` 从 `light → dark`，canvas 上第一个有内容的像素从 `rgb(13,122,94)`（=`#0d7a5e`）变为 `rgb(64,200,161)`（≈`#41c9a2`）。折线确实立即重绘换色。
- R2-15（控件边界）：浏览器实测未选中 `input.cb` 的 `border-color = rgb(103,110,124)`，正是 `--color-faint #676e7c`；`:focus` 用实心 accent。
- R2-02（滚动锁）：复现原泄漏路径 —— 从命令面板选 `Open settings`，关闭后 `body.style.overflow` 与 `html.style.overflow` 均为 `""`（已解锁）。
- R2-09 / R2-10（焦点与面板）：打开命令面板后 `document.activeElement` 为 `INPUT[aria-label="Type a command"]`；9 条结果；面板关闭后焦点恢复。
- R2-14（校验反馈）：输入 `not-a-url` + Enter → `.err[role="alert"]` 显示 "Enter a complete http(s) URL."；换成合法 URL 后 Add 由 `disabled=true / opacity .45` 变为 `disabled=false / opacity 1`（`bg=rgb(13,122,94)`、白字）。先前截图里 Add 偏暗是禁用态，不是对比度缺陷。
- 批量操作条：1440×900 放大核对为 `2 selected · 2 active 3.4 GB / 5.2 GB | Pause ▶ Resume ⇄ Limit ⇄ Merge 🗑 Trash 🗑 Remove`，无重叠、浮在状态栏上方。
- 扩展弹窗：深/浅双主题下标题行 `DM Grabber` + 红点 `Disconnected`、单行状态、内联错误详情、主按钮与两个开关均按 §7 排布（纯浏览器中 `chrome.*` 不存在，故显示的是断连诊断态）。

**本轮视觉验证发现并修复的缺陷**：命令面板 footer 仍写着 `tab switch mode`，但阶段 3 已把 Tab 还给原生导航（R2-10）。已改为 `tab move focus`，重建后实测 footer 文本为 `↑↓ navigate / ↵ select / tab move focus / esc close`。

**沙箱构建的一个必要说明**：容器把 `RLIMIT_AS` 限到 4 GiB，V8 无法为 WASM 分配内存，因此 `vite build` 必须跳过 `vite:build-import-analysis` 与 `vite:dynamic-import-vars` 两个依赖 es-module-lexer WASM 的插件。跳过前者会让产物引用未定义的 `__VITE_IS_MODERN__` 而**加载即崩**（本轮首次截图全黑的原因）。验证用配置（`.pi/vite.sandbox.config.mjs`，gitignored）现在补了 `transformIndexHtml` 注入 `window.__VITE_IS_MODERN__=true`，产物开箱可运行；这只影响验证副本，不改产品配置，正常环境下的 `npm run build` 由 Vite 自行注入。

### Tauri 本体（2026-11-16 补做）

上一轮把"窗口里是 v0.7 旧界面"归因于"编译时嵌入了旧 `dist/`"。方向对，但机制只说了一半，而且它导致了一条错误的操作结论（"重编一次就会好"）。真实机制是：

- `tauri` 的 `build.rs` 只有一个判据 —— `let custom_protocol = has_feature("custom-protocol"); let dev = !custom_protocol;`，然后 `cargo:dev={dev}`，`tauri-build::is_dev()` 读的正是 `DEP_TAURI_DEV`。所以**普通 `cargo build --release` 编出来的是 dev 配置的二进制**：它去加载 `devUrl`（`http://localhost:5173`），压根不读 `dist/`。窗口里那行 `Could not connect to localhost: Connection refused` 就是它，和资源新旧无关。
- 生产二进制必须显式带特性：`cargo build --release -p dm-tauri --features tauri/custom-protocol`。`tauri build` 自己会传（CLI 二进制里同时含 `custom-protocol` 与 `tauri/custom-protocol` 两个串，说明它会按应用清单有无该特性二选一）。9 月那个旧二进制是 CLI 构建的，嵌的是当时的 `dist/`，所以显示 v0.7 界面 —— 那是"旧产物 + 正确构建方式"，不是"构建方式错了"。
- 按上面的命令重编（`dist/` 已替换为当前产物）后启动，窗口渲染的就是当前设计：账本行、`All / Active / Queued / Paused / Done / Errors` 筛选条、行内粘贴框与 `Default (no category)` / `More` / `Add`。
- 空库：`dm.sqlite` 的 `downloads` 为空，于是这次同时**首次验证了零数据空状态** —— `Nothing here yet` + `Ctrl K / Ctrl N / drop` 三个提示，状态栏 `0 active · — · 0 B downloaded · limit Unlimited · 0 done`。此前的截图全部是 demo 数据的 6 行，空状态一次都没出现过。
- 双主题：WebView 里 `dm-theme` 持久化为 `system`（`localstorage` 实测值 `system`），于是按 GTK 的 `prefers-color-scheme` 解析 —— 默认浅色；加 `GTK_THEME=Adwaita:dark` 再启动即深色。`system` 分支在**真实 Tauri 引擎**里同样成立，不只是 Chrome 里成立。

两个环境坑（与产品无关，但会让人误判成缺陷）：

- 沙箱把 `http_proxy/https_proxy` 指向 `127.0.0.1:38139`，WebKit 会把 `http://tauri.localhost` 的整页请求也交给这个代理，结果整页变成 `upstream connect failed: Connection refused (os error 111)`。启动时 `env -u http_proxy -u https_proxy …` 即恢复。这是第一次重编后窗口全白的原因。
- 这台机器上同时跑着真正的 Free Download Manager，抓屏必须按 `_NET_WM_NAME` 精确匹配：`_NET_CLIENT_LIST` 里 `0x2000011` 是 FDM，`0x2200003` 才是 DM。按"第一个窗口"取会抓到别人的界面。

**截图**（`docs/ui/verified-*.png`）：`verified-app-dark/light/1024/selection/palette/settings.png`（Blink）、`verified-webview-light/dark.png`（WebKitGTK 静态页）、`verified-popup-light/dark.png`（扩展弹窗）、`verified-tauri-light/dark.png`（真实 `dm-tauri`，1180×760 窗口原始尺寸）。

**仍无法覆盖**：带真实后端数据的设置面板（纯浏览器下只有 "Settings unavailable in preview mode."，而 Tauri 窗口内无法驱动交互，只能看首屏）；WebView 内的键盘 / 焦点 / 面板交互（没有 devtools 自动化通道，这部分只能在 Blink 里用 `agent-browser` 验）；真实屏幕上的字体与像素密度主观观感。
