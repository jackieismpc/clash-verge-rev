import Foundation
import ServiceManagement
import SwiftUI
import AppKit

struct NetworkPermission: Equatable, Sendable {
    enum Stage: Sendable { case development, unregistered, approvalRequired, approved, unavailable, isolated, demo }
    var stage: Stage
    var canEnable: Bool { stage == .approved }
    var canSetUp: Bool { stage != .isolated && stage != .demo }
    var actionTitle: String {
        switch stage {
        case .development: "需安装服务"
        case .unregistered: "需注册服务"
        case .approvalRequired: "等待批准"
        case .approved: "已批准"
        case .unavailable: "服务不可用"
        case .isolated: "隔离模式禁用"
        case .demo: "演示模式"
        }
    }
    @MainActor static func current() -> Self {
        guard Bundle.main.bundleURL.path == "/Applications/Clash Native.app" else { return Self(stage: .development) }
        switch SMAppService.daemon(plistName: "io.github.jackieismpc.clash-native.helper.plist").status {
        case .notRegistered: return Self(stage: .unregistered)
        case .requiresApproval: return Self(stage: .approvalRequired)
        case .enabled: return Self(stage: .approved)
        case .notFound: return Self(stage: .unavailable)
        @unknown default: return Self(stage: .unavailable)
        }
    }
    static func installationCommand(dataDirectory: String?) -> String? {
        guard let dataDirectory else { return nil }
        let url = URL(fileURLWithPath: dataDirectory)
        guard Array(url.pathComponents.suffix(3)) == [".work", "native-ui", "data"] else { return nil }
        let root = url.deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let script = root.appendingPathComponent("scripts/install-local.sh").path
        let quoted = "'" + script.replacingOccurrences(of: "'", with: "'\\''") + "'"
        return "sudo \(quoted)\nopen '/Applications/Clash Native.app'"
    }
}

struct NetworkSetupSheet: View {
    @Bindable var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @ViewState private var registrationError: String?
    @ViewState private var counted = false
    private var service: SMAppService { SMAppService.daemon(plistName: "io.github.jackieismpc.clash-native.helper.plist") }
    private var installCommand: String? { NetworkPermission.installationCommand(dataDirectory: Bundle.main.object(forInfoDictionaryKey: "CvnDevelopmentDataDir") as? String) }
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Label("准备系统网络权限", systemImage: "lock.shield").font(.title2)
            Text("普通内核运行与系统网络接管是两个步骤。权限准备完成后，再开启系统代理或 TUN。")
                .foregroundStyle(.secondary)
            LabeledContent("当前状态", value: model.networkPermission.actionTitle)
            if model.networkPermission.stage == .development {
                Text("请先退出本应用，执行下面命令安装，再打开安装版。已有订阅和配置保留。")
                if let installCommand {
                    Text(installCommand).font(.system(size: 12, design: .monospaced)).textSelection(.enabled)
                        .padding(12).frame(maxWidth: .infinity, alignment: .leading)
                        .background(.quaternary, in: RoundedRectangle(cornerRadius: 12))
                    Button("复制安装命令") { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(installCommand, forType: .string) }
                } else { Text("请从源仓库执行 sudo scripts/install-local.sh，再打开 /Applications/Clash Native.app。") }
            } else if model.networkPermission.stage == .unregistered {
                Text("注册本应用的独立权限服务，然后按 macOS 提示批准。注册本身不会开启代理。")
                Button("注册权限服务") { register() }
            } else if model.networkPermission.stage == .approvalRequired {
                Text("在系统设置的登录项与扩展中批准 Clash Native 的后台服务，然后返回这里重新检查。")
                Button("打开系统授权设置") { SMAppService.openSystemSettingsLoginItems() }
            } else if model.networkPermission.stage == .approved {
                Text("服务已批准。返回主窗口开启系统代理或 TUN；接管前请关闭其他应用的系统代理和 TUN。")
            } else {
                Text("请检查安装包与权限服务注册状态。必要时先注销服务，再重新安装和注册。")
            }
            if let registrationError { Text(registrationError).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
            if model.failure?.needsNetworkSetup == true {
                Text(model.failure?.detail ?? "").font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
            }
            HStack {
                if model.networkPermission.canEnable { Button("检查服务连接") { Task { await model.command("checkHelperStatus") } }.disabled(model.busy) }
                Button("重新检查") { Task { await model.refresh() } }
                Spacer()
                Button(model.networkPermission.canEnable ? "完成" : "稍后设置") { dismiss() }.keyboardShortcut(.defaultAction)
            }
        }.padding(24).frame(width: 600)
            .onAppear { if !counted { model.networkSetupOpened(); counted = true }; model.refreshNetworkPermission() }
            .onDisappear { if counted { model.networkSetupClosed(); counted = false } }
            .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in model.refreshNetworkPermission() }
    }
    private func register() {
        guard model.networkPermission.stage == .unregistered else { return }
        registrationError = nil
        do { try service.register(); model.refreshNetworkPermission() }
        catch { registrationError = "权限服务注册失败：" + error.localizedDescription }
    }
}
