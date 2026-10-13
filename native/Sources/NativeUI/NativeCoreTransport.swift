import Foundation
import Darwin

enum WireValue: Codable, Sendable, Equatable {
    case object([String: WireValue]), array([WireValue]), string(String), number(Double), bool(Bool), null

    init(from decoder: any Decoder) throws {
        let value = try decoder.singleValueContainer()
        if value.decodeNil() { self = .null }
        else if let item = try? value.decode(Bool.self) { self = .bool(item) }
        else if let item = try? value.decode(Double.self) { self = .number(item) }
        else if let item = try? value.decode(String.self) { self = .string(item) }
        else if let item = try? value.decode([WireValue].self) { self = .array(item) }
        else { self = .object(try value.decode([String: WireValue].self)) }
    }
    func encode(to encoder: any Encoder) throws {
        var value = encoder.singleValueContainer()
        switch self {
        case .object(let item): try value.encode(item)
        case .array(let item): try value.encode(item)
        case .string(let item): try value.encode(item)
        case .number(let item): try value.encode(item)
        case .bool(let item): try value.encode(item)
        case .null: try value.encodeNil()
        }
    }
    subscript(_ key: String) -> WireValue { if case .object(let items) = self { return items[key] ?? .null }; return .null }
    var text: String? { if case .string(let text) = self { return text }; return nil }
    var number: Double? { if case .number(let number) = self { return number }; return nil }
    var items: [WireValue] { if case .array(let items) = self { return items }; return [] }
    var fields: [String: WireValue] { if case .object(let items) = self { return items }; return [:] }
    var flag: Bool { if case .bool(let value) = self { return value }; return false }
}

private struct NativeEnvelope: Decodable, Sendable {
    var version: Int
    var id: String
    var kind: String
    var coreGeneration: UInt64
    var revision: UInt64
    var ok: Bool
    var timestamp: Double?
    var data: WireValue?
    var error: NativeError?
    struct NativeError: Decodable, Sendable { var code: String; var message: String }
}

struct NativeCoreOptions: Sendable {
    var libraryPath: String
    var dataDirectory: String
    var mihomoPath: String?
    var mixedPort: UInt16 = 17890

    static func application() -> Self {
        let environment = ProcessInfo.processInfo.environment
        let bundle = Bundle.main.bundleURL.appendingPathComponent("Contents")
        let data = environment["CVN_DATA_DIRECTORY"] ?? Bundle.main.object(forInfoDictionaryKey: "CvnDevelopmentDataDir") as? String
            ?? URL.applicationSupportDirectory.appendingPathComponent("ClashNative").path
        return Self(
            libraryPath: environment["CVN_LIBRARY_PATH"] ?? bundle.appendingPathComponent("Frameworks/libclash_verge_native_core.dylib").path,
            dataDirectory: data,
            mihomoPath: environment["CVN_MIHOMO_PATH"] ?? bundle.appendingPathComponent("Resources/mihomo").path
        )
    }
}

