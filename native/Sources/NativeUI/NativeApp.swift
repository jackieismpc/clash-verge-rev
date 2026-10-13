import SwiftUI

// SDK 27 also exposes a State macro whose plugin is absent from Command Line Tools.
// Keep the stable property wrapper explicit so this package builds without Xcode.
typealias ViewState<Value> = SwiftUI.State<Value>

@main struct NativeApp: App {
    @ViewState private var model = AppModel(transport: DemoTransport())

    var body: some Scene {
        Window("Clash Native", id: "main") {
            RootView(model: model)
                .frame(minWidth: 980, minHeight: 680)
                .task { await model.poll() }
        }
        .defaultSize(width: 1280, height: 820)
        .windowToolbarStyle(.unified)
        .commands {
            CommandGroup(after: .newItem) {
                Button("导入配置…") { model.showImport = true }
                    .keyboardShortcut("o")
                Button("刷新状态") { Task { await model.refresh() } }
                    .keyboardShortcut("r")
            }
        }
        MenuBarExtra("Clash Native", systemImage: "network") {
            MenuContent(model: model)
        }
        Settings { PreferencesView(model: model) }
    }
}

struct MenuContent: View {
    @Bindable var model: AppModel
    @Environment(\.openWindow) private var openWindow
    var body: some View {
        Text(model.isDemo ? "演示模式 · 未修改系统网络" : "Clash Native")
        Text("当前节点：\(model.selectedNode?.name ?? "未选择")")
        Divider()
        Button("打开主窗口") { openWindow(id: "main") }
        Button("刷新状态") { Task { await model.refresh() } }
        SettingsLink { Text("设置…") }
        Divider()
        Button("退出 Clash Native") { NSApplication.shared.terminate(nil) }
            .keyboardShortcut("q")
    }
}

struct PreferencesView: View {
    @Bindable var model: AppModel
    @AppStorage("nativeAppearance") private var appearance = "system"
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
            Section("运行状态") {
                LabeledContent("数据来源", value: model.isDemo ? "演示样本" : "Rust 内核桥接")
                Picker("出站模式", selection: Binding(get: { model.snapshot?.mode ?? "rule" }, set: { mode in Task { await model.command("setMode", payload: ["mode": mode]) } })) {
                    Text("规则模式").tag("rule")
                    Text("全局模式").tag("global")
                    Text("直连模式").tag("direct")
                }.disabled(model.busy)
                HStack {
                    Button("启动内核") { Task { await model.command("startCore") } }
                    Button("停止内核") { Task { await model.command("stopCore") } }
                }.disabled(model.isDemo || model.busy)
                Text(model.isDemo ? "演示操作仅更新内存中的样本，不会启动代理、下载订阅或修改系统网络。" : "状态和命令由独立 Rust 内核管理。")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .padding(20)
        .frame(width: 480, height: 410)
    }
}

struct RootView: View {
    @Bindable var model: AppModel
    @AppStorage("nativeAppearance") private var appearance = "system"
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.colorSchemeContrast) private var contrast

    var body: some View {
        NavigationSplitView {
            List(selection: $model.page) {
                Section("工作空间") {
                    ForEach(ClientPage.allCases) { page in
                        Label(page.title, systemImage: page.symbol).tag(page)
                            .padding(.vertical, 7)
                    }
                }
            }
            .listStyle(.sidebar)
            .navigationSplitViewColumnWidth(min: 208, ideal: 224, max: 256)
            .safeAreaInset(edge: .bottom) {
                VStack(alignment: .leading, spacing: 8) {
                    Label(model.isDemo ? "演示样本" : "内核桥接", systemImage: model.isDemo ? "sparkles" : "network")
                    Text(model.isDemo ? "未修改系统网络" : "独立运行目录")
                        .font(.system(size: 12)).foregroundStyle(.secondary)
                }
                .font(.system(size: 14))
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(20)
            }
        } detail: {
            VStack(spacing: 0) {
                if let failure = model.failure {
                    HStack {
                        Label(failure.message, systemImage: "exclamationmark.triangle")
                        Spacer()
                        Button("查看详情") { model.showFailure = true }
                        if failure.retryable { Button("重试") { Task { await model.refresh() } } }
                    }
                    .font(.system(size: 12))
                    .padding(.horizontal, 24).padding(.vertical, 10)
                    .background(Color.orange.opacity(contrast == .increased ? 0.25 : 0.12))
                }
                if model.loading && model.snapshot == nil {
                    ProgressView("正在读取状态…").frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if model.snapshot == nil {
                    ContentUnavailableView {
                        Label("状态暂不可用", systemImage: "network.slash")
                    } description: {
                        Text("请查看错误详情，或重新读取内核状态。")
                    } actions: {
                        Button("重试") { Task { await model.refresh() } }
                    }
                } else {
                    pageContent
                }
            }
            .background(Color(nsColor: .windowBackgroundColor))
            .navigationTitle(model.page?.title ?? "概览")
            .toolbar {
                ToolbarItem(placement: .principal) {
                    StatusPill(demo: model.isDemo, reduceTransparency: reduceTransparency)
                }
                ToolbarItemGroup(placement: .primaryAction) {
                    Button { Task { await model.refresh() } } label: { Label("刷新", systemImage: "arrow.clockwise") }
                        .help("刷新状态（⌘R）")
                    Button { model.showImport = true } label: { Label("导入配置", systemImage: "plus") }
                        .help("导入配置（⌘O）")
                    SettingsLink { Label("设置", systemImage: "gearshape") }
                }
            }
            .searchable(text: $model.query, prompt: "搜索当前页面")
        }
        .preferredColorScheme(appearance == "light" ? .light : appearance == "dark" ? .dark : nil)
        .sheet(isPresented: $model.showImport) { ImportSheet(model: model) }
        .sheet(isPresented: $model.showFailure) { FailureSheet(failure: model.failure) }
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.16), value: model.page)
        .onChange(of: model.page) { _, _ in model.query = "" }
    }

    @ViewBuilder private var pageContent: some View {
        switch model.page ?? .overview {
        case .overview: OverviewView(model: model)
        case .nodes: NodesView(model: model)
        case .profiles: ProfilesView(model: model)
        case .rules: RulesView(model: model)
        case .activity: ActivityView(model: model)
        case .unlock: UnlockView(model: model)
        }
    }
}

struct StatusPill: View {
    var demo: Bool
    var reduceTransparency: Bool
    var body: some View {
        Label(demo ? "演示 · 代理未启动" : "内核状态", systemImage: demo ? "circle.dotted" : "circle.fill")
            .labelStyle(.titleAndIcon)
            .font(.system(size: 12))
            .padding(.horizontal, 12).padding(.vertical, 7)
            .background(reduceTransparency ? Color(nsColor: .controlBackgroundColor) : .clear, in: Capsule())
            .glassEffect(.regular, in: Capsule())
    }
}
