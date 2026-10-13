import Foundation
import Observation

enum ClientPage: String, CaseIterable, Identifiable {
    case overview, profiles, nodes, rules, activity, unlock
    var id: Self { self }
    var title: String {
        switch self {
        case .overview: "概览"
        case .profiles: "配置"
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
    var busy = false
    var failure: ClientFailure?
    var showImport = false
    var showFailure = false
    var query = ""
    var inspectedNodeID: String?
    private let transport: any CoreTransport
    var isDemo: Bool { transport.isDemo }
    var selectedNode: NodeRecord? { snapshot?.nodes.first { $0.id == snapshot?.selectedNodeID } }
    var inspectedNode: NodeRecord? { snapshot?.nodes.first { $0.id == inspectedNodeID } }

    init(transport: any CoreTransport) { self.transport = transport }

    func poll() async {
        await refresh()
        while !Task.isCancelled {
            do { try await Task.sleep(for: .seconds(1)) } catch { return }
            await refresh()
        }
    }

    func refresh() async {
        do {
            accept(try await transport.snapshot())
            if failure?.code == "transport_error" { failure = nil }
        }
        catch { record(error) }
        loading = false
    }

    func command(_ name: String, payload: [String: String] = [:]) async {
        guard !busy else { return }
        busy = true
        defer { busy = false }
        do {
            let result = try await transport.execute(CommandEnvelope(expectedGeneration: snapshot?.generation, command: name, payload: payload))
            failure = nil
            accept(result)
        } catch { record(error); showFailure = !showImport }
    }

    func importProfile(name: String, source: String) async {
        await command("importProfile", payload: ["name": name, "source": source])
        if failure == nil { showImport = false }
    }

    private func accept(_ incoming: CoreSnapshot) {
        guard SnapshotOrdering.accepts(incoming, after: snapshot) else { return }
        snapshot = incoming
        if let error = incoming.error { failure = error }
    }

    private func record(_ error: Error) {
        failure = (error as? ClientFailure) ?? ClientFailure(code: "transport_error", message: "无法读取内核状态", detail: error.localizedDescription, retryable: true)
    }
}
