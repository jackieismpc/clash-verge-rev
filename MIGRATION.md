# Mac 原生迁移清单

基线：`fe08de43`。仅 Apple Silicon、macOS 26+。复用本机 Swift/Rust，
保留 Go mihomo `v1.19.32` 和内核更新链路，不修改协议引擎。
顺序：维护清单 → 清理 → SwiftUI → 旧前端退场 → Rust 接入 → 隔离验收。
2026-10-13 范围更新：用户要求本轮彻底删除 TS/JS/Tauri，即使减少功能。
不再保留旧 Rust 参考壳等待功能齐全；未接通能力按 M4 重新开发，历史可回看。

## 开发规则

每次开发先读本文，按编号选择未完成项。实现和进度更新进入同一次
`git commit -s -S`，并运行 `git verify-commit HEAD`。只在满足完成标准后
勾选；部分完成、外部失败和未经验证的项目保持未勾选。证据记录命令、
结果与提交主题；同一提交的 SHA 可用 `git log -- MIGRATION.md` 查找。
README 维护使用方法，本文是唯一进度清单。

## 已有基线

- [x] B1：Mac arm64 构建收窄与兼容源码清理。提交 `57da658b`、`76829bd7`；
  Rust workspace 检查与旧壳回归通过，残留项继续按 M1 清理。
- [x] B2：Rust 下载、哈希验证与解压工具。提交 `76829bd7`；2 项工具测试。
- [x] B3：SwiftUI 六页、菜单栏与独立 Rust/mihomo 基础接入。
  提交 `6c2605ac`、`4cf86c49`、`1c3cbdc0`；基础桥接测试通过。
- [x] B4：容器、CI、Renovate、GEMINI/CLAUDE 和旧发布文档清理。
  提交 `fe08de43`；11 项 Rust、5 项 Swift 测试及旧壳检查通过。

## M0：迁移清单

- [x] M0.1：建立本文、AGENTS 开发约束及 README 入口。
  验证：本地链接与任务编号检查、`git diff --check`。
  提交主题：`docs(migration): establish tracked native migration checklist`。

## M1：清理无关文件和平台残留

依赖 M0。只按真实引用删除；窗口字段 `windows` 和媒体客户端标识不
等同于 Windows 兼容代码。保留 GPL、来源、有效构建与接口说明。

- [x] M1.1：清理 Windows 服务错误、过时平台注释、错误分支及对应契约。
  验证：374 项旧 Rust、44 项旧前端测试通过；生成契约与类型/引用检查。
  契约生成复用本机 Rust 和项目缓存，避免访问全局 Cargo 缓存。
  提交主题：`refactor(mac): remove unreachable Windows service failure paths`。
- [x] M1.2：移除旧应用 updater、启动检查、安装 IPC、前端入口、配置字段、
  专属依赖及 Windows 安装图标；保留 mihomo updater 与订阅更新通知。
  验证：374 项旧 Rust、44 项前端测试；Clippy、契约、类型、lint、Knip、
  架构与旧 Web 构建通过。有效托盘/原生图标保留至对应旧壳退场。
  提交主题：`refactor(app): retire upstream application updater`。

## M2：完成原生 UI

依赖 M1。延续 Liquid Glass 导航/控件、稳定卡片、原生表格与细线趋势。
Swift 拥有展示和编辑草稿；业务缺口用能力状态表达，不伪装为可用。

- [x] M2.1：规则源/节点源展示与更新、本地覆盖入口、连接详情。
  验证：8 项 Swift 测试；真实 mihomo 读取本地及 loopback HTTP 规则源、
  更新远程规则源后验证规则数及实际请求，禁止更新本地文件源。
- [x] M2.2：日志级别过滤/暂停、测速进度/取消、错误恢复。
  验证：旧批次不能贡献新进度的回归、Rust 测速请求 ID/取消测试、
  日志过滤及失败演示；9 项核心 Rust、2 项构建工具测试通过。
