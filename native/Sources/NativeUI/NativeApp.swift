import SwiftUI

// SDK 27 also exposes a State macro whose plugin is absent from Command Line Tools.
// Keep the stable property wrapper explicit so this package builds without Xcode.
typealias ViewState<Value> = SwiftUI.State<Value>

@MainActor final class NativeAppDelegate: NSObject, NSApplicationDelegate {
    static var model: AppModel?
    private var terminating = false
    private var visibilityObservers: [NSObjectProtocol] = []
    func applicationDidFinishLaunching(_ notification: Notification) {
        Self.model?.startMonitoring()
        for name in [NSWindow.didChangeOcclusionStateNotification, NSWindow.didMiniaturizeNotification, NSWindow.didDeminiaturizeNotification, NSWindow.didBecomeKeyNotification, NSWindow.willCloseNotification] {
            visibilityObservers.append(NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { notification in
                let closingMain = name == NSWindow.willCloseNotification && (notification.object as? NSWindow)?.title == "Clash Native"
                MainActor.assumeIsolated {
                    if closingMain {
                        Self.model?.setWindowVisibility(false)
                    } else {
                        Self.model?.setWindowVisibility(NSApplication.shared.windows.contains { $0.title == "Clash Native" && $0.isVisible && !$0.isMiniaturized && $0.occlusionState.contains(.visible) })
                    }
                }
            })
        }
    }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard !terminating else { return .terminateLater }
        terminating = true
        Task {
            await Self.model?.shutdown()
            sender.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
    }
}

@main @MainActor struct NativeApp: App {
    @NSApplicationDelegateAdaptor(NativeAppDelegate.self) private var delegate
    @ViewState private var model: AppModel

    init() {
        let transport: any CoreTransport = CommandLine.arguments.contains("--demo")
            ? DemoTransport(scenario: CommandLine.arguments.first { $0.hasPrefix("--scenario=") }
                .flatMap { DemoScenario(rawValue: String($0.dropFirst("--scenario=".count))) } ?? .normal)
            : NativeCoreTransport(options: .application())
        let model = AppModel(transport: transport)
        _model = ViewState(initialValue: model)
        NativeAppDelegate.model = model
    }

    var body: some Scene {
        Window("Clash Native", id: "main") {
            RootView(model: model)
                .frame(minWidth: 980, minHeight: 628)
                .onAppear { model.startMonitoring(); model.setWindowVisibility(true) }
        }
        .defaultSize(width: model.isDemo && CommandLine.arguments.contains("--minimum-window") ? 980 : 1280,
                     height: model.isDemo && CommandLine.arguments.contains("--minimum-window") ? 680 : 820)
        .defaultLaunchBehavior(.presented)
        .windowToolbarStyle(.unified)
        .commands {
            CommandGroup(after: .newItem) {
                Button("导入配置…") { model.showImport = true }
                    .keyboardShortcut("o")
                Button("刷新状态") { Task { await model.refresh() } }
                    .keyboardShortcut("r")
            }
        }
        MenuBarExtra {
            MenuContent(model: model)
        } label: {
            NativeMenuLabel()
        }
        Settings { NativePreferencesView(model: model) }
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
                    Label(model.isDemo ? "演示样本" : "原生客户端", systemImage: model.isDemo ? "sparkles" : "network")
                    Text(model.isDemo ? "未修改系统网络" : "本地配置与状态")
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
            .background(Color(nsColor: .windowBackgroundColor).overlay(Color.indigo.opacity(0.035)))
            .navigationTitle(model.page?.title ?? "概览")
            .toolbar {
                ToolbarItem(placement: .principal) {
                    StatusPill(demo: model.isDemo, status: model.snapshot?.coreStatus ?? "stopped", reduceTransparency: reduceTransparency)
                }
                ToolbarItemGroup(placement: .primaryAction) {
                    NetworkFeatureControl(title: "系统代理", status: model.snapshot?.networkFeatures.systemProxy ?? .unavailable, demo: model.isDemo) {
                        Task { await model.command(model.isDemo ? "toggleSystemProxyDemo" : "toggleSystemProxy") }
                    }
                    NetworkFeatureControl(title: "TUN", status: model.snapshot?.networkFeatures.tun ?? .unavailable, demo: model.isDemo) {
                        Task { await model.command(model.isDemo ? "toggleTunDemo" : "toggleTun") }
                    }
                    Button { Task { await model.refresh() } } label: { Label("刷新", systemImage: "arrow.clockwise") }
                        .help("刷新状态（⌘R）")
                    Button { model.showImport = true } label: { Label("导入配置", systemImage: "plus") }
                        .help("导入配置（⌘O）")
                    SettingsLink { Label("设置", systemImage: "gearshape") }
                }
            }
            .searchable(text: $model.query, prompt: "搜索当前页面")
        }
        .preferredColorScheme(preferredScheme)
        .sheet(isPresented: $model.showImport) { ImportSheet(model: model) }
        .sheet(isPresented: $model.showFailure) { FailureSheet(failure: model.failure) }
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.16), value: model.page)
        .onChange(of: model.page) { _, _ in model.query = "" }
        .onAppear {
            guard model.isDemo else { return }
            DispatchQueue.main.async {
                guard let window = NSApplication.shared.windows.first(where: { $0.styleMask.contains(.titled) }) else { return }
                let minimum = CommandLine.arguments.contains("--minimum-window")
                window.isRestorable = false
                window.setFrameAutosaveName("")
                window.setFrame(NSRect(x: window.frame.minX, y: window.frame.minY, width: minimum ? 980 : 1280, height: minimum ? 680 : 820), display: true)
                window.center()
                if CommandLine.arguments.contains("--activate-demo") {
                    window.makeKeyAndOrderFront(nil)
                    NSApplication.shared.activate(ignoringOtherApps: true)
                }
            }
        }
    }

    private var preferredScheme: ColorScheme? {
        if model.isDemo {
            if CommandLine.arguments.contains("--appearance=light") { return .light }
            if CommandLine.arguments.contains("--appearance=dark") { return .dark }
        }
        return appearance == "light" ? .light : appearance == "dark" ? .dark : nil
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
    var status: String
    var reduceTransparency: Bool
    private var label: String {
        if demo { return "演示 · 代理未启动" }
        switch status { case "running": return "运行中"; case "starting": return "启动中"; case "failed": return "需要修复"; default: return "未运行" }
    }
    var body: some View {
        Label(label, systemImage: demo || status == "stopped" ? "circle.dotted" : "circle.fill")
            .foregroundStyle(status == "running" ? Color.green : status == "failed" ? Color.orange : Color.secondary)
            .labelStyle(.titleAndIcon)
            .font(.system(size: 12))
            .padding(.horizontal, 12).padding(.vertical, 7)
            .background(reduceTransparency ? Color(nsColor: .controlBackgroundColor) : .clear, in: Capsule())
            .glassEffect(.regular, in: Capsule())
    }
}

struct NativeMenuLabel: View {
    @Environment(\.openWindow) private var openWindow
    @ViewState private var opened = false
    var body: some View {
        Image(systemName: "network")
            .task {
                if !opened { opened = true; openWindow(id: "main") }
            }
    }
}
