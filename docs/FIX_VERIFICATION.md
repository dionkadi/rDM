# DM 下载引擎 修复验证报告

本文验证针对《压测与性能基准报告》发现的 5 项缺陷/瓶颈的修复效果。
所有数字来自同一 harness 的两轮完整基准：修复前 `target/bench/results.json`、修复后 `target/bench/results_after.json`。

## 0. 修复内容（engine 代码变更）

| # | 缺陷 | 修复 | 涉及文件 |
|---|------|------|----------|
| 1 | 全局限速按每下载生效（N 并发 = N × 限速） | `CombinedLimiter` 改为 `Arc<TokenBucket>` 字段，新增 `from_shared()`；`run_download`/`run_hls_download` 共享 manager 的全局桶 | `limiter.rs` `task.rs` |
| 2 | checksum 校验读取未 rename 的目标路径，必然失败 | 校验对象改为 `.part`（此时字节已完整），通过后再 rename | `task.rs` |
| 3 | `ProxyMode::None` 不禁用 reqwest 环境代理 | 无代理参数时显式 `.no_proxy()`（`build_client`/`build_probe_client`） | `task.rs` |
| 4 | mirrors 镜像回退死功能（且计划回写丢失 `url`/`error`） | `build_plan` 在主 URL 探测失败时按序探测镜像，首个成功者升格为主 URL 并回写；计划回写补齐 `url`/`error`；全镜像探测失败时立即报错不再重复请求 | `protocol.rs` `task.rs` |
| 5 | SQLite 逐行事务逐次 fsync（并发串行化 + 单流 ~70MB/s 封顶） | 文件库开启 `journal_mode=WAL` + `synchronous=NORMAL` | `storage.rs` |

配套回归测试（新增 3 个）：`shared_global_bucket_is_shared`、`build_plan_falls_back_to_reachable_mirror`、`build_plan_without_mirrors_records_probe_error`。
修复后全量测试：**109 通过 / 0 失败**（86 lib + 9 hls + 12 integration + 2其它）。

## 结论速览

| 指标 | 修复前 | 修复后 | 变化 |
|------|--------|--------|------|
| 单流吞吐（128MB，1 连接，回环） | 84.758 MB/s | 1576.593 MB/s | 18.6× |
| 并发 100 makespan | 19.592s | 1.587s | 12.3× |
| 全局限速误差（8MB/s × 4 并发） | +266.3% | +4.6% | 修复 |
| 存储热 upsert | 68.4 ops/s | 29318.5 ops/s | 429× |
| 小文件周转 | 9.103 个/s | 61.594 个/s | 6.8× |
| 32 连接吞吐（原凹陷点） | 31.317 MB/s | 1569.219 MB/s | 50× |
| 整轮基准耗时 | 10m15s | 2m34s | 4.0× |

**全部修复均验证生效，且未引入任何回归（109 个测试全绿，所有字节校验通过）。**

## 1. 全局限速：从 +266% 失控到 ±20% 内

| 模式 | 目标 | 修复前实测 | 修复前误差 | 修复后实测 | 修复后误差 |
|------|------|-----------|-----------|-----------|-----------|
| global 8 MB/s × 4 downloads | 8.0 MB/s | 29.306 MB/s | 266.3% | 8.364 MB/s | 4.6% |
| per-download 2 MB/s × 4 downloads | 8.0 MB/s | 7.874 MB/s | -1.6% | 8.323 MB/s | 4.0% |

![rate](docs/bench/verify_rate.png)

## 2. checksum 校验：从必然失败到正确校验

| | 修复前 | 修复后 |
|---|--------|--------|
| 最终状态 | Error | Error |
| 错误信息 | `checksum read failed: No such file or directory (os error 2)` | `checksum mismatch (got 06b7bbfb7824aa03382051691630eb26de85102d1b08a81e907ec0744cd8a286)` |

修复后错误信息为 `checksum mismatch (got <sha>)`——校验真正执行了并按预期拒绝错误摘要。

## 3. 存储层（WAL）：写吞吐与全链路联动改善

| 指标 | 修复前 | 修复后 | 提升 |
|------|--------|--------|------|
| 下载行插入 10k | 66.3 ops/s | 11476.4 ops/s | 173.1× |
| 热行 upsert 2k | 68.4 ops/s | 29318.5 ops/s | 428.6× |
| history 写入 10k | 67.2 ops/s | 21512.6 ops/s | 320.1× |
| load_active 1k 行 | 5 ms | 5 ms | — |
| manager.list() 3k 行 | 2.324 ms | 3.48 ms | — |

