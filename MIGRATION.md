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

- [ ] M4.1：订阅调度、节点 URI 导入、provider 操作和持久本地规则覆盖。
  本轮迁出 13 类节点 URI 与别名解析；provider 更新/本地 YAML 覆盖已接通。
  自动订阅调度、专用规则覆盖及完整工作流验收仍未完成。
- [ ] M4.2：旧数据/备份导入、脚本迁移提示、mihomo 更新入口。
  本轮导入拒绝自定义 JS 并提示迁到 YAML，不执行或改写原文件。
  官方 stable/alpha 查询和候选暂存迁到 Rust CLI；不替换固定内核。
  旧数据/ZIP 导入、UI 候选激活及更新回滚仍未实现。
- [ ] M4.3：Mac 系统代理/DNS/TUN 与权限服务适配器；测试模式禁止 OS 写入。
  本机只做模拟适配器测试，禁止对日常网络进行真测。
- [ ] M4.4：流重连、可见性/后台订阅、日志轮转、取消及退出清理。
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
- [ ] M5.2：确定性测试覆盖路径/端口、配置回滚、取消/过期结果、快速
  切换、流断线、备份损坏、退出清理与重新启动。
- [ ] M5.3：真实测试入口。默认 VLESS/SS/Hysteria2/TUIC 每类最多两节点，
  最多四任务；全量显式开启。客户端明确指定测试代理，清除代理环境。
  覆盖单项/批量测速、进度/取消、HTTPS 转发与实际代理链；分别记录
  延迟、TLS、首包及请求成功，DIRECT/其他节点回退不算通过。
- [ ] M5.4：前后只读比较日常实例、系统代理/DNS/路由。只清理所属进程
  和 socket，不按名字杀进程，不恢复不属于测试的全局状态。
- [ ] M5.5：私有日志、匿名报告、Git 敏感产物检查与真实测试证据。
  外部不可达和日常 TUN 路径影响独立报告，不冒充产品验收。

同机测试隔离数据/进程/控制权，仍共享主机网络。系统代理/TUN 本轮
只用模拟适配器验证，不安装或接管日常 helper，不修改 mihomo 内核。

## 最终验收

- [ ] SwiftUI 视觉与交互、Swift/Rust 桥接、真实网络结果分别有证据。
- [x] 原生链路仅依赖 Swift、Rust、系统工具和 mihomo；无 Node/Tauri。
- [ ] 所有生成产物被忽略，配置凭据没有进入 Git，签名验证通过。
- [ ] 性能优化有可复现测量；不将低延迟等同于高吞吐。