- [x] M2.3：系统代理/TUN 能力控件及匿名状态演示；禁用、处理中、失败
  和成功状态均明确，演示无 OS 副作用。
  验证：状态夹具测试、可运行 arm64 原生包；真实控件仍不可启用。
  M2.1–M2.3 提交主题：`feat(ui): complete native rule and activity workflows`。
- [x] M2.4：真实窗口视觉/交互验收。默认/最小窗口、明暗、长中文、
  大列表、键盘/VoiceOver、减少透明度/动态、关闭/重开全部覆盖。
  截图与记录留 `.work`；AppKit 内容位图不能证明真实玻璃效果。
  已检查真实概览明暗截图与匿名长列表；修复表格长名称撑宽和日志标签
  换行。2026-10-13 用户确认修订版及上述人工检查「已验收通过」，
  满足 M3 退场条件；不将截图采集工具失败视为 UI 失败。

## M3：旧前端退场

依赖 M2 验收。按最新授权同时删除旧 Rust/Tauri 业务、专属依赖和资源；
只保留原生实现及 mihomo 更新能力。不得以编译成功代替视觉验收。

- [x] M3.1：迁移必要行为为 Rust/Swift 回归用例，删除 React、TS/TSX/MTS、
  worker、Vite/Vitest、Web 性能工具及 Node/pnpm 配置/开发脚本。
  13 类 URI/别名解析移到 Rust 并接入原生导入；保留已有取消、进度、
  过期结果和状态排序回归。默认移除未知 VLESS transport，拒绝非法端口；
  修正普通 WS 误启用 HTTP upgrade、编码用户名和 TLS 参数被降级的问题。
- [x] M3.2：解除内建 JS 文件引用，使用已有 Rust 变换并删除 JS。
  自定义 JS 不执行；导入明确提示，原文件只读保留。
  两个兼容变换在原生 Rust 执行，保留嵌套/未知协议字段；无 Boa 解释器。
  验证：21 项原生 Rust 测试含脚本拒绝/源文件保留/URI 持久化。
- [x] M3.3：原生构建/运行/检查无 Node，仓库无第二套 UI。
  验证：限制 PATH 为系统工具后 `scripts/check-local.sh` 通过：21 项核心
  Rust、7 项工具、9 项 Swift 测试，含私有 loopback/Unix mihomo 工作流。
  移除旧 target 与 Swift scratch 后 `scripts/native-ui-build.sh` 干净构建
  arm64 原生包，`codesign --verify --deep --strict` 通过。
  新 Rust audit 检查 Git 输入、拒绝 TS/JS/Tauri、私有配置/日志/二进制；
  Cargo.lock 894 → 249 项，无保留依赖升级。Node 安装与旧编译缓存已删除。
  提交主题：`refactor(native): remove legacy frontend and Tauri stack`。

## M4：完善 Rust 接入和剩余迁移

依赖 M2；沿用版本化 C ABI/JSON。Rust 拥有配置事务、队列、取消和
生命周期，保留 generation/revision 顺序、资源归属及失败恢复。

- [x] M4.1：订阅调度、节点 URI 导入、provider 操作和持久本地规则覆盖。
  订阅单项后台更新，默认 1440 分钟；0 禁用，失败最多两次短重试。
  取消/删除/修改调度后旧任务不能提交，停止时等待所属任务退出。
  专用 prepend/append/delete 规则独立持久化，订阅刷新不覆盖。
  验证：loopback 自动更新工作流与规则保留、调度取消/过期票据、原生
  Swift → Rust → mihomo 规则覆盖/provider 更新/备份恢复工作流通过。
