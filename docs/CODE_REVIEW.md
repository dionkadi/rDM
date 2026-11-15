# DM 全项目代码审查报告

审查日期：2026-11-11 · 范围：`crates/engine`、`crates/native-host`、`src-tauri`、`src/`（Svelte/TS）、`browser-extension/`
方法：逐文件精读 + 两路并行深度审查（Rust 引擎 / 壳+前端+扩展）+ **主审查对每条 P0/P1 回查源码** + 客观信号（`cargo clippy`、`cargo test`、`svelte-check`、扩展 104+24 测试、`check-css-vars.mjs`、Node WebSocket 实测帧行为）。
准则参照：Rust API Guidelines / Rust idioms 审查清单（错误类型、锁纪律、可见性、`&str`/`&[T]`）、RFC 6455、Tauri 2 安全清单（最小 capability、CSP、命令面契约）、Svelte store 最佳实践、AGENTS.md/README 职责划分标准。

**基线信号**：`cargo test -p dm-engine` 65 通过 / 0 失败；扩展 104 测试 + 24 mutation 全过（mutation-check 在 CI 真实执行）；`svelte-check` 0 错误 / 1 警告（存量 a11y）；`check-css-vars.mjs` 通过。

---

## 一、总评

这是一个**明显高于平均水准的个人项目**：MV3 生命周期、事件桥序列化、取消安全、变异测试、CSS 变量闸都是生产级的；"UI 无关引擎 + Spawner 注入 + 薄命令面"的分层干净。问题集中在三类：(1) **自研 WebSocket 帧格式违反 RFC 6455**（P0，有实测断连证据）；(2) **命令契约两端漂移**（set-priority 字段错位、AuthSpec `none` 变体缺失、proxy 参数悬空）；(3) **存储/命名一致性**（`save_path` 不随探测改名更新、`save_download` 静默吞错）。

| 维度 | 引擎 (crates/engine) | 壳+前端+扩展 |
|---|---|---|
| 设计原则 | 4 / 5 — 分层清晰；RequestPlan 职责过载 | 4.5 / 5 — 四方边界干净、命令全薄委托；design-tokens 双轨扣分 |
| 设计模式 | 3.5 / 5 — Spawner 注入优秀；扩展点全靠 if/else 硬编码 | 4 / 5 — 事件桥 1:1 干净；三处命令契约两端不一致 |
| Rust idioms | 3.5 / 5 — 锁纪律好；`clone()` 战术性滥用、错误类型不规范 | 3.5 / 5 — async/sync 分界正确；ws.rs 帧格式、nonce 时钟误用、双文件句柄、tray expect |
| 最佳实践 | 3.5 / 5 — 测试强；`let _ = save_download()` 静默吞错 | 4 / 5 — any 滥用≈0、清理到位；detail 解构 bug、快捷键覆盖式注册 |
| 可维护性 | 4 / 5 — "为什么"注释出色；魔法数字散落 | 4 / 5 — CSS 变量闸 + mutation 测试是亮点；死 store/死函数待清 |
| 可扩展性 | 3 / 5 — 新协议/校验算法/镜像策略均需多点手术 | 4 / 5 — 新命令 3–4 处对称改动；MV3 传输层可替换 |

---

## 二、P0 — 正确性风险（均经主审查回查源码确认）

### P0-1 自研 WS 服务端帧错误置位 MASK bit → 浏览器客户端 1006 断连 ✅已核实
`src-tauri/src/ws.rs:317-332` `write_text_frame` 写 `[0x81, 0x80|len]`，注释称"Apply the mask bit (0x80) so the client knows to expect unmasked payload"——这是对 RFC 6455 §5.1 的误读：MASK 位语义是"payload 已掩码且后跟 4 字节 key"，**server MUST NOT mask**。`write_close_frame`（ws.rs:340-344）同样错误（`[0x88, 0x80|2]`）。实测（Node 内置 WebSocket 客户端，与浏览器同为 undici 实现）：收到该帧即 error + close(1006)，消息无法送达。而 `native_host.rs:225` 每收一条消息都会回 ack `{"ok":true}`（非法帧）→ Chromium WS 通道每条消息后都在断，靠扩展的重连/退避硬撑。两个 Rust 测试（ws.rs 约 455、586-589 行）把错误行为固化成了期望值。
**修法**：`write_all(&[0x81, len as u8])`（≤125）/`[0x81, 126]`/`[0x81, 127]`，close 帧写 `[0x88, 2]`；修正注释与两个测试断言（mask bit 为 0）；加一条真实 WS 客户端集成测试。
**注**：AGENTS.md 曾断言"浏览器形态客户端验证过帧完好"——与本实测矛盾，修完后应以真实客户端回归为准。

