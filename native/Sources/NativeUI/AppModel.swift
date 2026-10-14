import Foundation
import Observation

enum ClientPage: String, CaseIterable, Identifiable {
    case overview, profiles, nodes, rules, activity, unlock
    var id: Self { self }
    var title: String {
        switch self {
        case .overview: "概览"
        case .profiles: "订阅"
        case .nodes: "节点"
        case .rules: "规则"
        case .activity: "活动"
        case .unlock: "解锁检测"
        }
    }
    var symbol: String {
        switch self {
        case .overview: "square.grid.2x2"
        case .profiles: "doc.text"
        case .nodes: "point.3.connected.trianglepath.dotted"
        case .rules: "list.bullet.rectangle"
        case .activity: "waveform.path.ecg"
        case .unlock: "lock.open"
        }
    }
}

@MainActor @Observable final class AppModel {
    var page: ClientPage? = .overview
    var snapshot: CoreSnapshot?
    var loading = true
    private var operations = 0
    var testingNodes = false
    private var testingRequestID: String?
    private var testingNames: Set<String> = []
    var testCompleted: Int {
        Set((snapshot?.nodes ?? []).filter {
            testingNames.contains($0.name) && $0.testRequestID == testingRequestID
        }.map(\.name)).count
    }
    var testTotal: Int { testingNames.count }
    @ObservationIgnored private var nodeTestTask: Task<Void, Never>?
    @ObservationIgnored private var refreshing = false
    @ObservationIgnored private var monitorTask: Task<Void, Never>?
    private var windowVisible = true
    var busy: Bool { operations > 0 }
    var failure: ClientFailure?
    var showImport = false
    var showFailure = false
    var showNetworkSetup = false
    @ObservationIgnored private var networkSetupCount = 0
    var networkPermission: NetworkPermission
    @ObservationIgnored private let permissionProbe: () -> NetworkPermission
    var query = ""
    var inspectedNodeID: String?
    private let transport: any CoreTransport
    var isDemo: Bool { transport.isDemo }
    var isIntegrationTest: Bool { transport.isIntegrationTest }
    var selectedNode: NodeRecord? { snapshot?.nodes.first { $0.id == snapshot?.selectedNodeID } }
    var inspectedNode: NodeRecord? { snapshot?.nodes.first { $0.id == inspectedNodeID } }

    init(transport: any CoreTransport, permissionProbe: (() -> NetworkPermission)? = nil) {
        self.transport = transport
        let probe = permissionProbe ?? { transport.isIntegrationTest ? NetworkPermission(stage: .isolated) : transport.isDemo ? NetworkPermission(stage: .demo) : NetworkPermission.current() }
        self.permissionProbe = probe
        self.networkPermission = probe()
        if transport.isDemo, let value = CommandLine.arguments.first(where: { $0.hasPrefix("--page=") }) {
            page = ClientPage(rawValue: String(value.dropFirst("--page=".count))) ?? .overview
        }
    }

    func poll() async {
        await refresh()
        while !Task.isCancelled {
            do { try await Task.sleep(for: .seconds(windowVisible ? 1 : 5)) } catch { return }
            await refresh()
        }
    }
    func startMonitoring() {
        guard monitorTask == nil else { return }
        monitorTask = Task { await poll() }
    }
    func setWindowVisibility(_ visible: Bool) {
        guard windowVisible != visible else { return }
        windowVisible = visible
        Task { await transport.setVisibility(visible) }
    }

    func networkSetupOpened() { networkSetupCount += 1 }
    func networkSetupClosed() { networkSetupCount = max(0, networkSetupCount - 1) }
    func presentNetworkSetup() { if networkSetupCount == 0 { showNetworkSetup = true } }
    func refreshNetworkPermission() { networkPermission = permissionProbe() }
    func refresh() async {
        refreshNetworkPermission()
        guard !refreshing else { return }
        refreshing = true
        defer { refreshing = false }
        do {
            accept(try await transport.snapshot())
            if failure?.code == "transport_error" { failure = nil }
        }
        catch { record(error) }
        loading = false
    }

    func command(_ name: String, payload: [String: String] = [:], requestID: String = UUID().uuidString) async {
        if !isDemo && !isIntegrationTest {
            let enablingNetwork = (name == "toggleSystemProxy" && snapshot?.networkFeatures.systemProxy != .on && snapshot?.networkFeatures.systemProxy != .failed)
                || (name == "toggleTun" && snapshot?.networkFeatures.tun != .on && snapshot?.networkFeatures.tun != .failed)
                || (["setSystemProxy", "setTun"].contains(name) && payload["enabled"] == "true")
                || (name == "setSystemDNS" && !(payload["servers"] ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            if enablingNetwork {
                refreshNetworkPermission()
                if !networkPermission.canEnable { presentNetworkSetup(); return }
            }
            if (name == "startCore" || enablingNetwork) && snapshot?.profiles.contains(where: { $0.active }) != true {
                showImport = true
                return
            }
        }
        operations += 1
        let testing = name == "testAllNodes" || name == "testNode"
        if testing { testingNodes = true }
        defer { operations -= 1; if testing { testingNodes = false } }
        do {
            let result = try await transport.execute(CommandEnvelope(requestID: requestID, expectedGeneration: snapshot?.generation, command: name, payload: payload))
            failure = nil
            accept(result)
        } catch {
            await refresh()
            if !(error is CancellationError) {
                record(error)
                if failure?.needsNetworkSetup == true { presentNetworkSetup(); showFailure = false }
                else { showFailure = !showImport }
            }
        }
    }

    func importProfile(name: String, source: String) async {
        await command("importProfile", payload: ["name": name, "source": source])
        if failure == nil { showImport = false }
    }

    func testNodes(nodeID: String? = nil) {
        guard !testingNodes else { return }
        testingNodes = true
        let requestID = UUID().uuidString
        testingRequestID = requestID
        testingNames = Set((snapshot?.nodes ?? []).filter { nodeID == nil || $0.id == nodeID }.map(\.name))
        nodeTestTask = Task {
            await command(nodeID == nil ? "testAllNodes" : "testNode", payload: nodeID.map { ["nodeID": $0] } ?? [:], requestID: requestID)
            nodeTestTask = nil
        }
    }
    func cancelNodeTests() { nodeTestTask?.cancel() }

    func shutdown() async {
        monitorTask?.cancel(); monitorTask = nil
        nodeTestTask?.cancel()
        await transport.shutdown()
    }

    private func accept(_ incoming: CoreSnapshot) {
        guard SnapshotOrdering.accepts(incoming, after: snapshot) else { return }
        snapshot = incoming
        if let error = incoming.error { failure = error }
        else if failure?.code == "core_failed" || failure?.code == "streamFailed" || failure?.code == "stream_failed" { failure = nil }
    }

    func report(_ error: Error) {
        failure = (error as? ClientFailure) ?? ClientFailure(code: "native_operation", message: "操作未完成", detail: error.localizedDescription, retryable: true)
        showFailure = true
    }

    private func record(_ error: Error) {
        failure = (error as? ClientFailure) ?? ClientFailure(code: "transport_error", message: "无法读取内核状态", detail: error.localizedDescription, retryable: true)
    }
}