- [x] M4.2：旧数据/备份导入、脚本迁移提示、mihomo 更新入口。
  目录/ZIP 先复制私有原件再转换，保留选中配置与节点、旧自动更新禁用
  设置；拒绝路径逃逸、
  symlink、重复文件及超限展开。JS 与旧增强 YAML 保留并明确提示。
  stable/alpha 查询/候选暂存共用 Rust 库；UI 可明确启用私有候选或恢复
  固定基线。签名/哈希/架构/配置检查失败不切换，提交失败恢复旧内核。
  验证：旧 ZIP 实际命令导入、重复导入回滚、损坏备份、源文件保持、
  固定 mihomo 的候选激活/失败回滚/基线恢复/所属进程清理测试通过。
  本轮未执行官方远端更新，也未改变 Go 内核 v1.19.32。
- [x] M4.3：正式 Mac 系统代理/DNS/TUN 与独立权限服务适配器；测试模式
  在任何 OS/IPC 调用前禁止系统操作。SystemConfiguration 显式授权，
  0600 ownership journal 按字段 CAS 恢复，保留外部修改；关闭 PAC
  后启用手动代理。授权取消/checkpoint/恢复失败状态准确且可重试。
  TUN 采用独立 SMAppService root helper，固定 root 所有的签名安装包、
  peer audit token/code hash/UID 与 owner/generation 校验，私有 controller
  防火墙禁止任意特权配置路径；使用固定内核，不使用可写候选。
  验证：mock/纯编码权限、PAC、DNS、CAS、checkpoint、helper 生命周期、
  目录与协议校验通过；正式 helper 编译、打包及签名验证通过。
  提供用户执行的安装脚本，工具未安装/注册服务、授权或改变系统网络。
- [ ] M4.3V：系统代理/DNS/TUN、helper 注册/批准/启停、登录启动的真实
  macOS 系统验收。2026-10-13 用户确认没有 VM，暂时跳过，由用户后续
  自行检测。M4.3 勾选只表示实现及确定性验证完成，不表示 OS 真测通过。
- [x] M4.4：流重连、可见性/后台订阅、日志轮转、取消及退出清理。
  WebSocket 连接/握手/idle 超时与有限退避重连，重新连通清除流错误；
  隐藏窗口停止指标/连接订阅与投影刷新，必要日志/后台订阅继续。
  普通/特权内核日志最多 4 × 2 MiB，I/O 失败仍排空 pipe；取消和退出
  等待所属任务/进程；TUN 启动任意中途错误尝试清理所属 session，失败
  保留身份供恢复。成功 reload 清理无引用旧 staging，保留外部目录。
  验证：Unix WebSocket 断线重连、日志上限、取消/退出、重新启动和
  Swift 隐藏/显示真实桥接工作流通过。
  移除无消费者的 Rust staticlib 产物类型，只构建 Swift 使用的 cdylib
  和 helper/Rust 使用的 rlib；旧静态档及 incremental 缓存仅在项目内清理。
- [x] M4.5：按本轮新授权直接删除 Tauri、旧业务壳和专属依赖。
  保留已接通的原生功能；其余 M4 项不阻塞退场，明确记录待重做。
  验证：原生业务库独立构建、现有工作流回归、源码和依赖扫描通过。
  整体删除 src-tauri、Tauri 插件和旧壳专属 crate/资源/配置；mihomo
  下载清单迁到 resources，更新查询/候选暂存保留为纯 Rust CLI。
  未运行远端更新，也未替换 v1.19.32 基线；更新候选无供应方摘要承诺。
  M3 与 M4.5 在本轮同一签名提交完成。

## M5：隔离集成验收

默认检查只使用无凭据样本。真实机场测试必须单独显式启动。
只读源通过本地参数传入，不将内容写入受 Git 跟踪文件。
用户授权源：本机 Clash Verge profiles 下的 `Rh6ktlC2sPvF.yaml`。

