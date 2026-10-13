# Clash Native

面向 Apple Silicon、macOS 26+ 的原生代理客户端改造。

这是从 [Clash Verge Rev](https://github.com/clash-verge-rev/clash-verge-rev)
独立维护的 GPL-3.0 项目，保留原始 Git 历史与许可证。原始版权和
第三方组件许可证继续适用。代理协议引擎目前仍为 Go 编写的 mihomo；
Rust 负责业务和运行管理，SwiftUI 负责原生界面。

## 当前实现

迁移任务和验收进度统一记录在 [MIGRATION.md](MIGRATION.md)。每次开发先
阅读清单，完成任务后在同一次签名提交中更新状态与验证证据。

- 产品构建目标已收窄为 `aarch64-apple-darwin`、macOS 26+；Windows/Linux
  源码分支、直接依赖、安装资源和构建矩阵已清理。
- 原生 SwiftUI 视觉样板：六页导航、Liquid Glass 控件、菜单栏、概览、
  节点表、订阅管理和错误详情；已接通 Rust 与真实 mihomo。
  `--demo` 可切换到明确标记的视觉样本。
- 原生构建只使用本机 Rust、Swift 和系统打包工具；内核下载、SHA256
  校验与 gzip 解压由 Rust 完成，不依赖 Node/pnpm。
- 独立 Rust 业务库：配置事务、订阅获取、独立内核生命周期、Unix
  controller、状态流、解锁检测和本地备份；不依赖 Tauri。
- React/Tauri 保留为迁移期间的功能对照。原生系统代理、DNS/TUN、
  旧数据导入及完整产品验收仍在实施中，尚不作为日常代理替代品。

## 本地开发

复用本机已安装的 Rust 和 Swift 工具链。脚本不会安装工具链或修改全局
默认工具链。构建、缓存、临时文件、测试数据和本地应用放在 Git 忽略的
`.work/`。Tauri 的 `src-tauri/resources`、`src-tauri/sidecar` 也是忽略的
生成目录。不要提交下载的二进制或运行数据。

```sh
scripts/native-core-assets.sh
scripts/native-core-build.sh build
scripts/native-ui-build.sh
open '.work/native-ui/Clash Native.app'
scripts/check-local.sh
```

Rust 脚本优先使用 `rust-toolchain.toml` 指定的**已安装**工具链。若本机
使用其他兼容版本，可设置 `NATIVE_RUST_BIN_DIR` 指向其 `bin` 目录。
首次依赖下载需去掉 `--offline`，下载缓存仍保留在 `.work/cargo`。
`check-local.sh` 使用已有缓存，检查原生 Rust 格式、Clippy、单元测试及
Swift → Rust → mihomo 集成，不安装工具或调用 Node。详细界面说明见
[native/README.md](native/README.md)。

开发实例使用独立目录、socket 和端口。测试不接管或修改日常代理的
系统代理、DNS、路由、TUN 或服务。发布签名与公证尚未实施；当前本地
Swift 应用只有 ad-hoc 签名。

mihomo 内核更新实现和接口继续保留，暂不自研或更换内核。当前原生
构建固定已校验版本作为迁移基线；原生更新入口尚待接入。应用自身的
上游自动更新链路已删除。旧 React/Tauri 及 Node 依赖暂用于未迁移功能的
对照和回归检查，待系统代理、DNS/TUN 与旧数据迁移接通后再删除。

## 维护

只在本机开发和编译；不维护容器、交叉编译、GitHub CI 或上游应用发布
流水线。保留许可证、来源说明和 Git 历史；旧发行记录可从 Git 历史查询。
直接在 `main` 验证并提交，使用仓库本地的签名设置，签名失败时停止提交：

```sh
git commit -s -S -m "fix(component): describe the change"
git verify-commit HEAD
```

React/Tauri 仍在迁移中，涉及旧壳时使用已安装的 Node/pnpm 运行相关检查：

```sh
pnpm typecheck
pnpm test
pnpm lint
pnpm format:check
pnpm arch:check
pnpm knip:check
pnpm gen:contract --check
```

删除旧界面后会一并删除这些工具和配置。AGENTS.md 只记录实际开发
约束；不保留其他重复的 AI 工具说明。
