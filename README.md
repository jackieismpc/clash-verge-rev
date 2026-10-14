# Clash Native

Apple Silicon、macOS 26+ 的 SwiftUI 原生代理客户端。独立维护自
[Clash Verge Rev](https://github.com/clash-verge-rev/clash-verge-rev)，保留
Git 历史、来源说明和 GPL-3.0 许可证。Swift 负责界面，Rust 负责业务、
配置事务与运行管理，Go mihomo 固定基线为 `v1.19.32`。

迁移进度统一记录在 [MIGRATION.md](MIGRATION.md)。每次开发先阅读清单，
完成任务后在同一次签名提交更新验证证据。仓库已删除 TS/JS、React、Node
及 Tauri；只维护原生应用。

## 构建与使用

复用本机已安装的 Rust、Swift 和系统工具，不安装重复工具链或修改全局
设置。所有构建、缓存、数据和报告放在忽略的 `.work/`。固定内核下载与
哈希验证使用 Rust，清单位于 `resources/core-artifacts.json`。

```sh
scripts/native-core-assets.sh
scripts/native-ui-build.sh
open '.work/native-ui/Clash Native.app'
# 后续开发一条命令构建 debug 包并打开（先退出旧原生实例）：
scripts/native-ui-run.sh
scripts/check-local.sh
```

构建默认使用 release 优化和本地 ad-hoc 签名；`NATIVE_BUILD_PROFILE=debug`
可选择调试构建。Rust 脚本使用 `rust-toolchain.toml` 指定的已安装工具链；
其他兼容版本通过 `NATIVE_RUST_BIN_DIR` 指定 bin 目录。首次 Rust 依赖下载
执行 `scripts/native-core-build.sh build`，缓存仍在 `.work/cargo`；后续离线构建。
不提供分发签名或公证。详细界面与桥接说明见 [native/README.md](native/README.md)。

构建脚本拒绝覆盖正在运行的原生应用，需用 ⌘Q 退出后重新构建；不会停止
日常 Clash Verge。开发启动只打开界面，不自动接管系统网络。

默认是真实模式；`--demo` 仅供明确标记的视觉样本。先在「订阅」导入
YAML、节点链接或订阅地址，选择配置，启动内核并选择节点。可显式使用
`127.0.0.1:17890`，点击系统代理或 TUN 开关才会改变系统网络。

导入框支持直接粘贴单条节点链接、订阅 URL 或 YAML，分类和解析由 Rust
完成。无启用配置时启动入口先提示导入；运行期间删除最后一个配置被拒绝，
请先停止内核，避免无意退回 DIRECT。

本地追加规则插入最终 MATCH 前；显式追加 MATCH 可替换原 fallback。非法规则
不替换当前配置，覆盖会保留到订阅更新、重启和备份恢复后。

支持订阅后台更新、节点选择与测速取消、规则源更新和持久本地规则覆盖、
连接详情与关闭、日志过滤与暂停、解锁检测、原生备份及旧目录/ZIP 导入。
自定义 JS 只保留原件并提示迁移，不执行。关闭窗口保留后台任务，退出等待
所属任务与内核清理、网络恢复。UI 尚待用户后续验证真实网络权限流程。

## 系统网络与安装卸载

系统代理、系统 DNS 和 TUN 都由本项目的独立 SMAppService helper 管理，
不复用日常 Clash Verge 服务。安装路径固定为 `/Applications/Clash Native.app`。
先退出 Clash Native；更新已有安装前，在旧应用的设置 → 网络注销服务。

```sh
scripts/native-ui-build.sh
sudo scripts/install-local.sh
open '/Applications/Clash Native.app'
```

在设置 → 网络注册权限服务，并到系统的登录项与扩展批准。安装本身只复制
签名应用，不接管网络。权限服务未批准时，仍可使用独立 loopback 显式代理，
主窗口会显示「需安装/注册服务」或「等待批准」，点击进入权限准备页，
可复制安装命令；系统设置批准后状态自动刷新。权限未准备好时不提交网络
接管命令，普通内核仍可独立停止/重启。

网络接管前，helper 持久保存原始值和进程身份。正常退出先恢复仍属本应用
的代理/DNS，再等待所属核心退出。GUI 崩溃或被强制结束时，独立 helper
监护租约并恢复；服务重启先恢复旧 journal 和回收所属特权引擎。其他程序
后来修改的字段予以保留。恢复失败保留 journal 并重试，不删除恢复证据。
普通内核另有生命周期管道监护，GUI 消失后停止并等待 Go 子进程。

关闭系统代理或 TUN 会恢复所属系统 DNS；TUN 使用 `redir-host`，避免退出
后系统 DNS 缓存仍保留本应用生成的 Fake IP。手动系统 DNS 只接受公共/LAN
的单播 IP，不允许指向未提供 DNS 监听的 localhost。不会停止系统 DNS 服务
或清空全局 DNS 缓存。关闭主窗口仍在后台运行；⌘Q/菜单栏退出才结束接管。

卸载请使用下列命令，不要直接把应用拖到废纸篓：

```sh
# 先退出 Clash Native；以普通用户从构建此安装的仓库执行
scripts/uninstall-local.sh
```

卸载依次注销 helper 和登录启动、恢复网络及确认所属引擎退出，再删除安装包、
`/var/db/io.github.jackieismpc.clash-native.helper`、服务 socket、项目原生运行
目录/私有集成数据、安装备份、本应用的缓存/偏好/保存窗口状态及默认
Application Support 数据。任一步恢复失败都停止文件删除，可排除错误后重试。
源仓库、开发编译缓存、全局 Rust/Swift 和日常 Clash Verge 数据均保留。
脚本拒绝重定向的数据目录和仍在运行的本应用；自定义 `CVN_DATA_DIRECTORY`
仅供开发，指定的额外目录需开发者自行清理，正式使用请保留默认目录。

本轮没有 macOS VM，按用户确认暂跳过安装、注销及系统代理/DNS/TUN 的真实
系统测试。上述实现已有模拟事务与进程测试；它们不能证明真实 OS 恢复已验收。

## mihomo 更新

设置 → 内核更新可检查 stable/alpha、下载并验证私有候选、明确启用及
恢复固定基线。不会自动替换内核。TUN 使用安装包内受签名保护的固定内核，
启用 TUN 前先恢复固定基线。CLI 同样保留更新链路：

```sh
scripts/native-core-assets.sh --check-core-update
scripts/native-core-assets.sh --stage-core-update
# 如需预发布，在上述命令末尾追加 --alpha
```

CLI 候选在 `.work/core-updates`，UI 候选在应用私有数据目录；不能跨目录
直接启用。报告 SHA256 是本次文件摘要，不代表供应方签名。应用自身的
上游更新已移除；尚未修改或自研协议引擎。

## 隔离测试

默认 `scripts/check-local.sh` 只用无凭据样本。真实测试需显式传入私有
配置绝对路径：每类最多两个 VLESS/SS/Hysteria2/TUIC，最多四任务；
`--all-nodes` 才开启全量。只读源文件，使用自己的 loopback 代理及 Unix
controller，不修改系统代理、DNS、TUN 或路由。

```sh
scripts/native-integration.sh --profile '/absolute/path/to/private.yaml'
# 全量测试：追加 --all-nodes
```

配置、原始日志和匿名报告只留在 0700 的 `.work/integration/`，文件 0600。
记录测速、TLS、首包、HTTPS 和实际代理链；DIRECT/其他节点回退不算通过。
同机测试仍共享主机网络，保留日常 TUN 对出口的影响，不自动恢复全局设置。
外部失败不冒充通过，也不直接归因于客户端。系统网络真测由用户后续执行。

## 维护

仅本机开发编译，不维护容器、交叉编译、GitHub CI 或上游发布流水线。
直接在 `main` 验证并签名提交；签名失败时停止提交：

```sh
git commit -s -S -m "fix(component): describe the change"
git verify-commit HEAD
```

源码 audit 拒绝 TS/JS/Tauri 回流及配置凭据、日志、下载二进制进入 Git。
AGENTS.md 只记录实际开发约束；删除的旧功能可从历史对照后重新开发。
