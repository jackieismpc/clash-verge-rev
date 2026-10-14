import SwiftUI
import AppKit
import ServiceManagement

struct NativePreferencesView: View {
    @Bindable var model: AppModel
    var body: some View {
        TabView {
            GeneralPreferences(model: model).tabItem { Label("通用", systemImage: "gearshape") }
            NetworkPreferences(model: model).tabItem { Label("网络", systemImage: "network") }
            CoreUpdatePreferences(model: model).tabItem { Label("内核更新", systemImage: "arrow.down.circle") }
            BackupPreferences(model: model).tabItem { Label("本地备份", systemImage: "externaldrive") }
            Form {
                LabeledContent("应用", value: "Clash Native 0.1.0")
                LabeledContent("系统要求", value: "Apple Silicon · macOS 26+")
                LabeledContent("许可证", value: "GPL-3.0")
                Link("原项目 Clash Verge Rev", destination: URL(string: "https://github.com/clash-verge-rev/clash-verge-rev")!)
            }.formStyle(.grouped).tabItem { Label("关于", systemImage: "info.circle") }
        }.padding(16).frame(width: 600, height: 510)
    }
}

private struct GeneralPreferences: View {
    @Bindable var model: AppModel
    @AppStorage("nativeAppearance") private var appearance = "system"
    @ViewState private var loginEnabled = SMAppService.mainApp.status == .enabled
    var body: some View {
        Form {
            Section("外观") {
                Picker("颜色模式", selection: $appearance) {
                    Text("跟随系统").tag("system")
                    Text("浅色").tag("light")
                    Text("深色").tag("dark")
                }
                Text("透明度、动态效果与对比度遵循 macOS 辅助功能设置。")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section("启动与后台") {
                Toggle("登录时打开", isOn: Binding(get: { loginEnabled }, set: { enabled in
                    do {
                        if enabled { try SMAppService.mainApp.register() }
                        else { try SMAppService.mainApp.unregister() }
                        loginEnabled = SMAppService.mainApp.status == .enabled
                    } catch { model.report(error) }
                })).disabled(model.isDemo || model.isIntegrationTest)
                Text("关闭主窗口后继续后台代理，通过菜单栏重开窗口。退出应用会停止所属内核。")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }.formStyle(.grouped)
    }
}

private struct NetworkPreferences: View {
    @Bindable var model: AppModel
    @ViewState private var showOverrides = false
    @ViewState private var showNetworkSetup = false
    @ViewState private var dns = ""
    private var service: SMAppService { SMAppService.daemon(plistName: "io.github.jackieismpc.clash-native.helper.plist") }
    var body: some View {
        Form {
            Section("代理内核") {
                LabeledContent("状态", value: model.snapshot?.coreStatus == "running" ? "运行中" : "未运行")
                Picker("出站模式", selection: Binding(get: { model.snapshot?.mode ?? "rule" }, set: { mode in Task { await model.command("setMode", payload: ["mode": mode]) } })) {
                    Text("规则模式").tag("rule")
                    Text("全局模式").tag("global")
                    Text("直连模式").tag("direct")
                }.disabled(model.busy)
                HStack {
                    Button("启动内核") { Task { await model.command("startCore") } }
                        .disabled(model.busy || model.snapshot?.coreStatus == "running")
                    Button("停止内核") { Task { await model.command("stopCore") } }
                        .disabled(!["running", "failed"].contains(model.snapshot?.coreStatus ?? "stopped"))
                }.disabled(model.isDemo)
            }
            Section("系统网络") {
                Toggle("系统代理", isOn: Binding(get: { model.snapshot?.networkFeatures.systemProxy == .on || model.snapshot?.networkFeatures.systemProxy == .failed }, set: { enabled in
                    if enabled && !model.networkPermission.canEnable { showNetworkSetup = true }
                    else { Task { await model.command("setSystemProxy", payload: ["enabled": String(enabled)]) } }
                }))
                Toggle("TUN 增强模式", isOn: Binding(get: { model.snapshot?.networkFeatures.tun == .on || model.snapshot?.networkFeatures.tun == .failed }, set: { enabled in
                    if enabled && !model.networkPermission.canEnable { showNetworkSetup = true }
                    else { Task { await model.command("setTun", payload: ["enabled": String(enabled)]) } }
                }))
                if !model.networkPermission.canEnable {
                    Button(model.networkPermission.actionTitle + "…") { showNetworkSetup = true }
                }
                TextField("系统 DNS（公共或 LAN 地址，用逗号分隔）", text: $dns)
                HStack {
                    Button("应用 DNS") {
                        if !model.networkPermission.canEnable { showNetworkSetup = true }
                        else { Task { await model.command("setSystemDNS", payload: ["servers": dns]) } }
                    }.disabled(dns.isEmpty)
                    Button("恢复所属系统代理与 DNS") { Task { await model.command("restoreNetwork") } }
                }
                Text("系统代理、DNS 与 TUN 由独立权限服务监护；应用消失时恢复所属设置，保留其他应用后续修改。")
                    .font(.caption).foregroundStyle(.secondary)
            }.disabled(model.isDemo || model.isIntegrationTest || model.busy)
            Section("网络权限服务") {
                Text("TUN 使用真实 IP 的 DNS 模式，避免关闭后系统缓存保留本应用的 Fake IP。关闭代理模式或退出会恢复所属系统 DNS。")
                    .font(.caption).foregroundStyle(.secondary)
                Text(model.networkPermission.actionTitle)
                Text("接管系统网络需要将应用安装到 /Applications/Clash Native.app，并授予独立服务权限。不会使用现有 Clash Verge 的服务。")
                    .font(.caption).foregroundStyle(.secondary)
                HStack {
                    Button("准备权限服务…") { showNetworkSetup = true }
                    Button("重新检查") { model.refreshNetworkPermission() }
                    Button("注销服务") {
                        Task {
                            do {
                                await model.command("setTun", payload: ["enabled": "false"])
                                guard model.failure == nil else { return }
                                await model.command("restoreNetwork")
                                guard model.failure == nil else { return }
                                try await service.unregister()
                                model.refreshNetworkPermission()
                            } catch { model.report(error) }
                        }
                    }.disabled(model.networkPermission.stage == .development || model.networkPermission.stage == .unregistered)
                }
            }.disabled(model.isDemo || model.isIntegrationTest || model.busy)
            Section("配置") {
                Button("高级 YAML 覆写…") { showOverrides = true }.disabled(model.isDemo)
            }
        }.formStyle(.grouped)
            .sheet(isPresented: $showNetworkSetup) { NetworkSetupSheet(model: model) }
            .sheet(isPresented: $showOverrides) { OverridesEditor(model: model) }
    }
}

struct OverridesEditor: View {
    @Bindable var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @ViewState private var yaml = ""
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("高级 YAML 覆写").font(.title2)
            Text("覆写独立保存，订阅更新后仍保留。列表字段会替换原列表。")
                .font(.caption).foregroundStyle(.secondary)
            TextEditor(text: $yaml).font(.system(size: 12, design: .monospaced)).frame(minHeight: 300)
            if let error = model.failure { Text(error.detail).font(.caption).foregroundStyle(.red).textSelection(.enabled) }
            HStack {
                Spacer()
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("保存") { Task { await model.command("setOverrides", payload: ["yaml": yaml]); if model.failure == nil { dismiss() } } }
                    .keyboardShortcut(.defaultAction).disabled(model.busy)
            }
        }.padding(24).frame(width: 700, height: 460)
            .onAppear { yaml = model.snapshot?.overrides ?? "{}" }
    }
}