### P0-2 探测改名后 `save_path` 不跟随 → 磁盘文件名与 UI 永久失同步 ✅已核实
`src-tauri/src/commands.rs:90-108` 在 add 时用 URL 猜文件名并固化 `save_path = dir.join(fname)`；无路径段的 URL（`…/download?id=123`）落为 `download.bin`。探测拿到 `Content-Disposition` 后，`protocol.rs:357` 与 `task.rs:276-278` **只更新 `d.filename`，从不重算 `d.save_path`**；收尾 `task.rs:635` 的 `.part → save_path` 原子改名因此写到占位路径。结果：UI 显示 `real-name.iso`，磁盘上是 `download.bin`。
**修法**：`filename` 被探测/HLS 重命名更新的每一处（`protocol.rs:357`、`task.rs:276`、`task.rs:1633`）同步 `d.save_path = parent.join(&d.filename)`（对齐 `model.rs::rename_with_extension` 的既有做法）。
**回归测试**：无路径段 URL 的端到端用例，断言 `save_path` 文件名 == `filename`。

---

## 三、P1 — 重要改进

### 壳 / 前端 / 扩展
1. **set-priority 动作取错 detail 字段，优先级永远是 undefined** ✅已核实。`DownloadRow.svelte:213` dispatch `{ type:"set-priority", id, priority }`，`App.svelte:290` 解构 `const { type, id, limit } = e.detail` 后 `setPriority(id, limit)` —— `limit` 是 undefined。行内优先级下拉实际不可用。**修**：解构出 `priority` 并传递；补分发单测。
2. **`HeadersAuth` 的 `{kind:"none"}` 在 Rust 侧不存在，"无认证"保存必失败** ✅已核实。`api.ts:228` 定义了 `kind:"none"`，`AuthDialog.svelte:268` 在选 "None" 时发送它；Rust `AuthSpec`（`model.rs:275-279`）只有 basic/bearer/digest，`tag="kind"` 反序列化直接 Err——用户无法通过对话框清除已有认证。**修**：`authPayload()` 对 none 返回 `null`，或 Rust 增加 `None` 变体。
3. **native_host nonce 用 `Instant::now().elapsed()`，值恒≈0，去重失效** ✅已核实（`native_host.rs:403`）。nonce 退化为 `"{~0}-{url}"`，前端 `seenCaptureNonces` 会把同一 URL 的第二次合法捕获误判为重复而丢弃。**修**：`SystemTime::now().duration_since(UNIX_EPOCH).as_nanos()` 或进程内 `AtomicU64` 自增。
4. **`addDownload` 传 `proxy` 参数但 Rust `add_download` 无此参数**（`api.ts:42,64` vs `commands.rs:79-88`）：类型承诺的按下载代理从未生效。**修**：Rust 端加 `proxy: Option<String>` 接入 `d.proxy`（model.rs 已有字段），或删掉 TS 参数。
5. **`csp: null` 关闭 CSP**（`tauri.conf.json:25`）：WebView 无任何约束。**修**：最小 CSP（`default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'`），接入 updater 前收紧 `connect-src`。
6. **`opener:allow-open-url` 权限无调用方**（`capabilities/default.json:23`）：纯多余攻击面，删除。