- [x] M5.1：初始化支持 `executionMode: interactive | integrationTest`。
  Rust 强制隔离：独立 `.work/integration/<编号>`，0700 目录、0600 文件、
  Unix socket、随机 secret、loopback 端口；关闭 TUN、DNS 监听、NTP、
  LAN、额外监听和外部 UI；provider 路径重写，不允许越界。
  禁止系统代理/DNS/路由、helper 和登录启动操作，覆写不能绕过。
  额外参数 `integrationRoot` 必须指向真实 `.work/integration`，运行目录
  必须在其下。提前落实隔离前提，为后续真实测试提供边界；不进行性能
  重构。Go 子进程清空继承环境并设置私有文件掩码，父进程环境不变。
  验证：13 项核心 Rust、2 项工具、8 项 Swift 测试；覆盖路径逃逸/
  symlink、覆写再启用网络、禁止命令、UDP 端口冲突和 0600/0700 权限。
  Swift 真实 mihomo 桥接已使用该模式；机场测试仍未运行。
  提交主题：`feat(test): enforce private native integration runtime policy`。
- [x] M5.2：确定性测试覆盖路径/端口、配置回滚、取消/过期结果、快速
  变更、流断线、备份损坏、退出清理、重新启动与内核切换回滚。
  验证：`scripts/check-local.sh`（源码 audit、fmt、Clippy、Rust 与 Swift），
  `NATIVE_BUILD_PROFILE=release scripts/native-ui-test.sh` 验证正式优化桥接。
  最终完整检查：62 项核心 Rust、3 项真实测试入口、4 项构建工具、
  4 项共享更新库和 9 项 Swift 测试通过，fmt/Clippy/audit 全通过。
- [x] M5.3：真实测试入口 `scripts/native-integration.sh --profile /absolute/...`。
  默认 VLESS/SS/Hysteria2/TUIC 每类最多两个；同时测速最多四个，无人为
  等待，全量需 `--all-nodes`。显式代理、清除代理环境和 curl 配置发现，
  正常 TLS，核对目标节点/测试策略组，拒绝 DIRECT/其他节点回退。
  2026-10-14 实际抽测 8 节点；4/8 HTTPS、TLS 与目标链通过：VLESS 1/2、
  Hysteria2 1/2、TUIC 2/2；SS 0/2。其余 4 项 curl TLS 建连失败（exit 35），
  不自动归因于客户端或机场。成功项批测延迟 464–1149 ms，首包
  161–689 ms；只作本次样本记录，不声称协议/吞吐提升。
- [x] M5.4：真实测试前后只读比较日常实例、系统代理/DNS/全部路由与
  默认路由，均一致。只停止所属 PID；退出后 PID/socket 消失，TCP/UDP
  监听释放，未按名称杀进程或恢复全局设置。原始机场文件字节不变。
- [x] M5.5：真实报告 `.work/integration/i-ffc146c1/summary.json`；原始
  事件/curl/网络快照仅私有目录，权限和 Git 忽略检查通过；摘要用匿名
  节点编号，未提交 YAML/凭据/日志。9 条单项/批测进度完整，取消、旧
  generation 拦截均通过。整体测试退出 1，明确表示 4 项网络失败。
- [ ] M5.6：复查失败网络样本，尤其 Shadowsocks 的实际转发证据；本轮
  完成隔离边界和尝试记录，未将未通过协议记为全量产品验收通过。

同机测试隔离数据/进程/控制权，仍共享主机网络。系统代理/TUN 本轮
只用模拟适配器验证，不安装或接管日常 helper，不修改 mihomo 内核。

## 本轮提交与最终验收

2026-10-14 提交主题：`feat(native): complete real runtime and isolated network validation`。
实现、使用说明及上述进度进入同一次签名提交。完整检查与正式打包日志
位于 `.work/m4-check.log`、`.work/m4-release-bridge.log`、
`.work/m4-product-build.log`；真实测试日志 `.work/m5-airport-run.log`。

- [x] SwiftUI 已有用户视觉验收；真实 Swift/Rust/mihomo 桥接通过，真实
  网络成功/失败有独立记录，默认应用不使用演示数据。
