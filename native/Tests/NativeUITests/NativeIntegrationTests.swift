import Foundation
import Testing
import Darwin
@testable import NativeUI

private func unusedPort() throws -> UInt16 {
    let descriptor = socket(AF_INET, SOCK_STREAM, 0)
    guard descriptor >= 0 else { throw POSIXError(.EIO) }
    defer { close(descriptor) }
    var address = sockaddr_in()
    address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
    address.sin_family = sa_family_t(AF_INET)
    address.sin_addr.s_addr = inet_addr("127.0.0.1")
    let bound = withUnsafePointer(to: &address) { pointer in
        pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(descriptor, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) }
    }
    guard bound == 0 else { throw POSIXError(.EADDRINUSE) }
    var length = socklen_t(MemoryLayout<sockaddr_in>.size)
    let read = withUnsafeMutablePointer(to: &address) { pointer in
        pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { getsockname(descriptor, $0, &length) }
    }
    guard read == 0 else { throw POSIXError(.EIO) }
    return UInt16(bigEndian: address.sin_port)
}

@Test func nativeURIImportValidatesWithOwnedMihomoAndRejectsScripts() async throws {
    let environment = ProcessInfo.processInfo.environment
    let library = try #require(environment["CVN_LIBRARY_PATH"])
    let mihomo = try #require(environment["CVN_MIHOMO_PATH"])
    let root = try #require(environment["CVN_TEST_ROOT"])
    let directory = URL(fileURLWithPath: root).appendingPathComponent("u-" + UUID().uuidString.prefix(8))
    defer { try? FileManager.default.removeItem(at: directory) }
    let transport = NativeCoreTransport(options: NativeCoreOptions(libraryPath: library, dataDirectory: directory.path, mihomoPath: mihomo, mixedPort: try unusedPort(), executionMode: .integrationTest, integrationRoot: root))
    do {
        let imported = try await transport.execute(CommandEnvelope(command: "importProfile", payload: ["name": "链接样本", "uri": "ss://YWVzLTEyOC1nY206ZXhhbXBsZQ@example.invalid:443#Sample"]))
        #expect(imported.profiles.count == 1)
        let started = try await transport.execute(CommandEnvelope(command: "startCore"))
        #expect(started.coreStatus == "running")
        #expect(started.nodes.contains { $0.group == "Imported" && $0.name == "Sample" })
        #expect(started.rules.contains { $0.target == "Imported" })
        _ = try await transport.execute(CommandEnvelope(command: "stopCore"))
        await #expect(throws: ClientFailure.self) {
            try await transport.execute(CommandEnvelope(command: "importProfile", payload: ["filename": "legacy.js", "yaml": "function main(config) { return config; }"]))
        }
        #expect(try await transport.snapshot().profiles.count == 1)
        await transport.shutdown()
    } catch {
        await transport.shutdown()
        throw error
    }
}