### 引擎
7. **`let _ = ctx.storage.save_download(&d)` 全面静默**（`task.rs:228/331/543/649/782/1371…`）：磁盘满/锁死时 UI 显示 Completed，重启后进度回滚。**修**：log 警告；finalize 失败降级为 `Error("completed but could not persist: …")`。
8. **`serde_json::Error` 泄漏进公开 `StorageError`**（`storage.rs:14`）：违反 C-GOOD-ERR，下游无法编程匹配。**修**：包装为 `DataCorrupt(String)`。
9. **`build_client` 的 `unwrap_or_else(|_| Client::new())`**（`task.rs:213`）：坏代理配置静默回落**无代理裸客户端**，行为与用户配置不符。**修**：返回 `Result`，坏代理显式报错。
10. **`RequestPlan` 职责过载**（`protocol.rs:282-391`）：探测 + 建图 + 命名 + 失败分类 + 镜像切换五件事一个函数 110 行。**修**：拆 `probe` / `plan_chunks` / `resolve_name`，镜像切换抽 `RetryPolicy`（`TODO.md` 的 site_rules 正好需要）。
11. **`.m3u8` 拒绝路径不一致**（`task.rs:583-597`）：HLS 分支拒绝后普通 GET 仍抓下 m3u8 文本标 Completed。**修**：与 `add()` 的 rename 契约对齐，统一"拒绝并报因"。
12. **`TaskState::progress()` 每 150ms 全量 `download.lock().clone()`**（chunks+headers 深拷贝，`task.rs:331-343`）。**修**：增量上报（bytes/status/speed）。
13. **测试盲区**：镜像故障转移、`verify_chunks_against_disk`、跨重启重排、SAMPLE-AES 拒绝、自定义 headers 均无测试；`fn hlp()` 测试助手 H1/H2 常量反转。
14. **`native-host` 的 `looks_like_downloadable_url`**（`main.rs:242-244`）：`http://a/`（空路径）被静默丢弃，与"dumb pipe、拒绝要出声"哲学矛盾。

---

## 四、P2 — 打磨项（精选，均出自两路审查并经抽查）

**壳 / 前端**
- `commands.rs:642-671` 长 doc 属于 `trash_download` 却挂在 `trash_paths` 上，两段注释粘连。
- `notify_on_complete` 是 `notify` 的纯别名且 snake_case 命名违反 api.ts 约定（`commands.rs:431-433`）——删。
- `design-tokens.ts` 的 `generateCSSVariables`/`getToken` 全项目零调用（死代码）；保留 `tokens` 常量即可，或让 `check-css-vars.mjs` 反向校验 TS/CSS 值一致。
- `useKeyboardShortcuts.ts:16-24` 模块级 `shortcuts` 覆盖式赋值 + `App.svelte:824` 丢弃 `initKeyboardShortcuts` 的清理函数——第二个注册方会静默顶掉全局快捷键。
- `ui.ts:84-91` 死 store：`sortBy/viewMode/showAddDialog/showOnboarding` 无消费者。
- `AuthDialog.svelte:343` a11y 警告（label 未关联控件）——`npm run check` 唯一警告，改 `role="radiogroup"` + `aria-labelledby`。
- `logging.rs:102-120` 双文件句柄（guard 开一个、shim 又 append 一个）+ 216 行 `_FLUSH_INTERVAL_TYPE` 死常量。
- `tray.rs:20` `expect` 在托盘构建路径：图标缺失整个应用 panic，应降级为日志。
- Svelte 小项：body scroll lock 靠 `$:` + onDestroy 兜底（模式能用但易错）；速度监控 1s 轮询与 Progress 事件双通道。

**引擎 / native-host**
- `index.ts` 公开 20+ 个无文档内部类型（`RunContext`/`TaskState`…），违背 C-HIDDEN；`error.rs` 181 行死代码。
- `Download::new` 与测试助手 `new_download` 字段漂移；`pair_kind` 未序列化（重启丢配对）。
- `chrono::Utc::now()` 直调使时间不可注入（重试窗口、TTL 测试要真实等待）；`Config::load()` 静默吞读盘失败为 default。
- `scheduler` 线性扫 per_download 链表（O(n)/acquire，量级小）；`--color-scrollbar` light 主题 alpha 偏弱；DropdownMenu 动画 24 步 JS 内插可换 CSS transition。
- `manifest.json` 缺 `action.default_icon`；`background.js` 的 WS 无 token 认证（本机任意进程可仿冒 host，建议握手 token）。

---

## 五、做得好的（保持）