- [x] 原生链路仅依赖 Swift、Rust、系统工具和 mihomo；无 Node/Tauri。
- [x] 生成产物被忽略，未暂存配置凭据；源码 audit 拒绝私有 JSON/YAML、
  日志及无后缀 Mach-O/ELF。正式 bundle `codesign --verify --deep --strict`
  通过，Git 提交使用签名并 `git verify-commit HEAD`。
- [ ] M4.3V 与 M5.6 的真实系统/失败网络验收完成。用户后续系统检测与
  外部样本复查仍需进行；模拟适配器不代替这些结论。
- [ ] 性能优化有整产品 GUI/helper/mihomo 可复现内存/CPU/吞吐测量。
  本轮只记录节点延迟/TLS/首包与有界缓存行为，不将低延迟等同高吞吐。

## M6：生产稳定性与 FFI 收紧

2026-10-14 用户要求继续稳定化与冗余清理，FFI 优先；Go 内核固定，
系统网络真测仍按 M4.3V 的用户约定跳过。

- [x] M6.1：ABI 2 使用显式 UTF-8 字节长度、受控句柄与缓冲所有权；
  Swift 直接导入唯一 C 头文件，删除重复手写 ABI 定义和 NUL 扫描。
  完成标准：非法/过期句柄、重复释放、输入边界、版本不匹配、并发调用、
  创建/销毁/重新打开及真实 Swift 桥接通过；JSON schema 仍为 1。
- [x] M6.2：限制命令积压字节数，确保停止与故障能结束所有接受的请求；
  Swift 请求有明确等待上限、取消及退出不无限轮询。
  完成标准：任务异常、积压、停止并发及恢复回归通过。
- [x] M6.3：限制 controller HTTP/WebSocket 数据与后台事件缓存；
  清理不再被使用的兼容入口与重复逻辑，依据真实引用删除。
  完成标准：超限/断流/恢复测试和原有功能工作流通过。

- [x] M6.4：系统代理/DNS 迁到独立 helper 的持久租约；监护 GUI 与核心
  进程身份，异常结束自动恢复，服务重启先回收旧 journal/所属引擎。
  TUN 先以无路由模式启动、写入进程恢复记录后才启用；使用 redir-host
  避免退出后系统缓存残留 Fake IP。关闭代理/TUN 不重新应用本应用 DNS。
  完成标准：DNS/DHCP/外部编辑/部分 apply 失败重试、租约失效、PID 复用
  与恢复顺序的确定性测试通过；真实 OS 验收继续按 M4.3V 留给用户。
- [x] M6.5：安装/卸载顺序与无残留检查。卸载先注销服务和登录启动、恢复
  所属配置、确认所属引擎退出，再删包、helper 数据、运行目录、偏好。
  恢复失败停止删除，源仓库/全局工具链/日常 Clash Verge 均不触碰。
  完成标准：脚本语法、路径/顺序、模拟卸载失败场景通过；本机真实安装
  卸载由用户执行，不用模拟测试冒充正式系统验收。
- [x] M6.6：本地规则放在 MATCH 前，明确替换 fallback，避免无效后置规则；
  删除匹配规范化、去重、订阅刷新/备份/重启保留，错误事务不改有效规则。
  完成标准：最终 mihomo 规则顺序和真实请求命中验证通过。

M6 验证证据（2026-10-14）：

- `scripts/check-local.sh`：源码 audit、fmt、Clippy `-D warnings`、Rust 及
  Swift 工作流通过：76 核心 + 3 隔离 runner + 4 构建工具 + 4 更新库的
  Rust 测试、11 项 Swift 测试；日志 `.work/m6-check-final.log`。
- `NATIVE_BUILD_PROFILE=release scripts/native-ui-test.sh`：11 项优化构建
  Swift/真实桥接测试通过；`.work/m6-release-bridge.log`。
- `scripts/native-ui-build.sh`：正式 arm64 包和 `codesign --verify --deep
  --strict` 通过；`.work/m6-product-build.log`。产物与测试数据均 Git 忽略。
