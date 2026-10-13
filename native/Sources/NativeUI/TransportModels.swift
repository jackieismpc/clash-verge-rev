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
    var latencySort: Int { latencyMS ?? Int.max }
}

struct ProfileRecord: Codable, Identifiable, Sendable, Hashable {
    var id: String
    var name: String
    var source: String
    var active: Bool
    var updatedAt: Double?
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
    func snapshot() async throws -> CoreSnapshot
    func execute(_ command: CommandEnvelope) async throws -> CoreSnapshot
    func shutdown() async
}

extension CoreTransport {
    func shutdown() async {}
}

enum SnapshotOrdering {
    static func accepts(_ incoming: CoreSnapshot, after current: CoreSnapshot?) -> Bool {
        guard incoming.schemaVersion == 1 else { return false }
        guard let current else { return true }
        return incoming.generation > current.generation ||
            (incoming.generation == current.generation && incoming.sequence >= current.sequence)
    }
}