1. **Spawner 注入**：引擎零 `tokio::spawn` 直调，非 Tokio 线程下可正确驱动。
2. **取消安全**：`select!` + `cancelled` 检查防丢唤醒；`kill_on_drop(true)` 把超时变安全。
3. **锁纪律**：全部 `std::sync::Mutex` 短临界区，`.await` 前 clone-then-drop。
4. **MV3 卫生**：`onCreated` 顶层同步注册、`lastError` 同 tick 读取、`storage.session` + 400ms 防抖、install-time 过滤、`ignoredUrls` TTL 防回环。
5. **变异测试（mutation-check.mjs）**：24 个"重新引入历史 bug 必须被抓住"的自检且在 CI 真实执行——比覆盖率诚实得多。
6. **CSS 变量闸（check-css-vars.mjs）**：抓住"未定义 var() 静默丢声明"这一 CSS 最坑失效模式。
7. **capabilities 大体最小** + 事件桥 `#[serde(tag="kind")]` 与 `types.ts` 逐字对应。
8. **注释讲"为什么"**（稀疏文件 `resize: 1px`、AMF object-end 跳过、WebView2 async 分界引用 tauri#9453）是教科书级。
9. **HLS 模块**：纯解析 + `buffered` 有序并发 + per-segment resume sidecar + 拒绝路径全带理由。
10. **前端 store 边界**：持久化只有 `$theme`；`removedIds` 防幽灵行与引擎删除语义互相咬合。

---

## 六、本次会话已顺带修复（任务 1/3 内）

| 问题 | 归属 | 状态 |
|---|---|---|
| AGENTS.md 425 行叙事膨胀、与 README 职责纠缠 | 文档 | ✅ 重构为 130 行（归属表置顶、规则保留、故事压缩） |
| `UI_DESIGN_PLAN.md` 被删但 AGENTS/README 仍引用 | 文档 | ✅ 以 v2 设计规范重建 `docs/UI_DESIGN_PLAN.md` |
| README 过时内容（测试数、发布示例版本） | 文档 | ✅ 修正并吸收发布流程 |
| `design-tokens.ts` 与 `app.css` 数值脱节（280 vs 248px 等） | 前端 | ✅ 同步，声明 app.css 为唯一真源 |
| README 宣称的 demo 模式实际不可用（`refreshDownloads` 吞错致 catch 永不触发） | 前端 | ✅ 改为先探测 `api.listDownloads()` |
| Google Fonts 运行时依赖（不可达时回退 Georgia，观感骤降） | 前端 | ✅ 系统字体栈 + tabular-nums |
| StatusBar 永远为 0 的 "Up 0 B/s" 段、三处聚合指标冗余 | 前端 | ✅ 单一归属（status bar + activity strip） |

## 七、Top-10 优先改进清单

1. **P0-1 修 ws.rs 帧 mask bit**（去掉 0x80、修注释与固化错误的测试、加真实 WS 客户端回归）
2. **P0-2 `save_path` ↔ `filename` 同步**（探测/HLS 改名处重算 save_path）
3. **P1-1 set-priority 传 `priority` 而非 `limit`**（App.svelte:290）
4. **P1-2 AuthSpec `none` 契约**（none → `null`，或 Rust 加变体）
5. **P1-3 nonce 换 SystemTime / AtomicU64**
6. **P1-7 `save_download` 失败要出声**（log + finalize 降级 Error）
7. **P1-4/5/6** 契约收口三连：proxy 参数落地、最小 CSP、删多余 `open-url` 权限
8. **P1-9/10** `build_client` 坏代理显式报错；`RequestPlan` 拆分
9. **P1-12/13** `progress()` 增量化；补镜像/磁盘校验/拒绝路径测试
10. **P2 清理**：死 store/死函数/别名命令、快捷键注册、AuthDialog a11y（门禁回全绿）

---

## 八、勘误与方法论备注

第一稿曾出现 3 条 P0（"category-code 变量缺失"、"FileIcon 勾选框镜像重置"、"remove_download 绕过管理器"），回查源码后**均不成立且不出现在两路审查的最终报告中**——系摘要先行、正文未达时的误合成。本稿已删除，以两路报告正文 + 逐条回查为准。此前剔除的子审查误报亦记录在案：

- ~~"workspace 成员缺 src-tauri"~~（engine）：members 含 `src-tauri`，仅 exclude `probe-listener`。
- ~~"mutation-check 从未在 CI 运行"~~（shell 摘要误传）：`ci.yml` extension job 真实执行。
- "serde_json::Error 在 model.rs"（engine）：实为 `storage.rs::StorageError::Serde`，论断成立、坐标修正。
- "占位文件名在 finalize 落库"（engine）：机制描述偏差，失同步本质已按实际代码重写为 P0-2。

**方法论**：子审查对结构性判断（分层、模式、惯例）质量高，但 file:line 级行为断言存在幻觉率；本报告所有 P0/P1 均经主审查逐条回查源码（含 Node WS 实测），未逐一复核的 P2 保留为方向性建议，动手前请先定位代码。