- `scripts/installation-test.sh`：拒绝 symlink，注销失败/恢复失败不删除，
  成功恢复先于范围限定删除，外部哨兵文件保留。未调用真实 sudo/SM 注销。
- 实际向自有测试父进程发送 SIGKILL，所属普通 mihomo、socket、TCP 监听
  随监护退出；未按进程名称杀进程。特权租约的进程丢失/PID 复用/DHCP恢复
  使用模拟后端；helper 重启和真实 TUN 路由回收仍需 M4.3V 人工验证。
- 真正的 Swift → ABI 2 → 固定 mihomo：本地 HTTP 目标 DIRECT 成功，追加
  REJECT 后目标未收到请求；非法 MATCH 事务不改有效规则，备份恢复及
  内核重启后仍拦截；ABI 1 在创建前被拒绝。
- C 头文件经 clang 实际编译验证大小/偏移；FFI 并发销毁、重复释放、伪造
  缓冲、非法 UTF-8、超长输入及输出池满后接受结果可重读通过。命令积压
  按字节拒绝，worker 故障结束已接受 ID，重复完成不会重复发布。
- Unix controller 超限 chunked HTTP 与 WebSocket 后下一请求/重连恢复；
  指标合并和日志字节上限测试通过。Swift 请求有 deadline，退出等待有上限；
  未确认 Rust 已完成时保留 dylib 映射，避免后台线程执行已卸载代码。
- 旧 GUI 网络 journal 不静默忽略；helper 从签名 bundle 的默认数据路径
  导入 root journal 后恢复。中断的 pending checkpoint 可恢复，symlink
  staging 被拒绝；部分 commit/apply 失败的持久恢复和外部 DNS 编辑保留。
- 安装/卸载说明同步 README；仅关闭窗口仍保持后台代理，退出或关闭代理/
  TUN 才恢复对应网络。原生运行目录、缓存与偏好属于卸载范围，开发源码与
  编译缓存保留；自定义开发目录需自行处理。

正式构建/验收前后只读快照 `.work/m6-network-check/`：系统代理与 DNS
字节一致；路由的目标/网关/接口集合一致，但 en0 邻居缓存标志和到期计时
有变化，未将原始路由快照记为完全一致，也未执行全局恢复。

M6 完成仅指实现与上述自动回归，未更改 M4.3V 的真实 OS 验收状态。
未安装/注销服务，未改系统代理、DNS、TUN/路由，未刷新全局 DNS 缓存。
本轮提交主题：`fix(native): harden owned network recovery and ABI lifecycle`。

## M7：权限引导与真实工作流修复

2026-10-14 用户实测：开发应用启动内核后开启系统代理报 helper 未注册；
继续保留独立特权监护和固定安装身份，不绕过异常退出恢复保证。

- [x] M7.1：区分 helper 未连接与命令已提交后不确定失败；未接管网络时
  缺少服务不产生假恢复状态，不阻止普通内核停止/重启。明确错误代码和原因。
  验证：未发送/已发送/已有租约失败回归、停止与重启工作流。
- [x] M7.2：原生权限设置共用实时状态，系统网络操作提供安装/注册/批准
  引导和可复制命令；未准备好时不发送接管命令。取消/恢复不依赖启用前置条件。
  验证：开发目录、未注册、等待批准、已批准和隔离模式的行为回归。
- [x] M7.3：通用导入框识别节点 URI/订阅 URL/YAML，由 Rust 分类并解析；
  刷新重新读取权威快照，修复恢复后残留错误和并发结果的状态覆盖。
  验证：实际导入框 payload、无事件快照变化、故障恢复、原有规则/备份等。
- [x] M7.4：复查核心工作流、安装脚本与启动诊断，提供一条命令构建并打开
  开发应用；保护正在运行的原生包。全部检查、正式构建、签名提交。
  真实系统代理/DNS/TUN 与服务批准仍由用户执行，不把引导实现记为 OS 真测。


