import SwiftUI
import ServiceManagement

struct NativePreferencesView: View {
    @Bindable var model: AppModel
    var body: some View {
        TabView {
            GeneralPreferences(model: model).tabItem { Label("通用", systemImage: "gearshape") }
            NetworkPreferences(model: model).tabItem { Label("网络", systemImage: "network") }
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
                })).disabled(model.isDemo)
                Text("关闭主窗口后继续后台代理，通过菜单栏重开窗口。退出应用会停止所属内核。")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }.formStyle(.grouped)
    }
}

private struct NetworkPreferences: View {
    @Bindable var model: AppModel
    @ViewState private var showOverrides = false
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
                        .disabled(model.snapshot?.coreStatus != "running")
                }.disabled(model.isDemo)
            }
            Section("系统网络") {
                LabeledContent("系统代理", value: "权限服务尚未接通")
                LabeledContent("TUN", value: "权限服务尚未接通")
                Text("当前内核仅监听独立的本地端口，未修改系统代理、DNS 或路由。")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section("配置") {
                Button("高级 YAML 覆写…") { showOverrides = true }.disabled(model.isDemo)
            }
        }.formStyle(.grouped)
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
                Button("创建备份") { Task { await model.command("createBackup") } }.disabled(model.isDemo)
            }
            Text("备份包含本应用的订阅、配置、覆写及设置。恢复会替换当前配置。")
                .font(.caption).foregroundStyle(.secondary)
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
