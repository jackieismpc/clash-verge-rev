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

@Test func rustBridgeRunsOwnedMihomoAndRestoresNativeBackup() async throws {
    let environment = ProcessInfo.processInfo.environment
    let library = try #require(environment["CVN_LIBRARY_PATH"])
    let mihomo = try #require(environment["CVN_MIHOMO_PATH"])
    let root = try #require(environment["CVN_TEST_ROOT"])
    let directory = URL(fileURLWithPath: root).appendingPathComponent("i-" + UUID().uuidString.prefix(8))
    defer { try? FileManager.default.removeItem(at: directory) }
    let transport = NativeCoreTransport(options: NativeCoreOptions(libraryPath: library, dataDirectory: directory.path, mihomoPath: mihomo, mixedPort: try unusedPort()))
    do {
        let initial = try await transport.snapshot()
        #expect(initial.coreStatus == "stopped")
        #expect(initial.profiles.isEmpty)
        let yaml = """
        proxies: []
        proxy-groups:
          - name: "测试 分组"
            type: select
            proxies: [DIRECT, REJECT]
        rules:
          - "MATCH,测试 分组"
        """
        let imported = try await transport.execute(CommandEnvelope(command: "importProfile", payload: ["name": "隔离测试", "yaml": yaml]))
        let profile = try #require(imported.profiles.first)
        let started = try await transport.execute(CommandEnvelope(command: "startCore"))
        #expect(started.coreStatus == "running")
        #expect(started.generation > initial.generation)
        let direct = try #require(started.nodes.first { $0.group == "测试 分组" && $0.name == "DIRECT" })
        #expect(started.rules.contains { $0.target == "测试 分组" })
        _ = try await transport.execute(CommandEnvelope(command: "selectNode", payload: ["nodeID": direct.id]))
        let saved = try await transport.execute(CommandEnvelope(command: "createBackup"))
        let backup = try #require(saved.backups.first)
        let changed = try await transport.execute(CommandEnvelope(command: "setMode", payload: ["mode": "direct"]))
        #expect(changed.mode == "direct")
        let restored = try await transport.execute(CommandEnvelope(command: "restoreBackup", payload: ["backupID": backup.id]))
        #expect(restored.mode == "rule")
        let stopped = try await transport.execute(CommandEnvelope(command: "stopCore"))
        #expect(stopped.coreStatus == "stopped")
        #expect(stopped.connections.isEmpty)
        let renamed = try await transport.execute(CommandEnvelope(command: "renameProfile", payload: ["profileID": profile.id, "name": "已保存"] ))
        #expect(renamed.profiles.first?.name == "已保存")
        await transport.shutdown()
        let reopened = NativeCoreTransport(options: NativeCoreOptions(libraryPath: library, dataDirectory: directory.path, mihomoPath: mihomo, mixedPort: try unusedPort()))
        #expect(try await reopened.snapshot().profiles.first?.name == "已保存")
        await reopened.shutdown()
    } catch {
        await transport.shutdown()
        throw error
    }
}