// This object is confined to NativeCoreTransport's executor. It never crosses
// the main actor, and its handle is destroyed only after pending calls finish.
private final class NativeLibrary {
    typealias Create = @convention(c) (UnsafePointer<CChar>?) -> UnsafeMutableRawPointer?
    typealias Read = @convention(c) (UnsafeMutableRawPointer?) -> UnsafeMutablePointer<CChar>?
    typealias Submit = @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>?) -> Int32
    typealias Free = @convention(c) (UnsafeMutablePointer<CChar>?) -> Void
    typealias Destroy = @convention(c) (UnsafeMutableRawPointer?) -> Void
    typealias LastError = @convention(c) () -> UnsafeMutablePointer<CChar>?
    typealias Version = @convention(c) () -> UInt32
    let library: UnsafeMutableRawPointer
    let handle: UnsafeMutableRawPointer
    let submit: Submit
    let cancel: Submit
    let snapshot: Read
    let nextEvent: Read
    let free: Free
    let stop: Destroy
    let destroy: Destroy

    init(options: NativeCoreOptions) throws {
        guard let library = dlopen(options.libraryPath, RTLD_NOW | RTLD_LOCAL) else {
            throw ClientFailure(code: "library_unavailable", message: "无法加载 Rust 业务库", detail: dlerror().map { String(cString: $0) } ?? options.libraryPath, retryable: true)
        }
        do {
            func symbol<T>(_ name: String, _: T.Type) throws -> T {
                guard let address = dlsym(library, name) else { throw ClientFailure(code: "abi_mismatch", message: "业务库接口不完整", detail: name, retryable: false) }
                return unsafeBitCast(address, to: T.self)
            }
            let version = try symbol("cvn_abi_version", Version.self)
            guard version() == 1 else { throw ClientFailure(code: "abi_mismatch", message: "业务库版本不兼容", detail: "Expected ABI 1", retryable: false) }
            let create = try symbol("cvn_create", Create.self)
            let lastError = try symbol("cvn_last_error", LastError.self)
            self.free = try symbol("cvn_free_string", Free.self)
            self.submit = try symbol("cvn_submit", Submit.self)
            self.cancel = try symbol("cvn_cancel", Submit.self)
            self.snapshot = try symbol("cvn_snapshot", Read.self)
            self.nextEvent = try symbol("cvn_next_event", Read.self)
            self.stop = try symbol("cvn_stop", Destroy.self)
            self.destroy = try symbol("cvn_destroy", Destroy.self)
            var config: [String: WireValue] = ["version": .number(1), "dataDir": .string(options.dataDirectory), "mixedPort": .number(Double(options.mixedPort))]
            if let path = options.mihomoPath { config["mihomoPath"] = .string(path) }
            let bytes = try JSONEncoder().encode(WireValue.object(config))
            let text = String(decoding: bytes, as: UTF8.self)
            guard let handle = text.withCString({ create($0) }) else {
                let pointer = lastError()
                let detail = pointer.map { String(cString: $0) } ?? "cvn_create failed"
                free(pointer)
                throw ClientFailure(code: "initialization_failed", message: "无法初始化独立运行目录", detail: detail, retryable: true)
            }
            self.handle = handle
            self.library = library
        } catch { dlclose(library); throw error }
    }
    deinit { destroy(handle); dlclose(library) }
    func read(_ pointer: UnsafeMutablePointer<CChar>?) throws -> NativeEnvelope? {
        guard let pointer else { return nil }
        defer { free(pointer) }
        let bytes = Data(bytes: pointer, count: strlen(pointer))
        let envelope = try JSONDecoder().decode(NativeEnvelope.self, from: bytes)
        guard envelope.version == 1 else { throw ClientFailure(code: "abi_mismatch", message: "业务数据版本不兼容", detail: "Expected schema 1", retryable: false) }
        return envelope
    }
}