@Test func rustBridgeRunsOwnedMihomoAndRestoresNativeBackup() async throws {
    let environment = ProcessInfo.processInfo.environment
    let library = try #require(environment["CVN_LIBRARY_PATH"])
    let mihomo = try #require(environment["CVN_MIHOMO_PATH"])
    let root = try #require(environment["CVN_TEST_ROOT"])
    let directory = URL(fileURLWithPath: root).appendingPathComponent("i-" + UUID().uuidString.prefix(8))
    defer { try? FileManager.default.removeItem(at: directory) }
    let transport = NativeCoreTransport(options: NativeCoreOptions(libraryPath: library, dataDirectory: directory.path, mihomoPath: mihomo, mixedPort: try unusedPort(), executionMode: .integrationTest, integrationRoot: root))
    let providerServer = try await HTTPRuleFixture.start()
    defer { providerServer.stop() }
    do {
        let initial = try await transport.snapshot()
        #expect(initial.coreStatus == "stopped")
        #expect(initial.profiles.isEmpty)
        try "payload:\n  - DOMAIN-SUFFIX,example.com\n".write(to: directory.appendingPathComponent("local-rules.yaml"), atomically: true, encoding: .utf8)
        let yaml = """
        proxies: []
        rule-providers:
          "规则 源":
            type: file
            behavior: classical
            format: yaml
            path: ./local-rules.yaml
          "远程 规则源":
            type: http
            behavior: classical
            format: yaml
            path: ./remote-rules.yaml
            url: \(providerServer.url)
            interval: 3600
        proxy-groups:
          - name: "测试 分组"
            type: select
            proxies: [DIRECT, REJECT]
        rules:
          - "RULE-SET,规则 源,测试 分组"
          - "RULE-SET,远程 规则源,测试 分组"
          - "MATCH,测试 分组"
        """
        let imported = try await transport.execute(CommandEnvelope(command: "importProfile", payload: ["name": "隔离测试", "yaml": yaml]))
        let profile = try #require(imported.profiles.first)
        let started = try await transport.execute(CommandEnvelope(command: "startCore"))
        #expect(started.coreStatus == "running")
        #expect(started.generation > initial.generation)
        for filename in ["state.json", "runtime.yaml", "core.log"] {
            let attributes = try FileManager.default.attributesOfItem(atPath: directory.appendingPathComponent(filename).path)
            #expect((attributes[.posixPermissions] as? NSNumber)?.intValue == 0o600)
        }
        #expect(transport.isIntegrationTest)
        let direct = try #require(started.nodes.first { $0.group == "测试 分组" && $0.name == "DIRECT" })
        #expect(started.rules.contains { $0.target == "测试 分组" })
        var providerState = started
        let providerDeadline = Date().addingTimeInterval(3)
        while providerState.providers.first(where: { $0.name == "规则 源" })?.count == 0 && Date() < providerDeadline {
            try await Task.sleep(for: .milliseconds(100))
            providerState = try await transport.execute(CommandEnvelope(command: "refresh"))
        }
        let ruleProvider = try #require(providerState.providers.first { $0.category == "rules" && $0.name == "规则 源" })
        #expect(ruleProvider.count == 1)
        await #expect(throws: ClientFailure.self) {
            try await transport.execute(CommandEnvelope(command: "updateProvider", payload: ["providerID": ruleProvider.id]))
        }
        let remote = try #require(providerState.providers.first { $0.name == "远程 规则源" })
        let priorRequests = providerServer.requestCount
        let refreshed = try await transport.execute(CommandEnvelope(command: "updateProvider", payload: ["providerID": remote.id]))
        #expect(refreshed.providers.first { $0.id == remote.id }?.count == 2)
        #expect(providerServer.requestCount > priorRequests)
        _ = try await transport.execute(CommandEnvelope(command: "selectNode", payload: ["nodeID": direct.id]))
        let covered = try await transport.execute(CommandEnvelope(command: "setRuleOverrides", payload: ["prepend": "DOMAIN,local.example,REJECT", "append": "", "delete": ""]))
        #expect(covered.ruleOverrides.prepend == ["DOMAIN,local.example,REJECT"])
        #expect(covered.rules.contains { $0.target == "REJECT" })
        await transport.setVisibility(false)
        _ = try await transport.snapshot()
        await transport.setVisibility(true)
        #expect(try await transport.snapshot().coreStatus == "running")
        let saved = try await transport.execute(CommandEnvelope(command: "createBackup"))
        let backup = try #require(saved.backups.first)
        let changed = try await transport.execute(CommandEnvelope(command: "setMode", payload: ["mode": "direct"]))
        #expect(changed.mode == "direct")
        let restored = try await transport.execute(CommandEnvelope(command: "restoreBackup", payload: ["backupID": backup.id]))
        #expect(restored.mode == "rule")
        #expect(restored.ruleOverrides.prepend == ["DOMAIN,local.example,REJECT"])
        let stopped = try await transport.execute(CommandEnvelope(command: "stopCore"))
        #expect(stopped.coreStatus == "stopped")
        #expect(stopped.connections.isEmpty)
        let renamed = try await transport.execute(CommandEnvelope(command: "renameProfile", payload: ["profileID": profile.id, "name": "已保存"] ))
        #expect(renamed.profiles.first?.name == "已保存")
        await transport.shutdown()
        let reopened = NativeCoreTransport(options: NativeCoreOptions(libraryPath: library, dataDirectory: directory.path, mihomoPath: mihomo, mixedPort: try unusedPort(), executionMode: .integrationTest, integrationRoot: root))
        #expect(try await reopened.snapshot().profiles.first?.name == "已保存")
        await reopened.shutdown()
    } catch {
        await transport.shutdown()
        throw error
    }
}