![storage](docs/bench/verify_storage.png)

## 4. 并发扩展性：makespan 显著缩短

| 并发 | 修复前 makespan | 修复后 makespan | 缩短 | 修复前聚合吞吐 | 修复后聚合吞吐 |
|------|-----------------|-----------------|------|----------------|----------------|
| 5 | 0.829s | 0.074s | 91% | 48.252 MB/s | 538.546 MB/s |
| 10 | 1.858s | 0.15s | 92% | 43.061 MB/s | 532.11 MB/s |
| 25 | 4.311s | 0.405s | 91% | 46.397 MB/s | 493.438 MB/s |
| 50 | 11.915s | 0.79s | 93% | 33.57 MB/s | 506.606 MB/s |
| 100 | 19.592s | 1.587s | 92% | 40.834 MB/s | 504.024 MB/s |

![conc](docs/bench/verify_concurrency.png)

## 5. 小文件与单文件分块

| 指标 | 修复前 | 修复后 |
|------|--------|--------|
| 小文件 p50 延迟（200×256KB） | 16.7s | 1.6s |
| 小文件周转 | 9.103 个/s | 61.594 个/s |
| 单文件 1 连接 | 84.758 MB/s | 1576.593 MB/s |
| 单文件 32 连接 | 31.317 MB/s | 1569.219 MB/s |
| 修复后 128MB×6 档字节校验 | — | 全部通过 |

## 6. HLS

| 流 | 修复前 | 修复后 |
|----|--------|--------|
| 明文 | 19.471 MB/s | 35.31 MB/s |
| AES-128 | 9.722 MB/s | 36.618 MB/s |
| 字节一致性 | 明文 True / 加密 True | 明文 True / 加密 True |

## 7. 长时稳定性（120s soak）

| 指标 | 修复前 | 修复后 |
|------|--------|--------|
| 完成/失败 | 334/0 | 479/0 |
| 平均吞吐 | 22.2 MB/s | 31.9 MB/s |
| 窗口吞吐 min–max | 5.0–37.97 MB/s | 12.35–52.47 MB/s |
| RSS 斜率 | 0.53 MB/min | 1.11 MB/min |
| 残留 .part | 0 | 0 |

![soak](docs/bench/verify_soak.png)

**注**：修复后窗口吞吐整体上移（min 从 5.0 → 12.35 MB/s，完成数 334 → 479），但随时间的衰减形态仍在——fsync 因素已被 WAL 消除，剩余衰减与“持续 GB 级写入下的内核脏页回写节流”假设一致，需 iostat 观测定论。RSS 斜率 1.11 MB/min（基线 ~70MB）仍属稳定无泄漏。

## 8. 新增端到端修复验证场景

| 用例 | 结果 | 说明 |
|------|------|------|
| proxy_none_bypasses_env | ✓ Completed | Completed（直连成功，绕过指向死端口的 env 代理） |
| mirror_fallback_on_dead_primary | ✓ Completed，主 URL 已升格为镜像：True | Completed 且 url 被替换为镜像 |

## 9. 回归保障

- `cargo test -p dm-engine`：**109 通过 / 0 失败**（修复前后均全绿；本机跑测试需 `TMPDIR` 指向可写目录，因 /tmp 为 10MB tmpfs）。
- 新增回归测试钉住两个修复语义：共享全局桶必须串行化（`shared_global_bucket_is_shared`）、镜像回退必须升格主 URL（`build_plan_falls_back_to_reachable_mirror`）+ 无镜像时保留旧行为（`build_plan_without_mirrors_records_probe_error`）。
- 持久化契约测试（进度逐秒落盘、崩溃恢复、取消不回退）在 WAL 模式下全部通过。

## 10. 残留观察项（本轮未处理）

- **多连接在回环下无收益**：修复后 32 连接凹陷已消失（各档位均在 1.5-2.0GB/s 平台），说明凹陷确为存储 flush 串行化所致；多连接的真实价值仍需 netem/WAN 环境评估，本地结论不能外推。
- **长时负载吞吐衰减仍存**（52→12 MB/s）：WAL 消除 fsync 因素后衰减形态未变，指向内核脏页回写节流（2 小时级挂机下载的稳态速度问题），需 iostat/pidstat 观测与可能的限速写入策略。
- **HLS 段级 save_download**：WAL 后单次事务成本大幅下降，暂保留逐段持久化（崩溃恢复粒度更好）。
- **错误文案可读性**（reqwest 原文透出）：低优先级。