M7 验证证据（2026-10-14）：

- `scripts/check-local.sh`：源码 audit、fmt、Clippy `-D warnings`、89 项
  Rust（78 核心 + 3 runner + 4 构建工具 + 4 更新库）、17 项 Swift 通过；
  `.work/m7-check-final.log`。`NATIVE_BUILD_PROFILE=release scripts/native-ui-test.sh`
  同样 17 项通过；`.work/m7-release-bridge.log`。
- `scripts/native-ui-build.sh`：正式 arm64 包与严格递归签名检查通过；
  `.work/m7-product-build.log`。没有启动真实 GUI 或注册服务，产物 Git 忽略。

- helper 的连接/身份/发送准备失败使用类型化错误；已有租约或已发送后失败
  保留恢复要求。未连接情况下开关保持 off，空恢复无需 IPC。异常/损坏的
  helper 状态不静默当作成功，停机成功清除旧错误，失败保留所属 PID。
- 开发目录/未注册/等待批准均在发送前进入权限引导；批准状态读取持续刷新。
  恢复关闭不受启用前置条件阻挡；设置与主窗口避免同时弹出重复权限 sheet。
  提供只读「检查服务连接」，注册成功不等于自动开启系统代理。
- 实际导入框 `source` payload 经 Swift → Rust → mihomo 验证节点 URI。
  规则模式显示正确的策略组选中节点，普通刷新保持选择，不被 GLOBAL 覆盖。
  未选择配置先引导导入，运行中删除最后一个配置被拒绝且原有运行继续。
- 测试专用 C 转发层丢弃所有 state 事件，其他 ABI 调用转发到真实 Rust：
  导入、启动、模式修改、停止仍通过权威快照正确更新，验证了丢事件场景。
  该夹具仅在测试中编译，不进入正式包；没有新增生产 ABI 或桥接框架。
- live instance 防护用自有 sleep 副本验证，拒绝构建/安装覆盖；只停止所属
  测试 PID。`scripts/native-ui-run.sh` 一条命令构建 debug 并打开界面，
  不安装、不注册服务、不自动接管网络，不允许 sudo 启动开发应用。
- 两轮真实机场隔离复测，各取同一 8 个节点，分别访问 Google 与 Apple。
  两轮均 2/8 通过 HTTPS/TLS/目标链：VLESS 1/2、TUIC 1/2、SS 0/2、
  Hysteria2 0/2。SS 在两个目标均未通过，不能简单归为单个测试 URL 问题，
  也不足以判定协议内核或凭据的原因。M5.6 保持未完成。
  报告 `.work/integration/i-bb7f692b/summary.json`、
  `.work/integration/i-3d94014c/summary.json`；两轮退出 1，明确网络失败。
  取消、进度、旧结果拦截、源文件不变及所属 PID/socket/TCP/UDP 清理通过；
  日常进程、系统代理、DNS、全部/默认路由快照均一致，原始日志不提交。

本轮提交主题：`fix(native): guide helper setup and repair runtime workflows`。
当前服务尚未安装/批准，代码修复不能代替管理员与系统设置授权；真实
系统代理/DNS/TUN 仍按 M4.3V 由用户执行验收。本轮没有注册/安装服务或
修改全局网络，不将权限引导、模拟测试或显式代理测试算作系统代理通过。

## M8：失败诊断与并发刷新稳定性

2026-10-14 继续 M5.6 失败样本复查，保持固定 Go 内核和已有系统真测边界。

- [x] M8.1：匿名报告区分显式代理建连、TLS、HTTP、目标链验证阶段，保留
  curl 退出码和有效时间；原始错误仅写私有目录，不输出机场地址或凭据。
  完成标准：失败分类、空/损坏指标、证书失败和目标链拒绝的确定性回归通过。
