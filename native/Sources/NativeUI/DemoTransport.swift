import Foundation

actor DemoTransport: CoreTransport {
    nonisolated let isDemo = true
    private var state: CoreSnapshot
    private var tick = 0

    init() {
        let now = Date().timeIntervalSince1970
        let history = (0..<300).map { index in
            Self.sample(timestamp: now - Double(299 - index), tick: index)
        }
        state = CoreSnapshot(
            generation: 1, sequence: 1, mode: "rule", coreStatus: "demo",
            selectedNodeID: "hk-01",
            nodes: [
                NodeRecord(id: "hk-01", name: "香港 · 低延迟线路", kind: "AnyTLS", group: "自动选择", latencyMS: 28, available: true),
                NodeRecord(id: "jp-01", name: "日本 · 东京", kind: "Shadowsocks", group: "全球节点", latencyMS: 61, available: true),
                NodeRecord(id: "sg-01", name: "新加坡 · 稳定线路", kind: "Trojan", group: "全球节点", latencyMS: 76, available: true),
                NodeRecord(id: "us-01", name: "美国 · 洛杉矶 · 较长的节点名称用于检查文字截断与辅助说明", kind: "VMess", group: "全球节点", latencyMS: 168, available: true),
                NodeRecord(id: "offline", name: "备用线路", kind: "AnyTLS", group: "全球节点", latencyMS: nil, available: false),
            ],
            profiles: [ProfileRecord(id: "demo-profile", name: "演示配置", source: "本地演示样本", active: true, updatedAt: now)],
            rules: [
                RuleRecord(id: "r1", kind: "DOMAIN-SUFFIX", value: "example.com", target: "自动选择"),
                RuleRecord(id: "r2", kind: "GEOIP", value: "CN", target: "DIRECT"),
                RuleRecord(id: "r3", kind: "MATCH", value: "所有剩余请求", target: "全球节点"),
            ],
            connections: [
                ConnectionRecord(id: "c1", host: "www.example.com", destination: "203.0.113.10:443", rule: "DOMAIN-SUFFIX", node: "香港 · 低延迟线路", uploadBytes: 20480, downloadBytes: 1835008),
                ConnectionRecord(id: "c2", host: "cdn.example.org", destination: "198.51.100.20:443", rule: "MATCH", node: "日本 · 东京", uploadBytes: 8192, downloadBytes: 6291456),
            ],
            logs: [LogRecord(id: "l1", timestamp: now, level: "info", message: "已载入演示样本。此窗口不会启动代理内核或修改系统网络。")],
            traffic: history, error: nil
        )
    }

    func snapshot() async throws -> CoreSnapshot {
        tick += 1
        state.sequence += 1
        state.traffic.append(Self.sample(timestamp: Date().timeIntervalSince1970, tick: tick + 300))
        state.traffic = Array(state.traffic.suffix(300))
        return state
    }

    func execute(_ command: CommandEnvelope) async throws -> CoreSnapshot {
        guard command.expectedGeneration == nil || command.expectedGeneration == state.generation else {
            throw ClientFailure(code: "stale_generation", message: "状态已更新，请重试", detail: "The command generation no longer matches the transport generation.", retryable: true)
        }
        switch command.command {
        case "refresh": break
        case "selectNode":
            guard let id = command.payload["nodeID"], let node = state.nodes.first(where: { $0.id == id }), node.available else {
                throw ClientFailure(code: "node_unavailable", message: "该节点暂不可用", detail: "Select an available node in the demonstration fixture.", retryable: false)
            }
            state.selectedNodeID = id
        case "importProfile":
            guard let source = command.payload["source"], !source.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw ClientFailure(code: "invalid_source", message: "请输入配置来源", detail: "A nonempty source is required.", retryable: false)
            }
            state.profiles.append(ProfileRecord(id: UUID().uuidString, name: command.payload["name"] ?? "导入的演示配置", source: source, active: false, updatedAt: Date().timeIntervalSince1970))
        case "selectProfile":
            guard let id = command.payload["profileID"], state.profiles.contains(where: { $0.id == id }) else {
                throw ClientFailure(code: "profile_missing", message: "配置已不存在", detail: "The requested profile ID does not exist in the demonstration fixture.", retryable: false)
            }
            for index in state.profiles.indices { state.profiles[index].active = state.profiles[index].id == id }
        case "setMode": state.mode = command.payload["mode"] ?? "rule"
        case "closeConnection": state.connections.removeAll { $0.id == command.payload["connectionID"] }
        case "clearLogs": state.logs = []
        default:
            throw ClientFailure(code: "demo_only", message: "演示模式不执行此操作", detail: "The fixture transport has no operating-system or core side effects.", retryable: false)
        }
        state.sequence += 1
        return state
    }

    private static func sample(timestamp: Double, tick: Int) -> TrafficSample {
        let gap = tick % 83 == 0 || tick % 83 == 1
        return TrafficSample(timestamp: timestamp,
                             uploadBytesPerSecond: gap ? nil : 10240 + 8000 * (1 + sin(Double(tick) / 13)),
                             downloadBytesPerSecond: gap ? nil : 134144 + 85000 * (1 + sin(Double(tick) / 19)))
    }
}
