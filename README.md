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
scripts/check-local.sh
```

构建默认使用 release 优化和本地 ad-hoc 签名；`NATIVE_BUILD_PROFILE=debug`
可选择调试构建。Rust 脚本使用 `rust-toolchain.toml` 指定的已安装工具链；
其他兼容版本通过 `NATIVE_RUST_BIN_DIR` 指定 bin 目录。首次 Rust 依赖下载
执行 `scripts/native-core-build.sh build`，缓存仍在 `.work/cargo`；后续离线构建。
不提供分发签名或公证。详细界面与桥接说明见 [native/README.md](native/README.md)。

默认是真实模式；`--demo` 仅供明确标记的视觉样本。先在「订阅」导入
YAML、节点链接或订阅地址，选择配置，启动内核并选择节点。可显式使用
`127.0.0.1:17890`，点击系统代理或 TUN 开关才会改变系统网络。

支持订阅后台更新、节点选择与测速取消、规则源更新和持久本地规则覆盖、
连接详情与关闭、日志过滤与暂停、解锁检测、原生备份及旧目录/ZIP 导入。
自定义 JS 只保留原件并提示迁移，不执行。关闭窗口保留后台任务，退出等待
所属任务与内核清理、网络恢复。UI 尚待用户后续验证真实网络权限流程。

## 系统代理与 TUN

系统代理/DNS 使用 macOS 授权提示和所属字段恢复日志。TUN 使用本项目
独立 SMAppService helper，不复用日常 Clash Verge 服务。安装路径固定为
`/Applications/Clash Native.app`；请退出旧的 Clash Native 后自行安装：

```sh
scripts/native-ui-build.sh
sudo scripts/install-local.sh
open '/Applications/Clash Native.app'
```

在设置 → 网络注册特权服务，按系统提示到登录项与扩展批准，再开启 TUN。
安装脚本只复制应用，不注册服务或改变网络。原生数据仍在项目 `.work`；
helper 的 root 数据位于 `/var/db/io.github.jackieismpc.clash-native.helper`，
服务 socket 位于 `/var/run/`，与日常实例分离。更新安装后要重开应用与服务，
避免旧进程和新签名不一致。本轮没有 macOS VM，按用户确认暂跳过系统
代理/DNS/TUN 的真实系统测试，后续由用户自行检查。

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