- [x] M8.2：节点详情保留测速失败原因；后台刷新遇到配置并发变化时丢弃
  过期投影，不弹错误、不掩盖真实故障。结果/状态恢复后旧错误不残留。
  完成标准：节点结果显示与并发刷新/变更回归通过。
- [x] M8.3：用完全人工构造的 loopback Shadowsocks 2022 服务验证真实
  Swift/Rust/mihomo 转发及错误密钥拒绝，所有进程/端口/文件只属于测试。
  完成标准：正确密钥转发到自有目标、错误密钥不抵达、关闭后监听释放。
  本地成功不能代替机场节点或系统代理/TUN 验收；M5.6 仍按外部结果记录。

M8 验证证据（2026-10-14，基线 `7b6f1c35`）：

- `scripts/check-local.sh`：源码 audit、fmt、Clippy `-D warnings`、90 项
  Rust（78 核心 + 4 runner + 4 构建工具 + 4 更新库）、18 项 Swift 通过；
  `.work/m8-check.log`。`NATIVE_BUILD_PROFILE=release scripts/native-ui-test.sh`
  同样 18 项通过；`.work/m8-release-bridge.log`。夹具启动失败兜底清理
  补强后再次运行 release 的 SS 专项通过；`.work/m8-release-cleanup.log`。
- `scripts/native-ui-build.sh`：正式 arm64 包与严格递归签名检查通过；
  `.work/m8-product-build.log`。没有启动真实 GUI、注册 helper 或修改系统网络。
- 版本 2 匿名报告只取 curl 数值与固定分类代码，原始 URL/错误不进入
  摘要。覆盖 TLS 握手/证书拒绝、阶段超时、HTTP 失败、缺少目标链、非法
  指标和进程中断；已完成的 TLS 验证不会因后续 HTTP 失败变成失败。
  这些是客户端观察阶段，不把 TLS 握手前断流直接判为证书或协议引擎问题。
- Swift → Rust → 固定 mihomo → 自有单密钥 SS 2022 loopback 服务 → 自有
  HTTP 目标：正确密钥转发成功，错误密钥不抵达目标；修改配置后测速成功，
  错误详情清除，普通刷新保留失败详情。失败后仍允许手动选择；关闭后两个
  所属 controller socket 消失，监听可重用。所有密钥都是人工样本。
  夹具使用[官方入站配置](https://wiki.metacubex.one/config/inbound/listeners/ss/)，
  未涵盖机场多用户服务及外部 HTTPS，也不将本地成功记为 M5.6 通过。
- 连续八轮真实内核模式修改与投影刷新并发通过。临时移除过期投影处理后
  同一回归明确抛出 `stale_result`；恢复实现后通过，负向对照日志
  `.work/m8-refresh-negative-control.log`，修复日志 `.work/m8-ss-fixed.log`。
  仅后台刷新丢弃正常的过期结果，用户测速及其他真实错误仍返回失败。
- 第三轮机场隔离复测仍 2/8 HTTPS/TLS/目标链成功：VLESS 1/2、TUIC 1/2、
  SS 0/2、Hysteria2 0/2，退出 1。六项失败均 curl 35、TLS 未完成；其中
  一个成功转发节点测速失败，不能把测速结果当作所有网络请求的可用性。
  所选节点的原始字段与源配置完整一致；此证据不能验证远端认证或线路。
  报告 `.work/integration/i-185c805d/summary.json`，日志 `.work/m8-airport.log`。
  取消、进度、过期代次拦截、源文件不变、私有权限和所属 PID/socket/TCP/UDP
  清理通过；日常进程、系统代理、DNS、全部/默认路由快照均字节一致。

本轮提交主题：`fix(native): retain probe diagnostics and tolerate superseded refresh`。
M5.6 外部失败样本与 M4.3V 真实系统验收保持未完成；固定 mihomo 未修改，
不安装/注册服务、不接管或恢复日常 Clash Verge 的网络配置。
