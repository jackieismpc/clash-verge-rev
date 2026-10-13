import Foundation

struct NodeRecord: Codable, Identifiable, Sendable, Hashable {
    var id: String
    var name: String
    var kind: String
    var group: String
    var latencyMS: Int?
    var available: Bool
    var testedAt: Double? = nil
    var testURL: String? = nil
    var testRequestID: String? = nil
    var latencySort: Int { latencyMS ?? Int.max }
}

struct ProfileRecord: Codable, Identifiable, Sendable, Hashable {
    var id: String
    var name: String
    var source: String
    var active: Bool
    var updatedAt: Double?
    var updateIntervalMinutes: Int? = nil
}
struct RuleOverrideRecord: Codable, Sendable {
    var prepend: [String] = []
    var append: [String] = []
    var delete: [String] = []
}
struct CoreUpdateRecord: Codable, Sendable {
    var channel: String
    var version: String
    var candidatePath: String?
    var activated: Bool
}

struct RuleRecord: Codable, Identifiable, Sendable, Hashable {
    var id: String
    var kind: String
    var value: String
    var target: String
}

struct ConnectionRecord: Codable, Identifiable, Sendable, Hashable {
    var id: String
    var host: String
    var destination: String
    var rule: String
    var node: String
    var uploadBytes: UInt64
    var downloadBytes: UInt64
    var source: String? = nil
    var network: String? = nil
    var process: String? = nil
    var startedAt: String? = nil
    var rulePayload: String? = nil
}

struct ProviderRecord: Codable, Identifiable, Sendable, Hashable {
    var id: String { category + ":" + name }
    var name: String
    var category: String
    var vehicle: String
    var behavior: String
    var count: Int
    var updatedAt: String?
}

enum NetworkFeatureStatus: String, Codable, Sendable, CaseIterable {
    case unavailable, off, on, applying, failed
    var title: String {
        switch self {
        case .unavailable: "尚未接通"
        case .off: "已关闭"
        case .on: "已开启"
        case .applying: "正在应用…"
        case .failed: "需要修复"
        }
    }
}

struct NetworkFeatures: Codable, Sendable {
    var systemProxy: NetworkFeatureStatus = .unavailable
    var tun: NetworkFeatureStatus = .unavailable
    var dns: NetworkFeatureStatus = .off
}

enum LogPresentation {
    static func filter(_ records: [LogRecord], level: String, query: String) -> [LogRecord] {
        records.filter {
            (level == "all" || $0.level.lowercased() == level) &&
            (query.isEmpty || $0.message.localizedCaseInsensitiveContains(query))
        }
    }
}

struct LogRecord: Codable, Identifiable, Sendable, Hashable {
    var id: String
    var timestamp: Double
    var level: String
    var message: String
}

struct TrafficSample: Codable, Identifiable, Sendable, Hashable {
    var timestamp: Double
    var uploadBytesPerSecond: Double?
    var downloadBytesPerSecond: Double?
    var id: Double { timestamp }
}

struct ClientFailure: Codable, Identifiable, Sendable, Hashable, Error {
    var code: String
    var message: String
    var detail: String
    var retryable: Bool
    var id: String { code + message }
}

struct UnlockRecord: Codable, Identifiable, Sendable, Hashable {
    var id: String
    var status: String
    var region: String?
    var checkedAt: String?
}

struct BackupRecord: Codable, Identifiable, Sendable, Hashable {
    var id: String
    var createdAt: Double
    var bytes: UInt64
}

struct CoreSnapshot: Codable, Sendable {
    var schemaVersion: Int = 1
    var generation: UInt64
    var sequence: UInt64
    var mode: String
    var coreStatus: String
    var selectedNodeID: String?
    var nodes: [NodeRecord]
    var profiles: [ProfileRecord]
    var rules: [RuleRecord]
    var connections: [ConnectionRecord]
    var logs: [LogRecord]
    var traffic: [TrafficSample]
    var error: ClientFailure?
    var unlock: [UnlockRecord] = []
    var backups: [BackupRecord] = []
    var overrides: String = "{}"
    var profileDocuments: [String: String] = [:]
    var providers: [ProviderRecord] = []
    var networkFeatures: NetworkFeatures = NetworkFeatures()
    var ruleOverrides: RuleOverrideRecord = RuleOverrideRecord()
    var migrationWarnings: [String] = []
    var coreUpdate: CoreUpdateRecord? = nil
}

struct CommandEnvelope: Codable, Sendable {
    var schemaVersion: Int = 1
    var requestID: String = UUID().uuidString
    var expectedGeneration: UInt64?
    var command: String
    var payload: [String: String] = [:]
}

protocol CoreTransport: Sendable {
    var isDemo: Bool { get }
    var isIntegrationTest: Bool { get }
    func snapshot() async throws -> CoreSnapshot
    func execute(_ command: CommandEnvelope) async throws -> CoreSnapshot
    func shutdown() async
    func setVisibility(_ visible: Bool) async
}

extension CoreTransport {
    var isIntegrationTest: Bool { false }
    func shutdown() async {}
    func setVisibility(_ visible: Bool) async {}
}

enum SnapshotOrdering {
    static func accepts(_ incoming: CoreSnapshot, after current: CoreSnapshot?) -> Bool {
        guard incoming.schemaVersion == 1 else { return false }
        guard let current else { return true }
        return incoming.generation > current.generation ||
            (incoming.generation == current.generation && incoming.sequence >= current.sequence)
    }
}