private struct BackupPreferences: View {
    @Bindable var model: AppModel
    @ViewState private var restoring: BackupRecord?
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text("本地备份").font(.title2)
                Spacer()
                Button("导入旧数据…") {
                    let panel = NSOpenPanel()
                    panel.canChooseDirectories = true
                    panel.canChooseFiles = true
                    panel.allowsMultipleSelection = false
                    panel.message = "选择旧 Clash Verge 数据目录或 ZIP 备份。源文件只读保留；自定义 JavaScript 不执行。"
                    if panel.runModal() == .OK, let url = panel.url {
                        Task { await model.command("importLegacyData", payload: ["path": url.path]) }
                    }
                }.disabled(model.isDemo || model.busy)
                Button("创建备份") { Task { await model.command("createBackup") } }.disabled(model.isDemo)
            }
            Text("原生 JSON 备份包含本应用配置。旧目录或 ZIP 导入先保存私有回退副本；自定义 JavaScript 只保留并提示迁到 YAML。恢复会替换当前配置。")
                .font(.caption).foregroundStyle(.secondary)
            if let failure = model.failure {
                Text(failure.detail).font(.caption).foregroundStyle(.red).textSelection(.enabled)
            }
            ForEach(model.snapshot?.migrationWarnings ?? [], id: \.self) { warning in
                Text(warning).font(.caption).foregroundStyle(.orange).textSelection(.enabled)
            }
            if model.snapshot?.backups.isEmpty ?? true { emptyState("暂无备份", "创建备份后可在这里恢复。", symbol: "externaldrive") }
            else {
                List(model.snapshot?.backups ?? []) { backup in
                    HStack {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(Date(timeIntervalSince1970: backup.createdAt), style: .date)
                            Text(backup.id).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                        }
                        Spacer()
                        Text(ByteCountFormatter.string(fromByteCount: Int64(clamping: backup.bytes), countStyle: .file))
                            .font(.caption).foregroundStyle(.secondary)
                        Button("恢复…") { restoring = backup }.disabled(model.busy)
                    }.padding(.vertical, 4)
                }
            }
        }.padding(16)
            .task { if !model.isDemo { await model.command("listBackups") } }
            .confirmationDialog("恢复备份并替换当前配置？", isPresented: Binding(get: { restoring != nil }, set: { if !$0 { restoring = nil } })) {
                Button("恢复备份", role: .destructive) {
                    if let backup = restoring { Task { await model.command("restoreBackup", payload: ["backupID": backup.id]) } }
                    restoring = nil
                }
            }
    }
}