actor NativeCoreTransport: CoreTransport {
    nonisolated let isDemo = false
    private let options: NativeCoreOptions
    private var library: NativeLibrary?
    private var stopped = false
    private var state = WireValue.null
    private var results: [String: NativeEnvelope] = [:]
    private var pending: Set<String> = []
    private var generation: UInt64 = 0
    private var sequence: UInt64 = 0
    private var revision: UInt64 = 0
    private var projectionRevision: UInt64?
    private var projectionGeneration: UInt64?
    private var subscribedGeneration: UInt64?
    private var nodes: [NodeRecord] = []
    private var selectedNodeID: String?
    private var rules: [RuleRecord] = []
    private var providers: [ProviderRecord] = []
    private var connections: [ConnectionRecord] = []
    private var logs: [LogRecord] = []
    private var traffic: [TrafficSample] = []
    private var unlock: [UnlockRecord] = []
    private var backups: [BackupRecord] = []
    private var failure: ClientFailure?
    private var lastProjection = Date.distantPast

    init(options: NativeCoreOptions) { self.options = options }

    private func open() throws -> NativeLibrary {
        guard !stopped else { throw CancellationError() }
        if let library { return library }
        let loaded = try NativeLibrary(options: options)
        guard let initial = try loaded.read(loaded.snapshot(loaded.handle)), let data = initial.data else {
            throw ClientFailure(code: "snapshot_missing", message: "初始化状态为空", detail: "cvn_snapshot", retryable: true)
        }
        self.library = loaded
        acceptState(initial, data: data)
        return loaded
    }

    func snapshot() async throws -> CoreSnapshot {
        _ = try open()
        try drain()
        if unlock.isEmpty { unlock = try await request("media.list").items.compactMap(unlockRecord) }
        if state["runtime"]["status"].text == "running" {
            if subscribedGeneration != generation {
                _ = try await request("core.subscribe", args: ["streams": .array(["traffic", "logs", "connections"].map(WireValue.string))])
                subscribedGeneration = generation
            }
            if pending.isEmpty && (projectionGeneration != generation || projectionRevision != revision || Date().timeIntervalSince(lastProjection) >= 5) {
                try await reloadProjection()
            }
        }
        return projectedSnapshot()
    }

    func execute(_ command: CommandEnvelope) async throws -> CoreSnapshot {
        _ = try open()
        try drain()
        if let expected = command.expectedGeneration, expected != generation {
            throw ClientFailure(code: "stale_generation", message: "内核已切换，请刷新后重试", detail: "Expected \(expected), current \(generation)", retryable: true)
        }
        let payload = command.payload
        switch command.command {
        case "refresh": projectionRevision = nil
        case "importProfile":
            var args: [String: WireValue] = ["name": .string(payload["name"] ?? "新配置")]
            if let url = payload["url"] { args["url"] = .string(url) }
            else if let yaml = payload["yaml"] { args["yaml"] = .string(yaml) }
            else {
                let source = payload["source"] ?? ""
                let kind = source.hasPrefix("https://") || source.hasPrefix("http://") ? "url" : "yaml"
                args[kind] = .string(source)
            }
            _ = try await request("profile.import", args: args, id: command.requestID)
        case "selectProfile", "deleteProfile", "updateProfile", "renameProfile":
            let name = ["selectProfile": "profile.select", "deleteProfile": "profile.delete", "updateProfile": "profile.update", "renameProfile": "profile.rename"][command.command]!
            var args: [String: WireValue] = ["id": .string(payload["profileID"] ?? "")]
            if let yaml = payload["yaml"] { args["yaml"] = .string(yaml) }
            if let name = payload["name"] { args["name"] = .string(name) }
            _ = try await request(name, args: args, id: command.requestID)
        case "setMode": _ = try await request("settings.patch", args: ["mode": .string(payload["mode"] ?? "rule")], id: command.requestID)
        case "startCore": _ = try await request("runtime.start", id: command.requestID)
        case "stopCore": _ = try await request("runtime.stop", id: command.requestID)
        case "selectNode":
            guard let node = nodes.first(where: { $0.id == payload["nodeID"] }) else { throw invalid("节点已不存在") }
            _ = try await coreRequest("PUT", "/proxies/\(escaped(node.group))", body: .object(["name": .string(node.name)]), id: command.requestID)
            projectionRevision = nil
        case "testNode", "testAllNodes":
            let names: [String]
            if command.command == "testAllNodes" { names = Array(Set(nodes.map(\.name))).sorted() }
            else {
                guard let node = nodes.first(where: { $0.id == payload["nodeID"] }) else { throw invalid("节点已不存在") }
                names = [node.name]
            }
            let result = try await request("proxy.delay", args: ["names": .array(names.map(WireValue.string)), "url": .string(payload["url"] ?? "https://www.gstatic.com/generate_204")], id: command.requestID)
            for value in result.items { applyDelay(value) }
        case "closeConnection": _ = try await coreRequest("DELETE", "/connections/\(escaped(payload["connectionID"] ?? ""))", id: command.requestID)
        case "clearLogs": logs.removeAll()
        case "checkMedia":
            let result = try await request("media.check", args: ["service": .string(payload["service"] ?? "")], id: command.requestID)
            if let item = unlockRecord(result), let index = unlock.firstIndex(where: { $0.id == item.id }) { unlock[index] = item }
        case "checkAllMedia":
            let result = try await request("media.checkAll", id: command.requestID)
            for value in result.items { applyUnlock(value) }
        case "createBackup": _ = try await request("backup.create", id: command.requestID); try await loadBackups()
        case "listBackups": try await loadBackups()
        case "restoreBackup": _ = try await request("backup.restore", args: ["id": .string(payload["backupID"] ?? "")], id: command.requestID)
        case "setOverrides": _ = try await request("overrides.set", args: ["yaml": .string(payload["yaml"] ?? "{}")], id: command.requestID)
        case "updateProvider":
            guard let provider = providers.first(where: { $0.id == payload["providerID"] }), provider.vehicle.lowercased() != "file" else {
                throw invalid("该规则源或节点源无法远程更新")
            }
            _ = try await coreRequest("PUT", "/providers/\(provider.category)/\(escaped(provider.name))", id: command.requestID)
            projectionRevision = nil
        default: throw invalid("此操作尚未实现：\(command.command)")
        }
        failure = nil
        return try await snapshot()
    }

    func shutdown() async {
        stopped = true
        let owned = library
        if let owned { owned.stop(owned.handle) }
        while !pending.isEmpty { try? await Task.sleep(for: .milliseconds(25)) }
        library = nil
        // Keep the library loaded until every submitted operation has unwound.
        _ = owned
        results.removeAll()
    }

    private func request(_ command: String, args: [String: WireValue] = [:], id: String = UUID().uuidString) async throws -> WireValue {
        let library = try open()
        let expectedGeneration = generation
        let expectedRevision = revision
        let isRead = ["proxy.delay", "media.check", "media.checkAll"].contains(command)
            || (command == "core.request" && args["method"] == .string("GET"))
        let request = WireValue.object(["version": .number(1), "id": .string(id), "command": .string(command), "args": .object(args), "coreGeneration": .number(Double(generation))])
        let json = String(decoding: try JSONEncoder().encode(request), as: UTF8.self)
        let status = json.withCString { library.submit(library.handle, $0) }
        guard status == 0 else { throw ClientFailure(code: "submit_\(status)", message: "操作未能进入任务队列", detail: "Queue full, duplicate request, or stopped runtime", retryable: true) }
        pending.insert(id)
        defer { pending.remove(id); results.removeValue(forKey: id) }
        do {
            while true {
                guard !stopped else { throw CancellationError() }
                try drain()
                if let result = results.removeValue(forKey: id) {
                    if isRead && (result.coreGeneration != expectedGeneration || generation != expectedGeneration || result.revision != expectedRevision || revision != expectedRevision) {
                        throw ClientFailure(code: "stale_result", message: "配置已变化，已丢弃过期结果", detail: command, retryable: true)
                    }
                    if !result.ok {
                        let error = result.error
                        throw ClientFailure(code: error?.code ?? "operation_failed", message: "操作失败", detail: error?.message ?? command, retryable: true)
                    }
                    return result.data ?? .null
                }
                try Task.checkCancellation()
                try await Task.sleep(for: .milliseconds(25))
            }
        } catch {
            id.withCString { _ = library.cancel(library.handle, $0) }
            throw error
        }
    }

    private func coreRequest(_ method: String, _ path: String, body: WireValue? = nil, id: String = UUID().uuidString) async throws -> WireValue {
        var args: [String: WireValue] = ["method": .string(method), "path": .string(path)]
        if let body { args["body"] = body }
        return try await request("core.request", args: args, id: id)
    }

    private func drain() throws {
        guard let library else { return }
        while let event = try library.read(library.nextEvent(library.handle)) {
            if event.kind == "result" { if pending.contains(event.id) { results[event.id] = event }; continue }
            if event.kind == "state", let data = event.data, event.ok { acceptState(event, data: data); continue }
            guard event.coreGeneration == generation else { continue }
            if !event.ok { failure = ClientFailure(code: event.error?.code ?? "stream_failed", message: "状态流暂不可用", detail: event.error?.message ?? "", retryable: true); continue }
            if event.kind == "stream", let data = event.data { acceptStream(data, timestamp: event.timestamp ?? Date().timeIntervalSince1970) }
        }
    }

    private func acceptState(_ event: NativeEnvelope, data: WireValue) {
        guard event.coreGeneration > generation || (event.coreGeneration == generation && event.revision >= revision) else { return }
        if generation != event.coreGeneration {
            nodes.removeAll(); rules.removeAll(); providers.removeAll(); connections.removeAll(); traffic.removeAll()
            selectedNodeID = nil; subscribedGeneration = nil; projectionGeneration = nil
        }
        generation = event.coreGeneration; revision = event.revision; state = data
        if let error = data["runtime"]["error"].text { failure = ClientFailure(code: "core_failed", message: "内核需要修复", detail: error, retryable: true) }
    }

    private func acceptStream(_ data: WireValue, timestamp: Double) {
        let value = data["value"]
        switch data["stream"].text {
        case "traffic":
            traffic.append(TrafficSample(timestamp: timestamp, uploadBytesPerSecond: value["up"].number, downloadBytesPerSecond: value["down"].number))
            traffic.removeAll { $0.timestamp < timestamp - 300 }
            if traffic.count > 301 { traffic.removeFirst(traffic.count - 301) }
        case "logs":
            logs.append(LogRecord(id: UUID().uuidString, timestamp: timestamp, level: value["type"].text ?? "info", message: value["payload"].text ?? ""))
            if logs.count > 1000 { logs.removeFirst(logs.count - 1000) }
        case "delay": applyDelay(value)
        case "media": applyUnlock(value)
        case "connections": connections = value["connections"].items.compactMap(connectionRecord)
        default: break
        }
    }

    private func reloadProjection() async throws {
        let expectedGeneration = generation
        let expectedRevision = revision
        let proxies = try await coreRequest("GET", "/proxies")
        let ruleData = try await coreRequest("GET", "/rules")
        let ruleProviders = try await coreRequest("GET", "/providers/rules")
        let proxyProviders = try await coreRequest("GET", "/providers/proxies")
        guard generation == expectedGeneration, revision == expectedRevision else { return }
        var next: [NodeRecord] = []
        var chosen: String?
        let records = proxies["proxies"].fields
        for (group, record) in records.sorted(by: { $0.key < $1.key }) where record["type"].text == "Selector" {
            for entry in record["all"].items {
                guard let name = entry.text else { continue }
                let value = records[name] ?? .null
                let id = group + "\u{001F}" + name
                let last = value["history"].items.last?["delay"].number
                let delay = last.flatMap { $0 > 0 ? Int($0) : nil }
                next.append(NodeRecord(id: id, name: name, kind: value["type"].text ?? "Unknown", group: group, latencyMS: delay, available: value["alive"] == .bool(false) ? false : true))
                if record["now"].text == name && (chosen == nil || group == "GLOBAL") { chosen = id }
            }
        }
        for index in next.indices {
            if let prior = nodes.first(where: { $0.id == next[index].id }) {
                next[index].testedAt = prior.testedAt
                next[index].testURL = prior.testURL
                next[index].testRequestID = prior.testRequestID
            }
        }
        nodes = next; selectedNodeID = chosen
        rules = ruleData["rules"].items.enumerated().map { index, value in
            RuleRecord(id: String(index), kind: value["type"].text ?? "", value: value["payload"].text ?? "", target: value["proxy"].text ?? "")
        }
        providers = [("rules", ruleProviders), ("proxies", proxyProviders)].flatMap { category, response in
            response["providers"].fields.sorted(by: { $0.key < $1.key }).map { name, value in
                ProviderRecord(name: name, category: category, vehicle: value["vehicleType"].text ?? "Unknown",
                               behavior: value["behavior"].text ?? value["type"].text ?? "",
                               count: Int(value["ruleCount"].number ?? Double(value["proxies"].items.count)),
                               updatedAt: value["updatedAt"].text)
            }
        }
        projectionGeneration = generation; projectionRevision = revision; lastProjection = Date()
    }

    private func applyDelay(_ value: WireValue) {
        let name = value["name"].text
        let delay = value["delay"].number.flatMap { $0 > 0 ? Int($0) : nil }
        for index in nodes.indices where nodes[index].name == name {
            nodes[index].latencyMS = delay
            nodes[index].available = value["error"].text == nil
            nodes[index].testedAt = value["testedAt"].number
            nodes[index].testURL = value["url"].text
            nodes[index].testRequestID = value["requestId"].text
        }
    }
    private func applyUnlock(_ value: WireValue) {
        guard let item = unlockRecord(value), let index = unlock.firstIndex(where: { $0.id == item.id }) else { return }
        unlock[index] = item
    }
    private func loadBackups() async throws {
        backups = try await request("backup.list").items.compactMap { value in
            guard let id = value["id"].text else { return nil }
            return BackupRecord(id: id, createdAt: value["createdAt"].number ?? 0, bytes: UInt64(value["bytes"].number ?? 0))
        }
    }
    private func projectedSnapshot() -> CoreSnapshot {
        sequence &+= 1
        let selected = state["selectedProfileId"].text
        let profiles = state["profiles"].items.compactMap { value -> ProfileRecord? in
            guard let id = value["id"].text else { return nil }
            return ProfileRecord(id: id, name: value["name"].text ?? "", source: value["url"].text ?? "本地 YAML", active: id == selected, updatedAt: value["updatedAt"].number)
        }
        var result = CoreSnapshot(generation: generation, sequence: sequence, mode: state["settings"]["mode"].text ?? "rule", coreStatus: state["runtime"]["status"].text ?? "stopped", selectedNodeID: selectedNodeID, nodes: nodes, profiles: profiles, rules: rules, connections: connections, logs: logs, traffic: traffic, error: failure)
        result.unlock = unlock; result.backups = backups; result.overrides = state["overrides"].text ?? "{}"
        result.providers = providers
        result.profileDocuments = Dictionary(uniqueKeysWithValues: state["profiles"].items.compactMap { value in
            guard let id = value["id"].text, let yaml = value["yaml"].text else { return nil }; return (id, yaml)
        })
        return result
    }
    private func unlockRecord(_ value: WireValue) -> UnlockRecord? {
        guard let name = value["name"].text else { return nil }
        return UnlockRecord(id: name, status: value["status"].text ?? "Pending", region: value["region"].text, checkedAt: value["check_time"].text)
    }
    private func connectionRecord(_ value: WireValue) -> ConnectionRecord? {
        guard let id = value["id"].text else { return nil }
        let metadata = value["metadata"]
        let host = metadata["host"].text ?? metadata["destinationIP"].text ?? ""
        return ConnectionRecord(id: id, host: host, destination: "\(metadata["destinationIP"].text ?? ""):\(metadata["destinationPort"].text ?? String(Int(metadata["destinationPort"].number ?? 0)))", rule: value["rule"].text ?? "", node: value["chains"].items.compactMap(\.text).joined(separator: " → "), uploadBytes: UInt64(value["upload"].number ?? 0), downloadBytes: UInt64(value["download"].number ?? 0), source: "\(metadata["sourceIP"].text ?? ""):\(metadata["sourcePort"].text ?? "")", network: metadata["network"].text, process: metadata["processPath"].text ?? metadata["process"].text, startedAt: value["start"].text, rulePayload: value["rulePayload"].text)
    }
    private func escaped(_ text: String) -> String {
        text.addingPercentEncoding(withAllowedCharacters: CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~")) ?? ""
    }
    private func invalid(_ message: String) -> ClientFailure { ClientFailure(code: "invalid_operation", message: message, detail: message, retryable: false) }
}