private struct CoreUpdatePreferences: View {
    @Bindable var model: AppModel
    @ViewState private var channel = "stable"

    var body: some View {
        Form {
            Section("mihomo 更新") {
                LabeledContent("固定基线", value: "v1.19.32 · Apple Silicon")
                Picker("更新通道", selection: $channel) {
                    Text("稳定版").tag("stable")
                    Text("Alpha 预览版").tag("alpha")
                }
                Button("检查官方版本") { Task { await model.command("checkCoreUpdate", payload: ["channel": channel]) } }
                Button("下载并验证候选") { Task { await model.command("stageCoreUpdate", payload: ["channel": channel]) } }
                Text("候选只存入本应用私有目录。验证通过后仍须明确启用；原有固定基线保留用于失败回滚。")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section("启用已验证候选") {
                if let update = model.snapshot?.coreUpdate {
                    LabeledContent("官方版本", value: update.version)
                    if let path = update.candidatePath {
                        Text(path).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                    }
                    if update.activated { Text("已启用私有候选").foregroundStyle(.secondary) }
                } else { Text("请先检查版本并下载候选。 ").foregroundStyle(.secondary) }
                Button("启用候选并重启所属内核") {
                    Task { await model.command("activateCoreUpdate", payload: ["candidatePath": model.snapshot?.coreUpdate?.candidatePath ?? ""]) }
                }.disabled(model.snapshot?.coreUpdate?.candidatePath == nil || model.snapshot?.coreUpdate?.activated == true)
            }
            Button("恢复固定基线并重启所属内核") { Task { await model.command("resetCoreUpdate") } }
            Text("特权 TUN 使用安装包内受签名保护的固定内核。启用 TUN 前，请先恢复固定基线。")
                .font(.caption).foregroundStyle(.secondary)
            if let failure = model.failure {
                Text(failure.detail).font(.caption).foregroundStyle(.red).textSelection(.enabled)
            }
        }.formStyle(.grouped).disabled(model.isDemo || model.busy || model.isIntegrationTest)
    }
}
