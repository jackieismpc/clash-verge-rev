import Foundation
import Testing
import Darwin
@testable import NativeUI

private final class ShadowsocksFixture {
    let port: UInt16
    let home: URL
    private let process = Process()
    private let lifetime = Pipe()
    var running: Bool { process.isRunning }

    init(home: URL, engine: String, host: String, key: String) throws {
        self.home = home
        self.port = try unusedPort()
        try FileManager.default.createDirectory(at: home.appendingPathComponent("tmp"), withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let config = home.appendingPathComponent("runtime.yaml")
        let yaml = """
        mode: direct
        allow-lan: false
        mixed-port: 0
        external-controller-unix: \(home.appendingPathComponent("c.sock").path)
        tun: {enable: false}
        dns: {enable: false}
        ntp: {enable: false}
        listeners:
          - {name: synthetic-ss, type: shadowsocks, listen: 127.0.0.1, port: \(port), cipher: 2022-blake3-aes-128-gcm, password: "\(key)", udp: false}
        rules: ['MATCH,DIRECT']
        """
        try yaml.write(to: config, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: config.path)
        let specification = try JSONSerialization.data(withJSONObject: ["binary": engine, "home": home.path, "config": config.path, "integration": true])
        process.executableURL = URL(fileURLWithPath: host)
        process.arguments = ["--supervise", String(decoding: specification, as: UTF8.self)]
        process.environment = [:]
        process.standardInput = lifetime
        let log = home.appendingPathComponent("core.log")
        guard FileManager.default.createFile(atPath: log.path, contents: nil, attributes: [.posixPermissions: 0o600]) else { throw POSIXError(.EIO) }
        let output = try FileHandle(forWritingTo: log)
        defer { try? output.close() }
        process.standardOutput = output
        process.standardError = output
        try process.run()
        // Process inherits the read descriptor; this parent keeps only the lifetime writer.
        try lifetime.fileHandleForReading.close()
    }

    func waitUntilReady() async throws {
        let deadline = ContinuousClock.now.advanced(by: .seconds(8))
        while process.isRunning && ContinuousClock.now < deadline {
            if acceptsConnection() { return }
            try await Task.sleep(for: .milliseconds(25))
        }
        throw ClientFailure(code: "fixture_unavailable", message: "Synthetic SS listener did not start", detail: "Inspect the owned fixture log", retryable: false)
    }

    private func acceptsConnection() -> Bool {
        let descriptor = socket(AF_INET, SOCK_STREAM, 0)
        guard descriptor >= 0 else { return false }
        defer { close(descriptor) }
        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_addr.s_addr = inet_addr("127.0.0.1")
        address.sin_port = port.bigEndian
        return withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { connect(descriptor, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) == 0 }
        }
    }

    func stop() throws {
        try? lifetime.fileHandleForWriting.close()
        let deadline = ContinuousClock.now.advanced(by: .seconds(8))
        while process.isRunning && ContinuousClock.now < deadline { Thread.sleep(forTimeInterval: 0.025) }
        guard !process.isRunning else { throw POSIXError(.ETIMEDOUT) }
        process.waitUntilExit()
    }

    deinit { try? lifetime.fileHandleForWriting.close() }
}

@Test func localShadowsocks2022ForwardsOnlyWithCorrectKeyAndKeepsFailureDetails() async throws {
    let env = ProcessInfo.processInfo.environment
    let root = try #require(env["CVN_TEST_ROOT"])
    let directory = URL(fileURLWithPath: root).appendingPathComponent("ss-" + UUID().uuidString.prefix(8))
    let key = Data(repeating: 1, count: 16).base64EncodedString()
    let wrongKey = Data(repeating: 2, count: 16).base64EncodedString()
    let server = try ShadowsocksFixture(home: directory.appendingPathComponent("server"), engine: try #require(env["CVN_MIHOMO_PATH"]), host: try #require(env["CVN_HOST_PATH"]), key: key)
    defer {
        try? server.stop()
        if !server.running { try? FileManager.default.removeItem(at: directory) }
    }
    // The pinned core treats sub-millisecond delay as a zero result, so this target responds after 10 ms.
    let target = try await HTTPRuleFixture.start(responseDelay: 0.01)
    defer { target.stop() }
    let port = try unusedPort()
    let transport = NativeCoreTransport(options: NativeCoreOptions(libraryPath: try #require(env["CVN_LIBRARY_PATH"]), dataDirectory: directory.appendingPathComponent("client").path, mihomoPath: env["CVN_MIHOMO_PATH"], hostPath: env["CVN_HOST_PATH"], mixedPort: port, executionMode: .integrationTest, integrationRoot: root))
    do {
        try await server.waitUntilReady()
        let yaml = """
        proxies:
          - {name: synthetic-good, type: ss, server: 127.0.0.1, port: \(server.port), cipher: 2022-blake3-aes-128-gcm, password: "\(key)"}
          - {name: synthetic-bad, type: ss, server: 127.0.0.1, port: \(server.port), cipher: 2022-blake3-aes-128-gcm, password: "\(wrongKey)"}
        proxy-groups:
          - {name: Synthetic-SS, type: select, proxies: [synthetic-good, synthetic-bad]}
        rules: ['MATCH,Synthetic-SS']
        """
        _ = try await transport.execute(CommandEnvelope(command: "importProfile", payload: ["name": "Synthetic SS", "yaml": yaml]))
        let started = try await transport.execute(CommandEnvelope(command: "startCore"))
        let good = try #require(started.nodes.first { $0.group == "Synthetic-SS" && $0.name == "synthetic-good" })
        let bad = try #require(started.nodes.first { $0.group == "Synthetic-SS" && $0.name == "synthetic-bad" })
        _ = try await transport.execute(CommandEnvelope(command: "selectNode", payload: ["nodeID": good.id]))
        #expect(try forwardThroughOwnedProxy(port, target.url) == 0)
        #expect(target.requestCount == 1)
        let passed = try await transport.execute(CommandEnvelope(command: "testNode", payload: ["nodeID": good.id, "url": target.url]))
        #expect(passed.nodes.first { $0.id == good.id }?.latencyMS != nil)
        #expect(passed.nodes.first { $0.id == good.id }?.testError == nil)
        _ = try await transport.execute(CommandEnvelope(command: "selectNode", payload: ["nodeID": bad.id]))
        let prior = target.requestCount
        #expect(try forwardThroughOwnedProxy(port, target.url) != 0)
        #expect(target.requestCount == prior)
        let failed = try await transport.execute(CommandEnvelope(command: "testNode", payload: ["nodeID": bad.id, "url": target.url]))
        #expect(failed.nodes.first { $0.id == bad.id }?.latencyMS == nil)
        #expect(failed.nodes.first { $0.id == bad.id }?.testError?.isEmpty == false)
        #expect(failed.nodes.first { $0.id == bad.id }?.statusTitle == "测速失败")
        let refreshed = try await transport.execute(CommandEnvelope(command: "refresh"))
        #expect(refreshed.nodes.first { $0.id == bad.id }?.testError == failed.nodes.first { $0.id == bad.id }?.testError)
        _ = try await transport.execute(CommandEnvelope(command: "selectNode", payload: ["nodeID": bad.id]))
        let profile = try #require(started.profiles.first)
        _ = try await transport.execute(CommandEnvelope(command: "updateProfile", payload: ["profileID": profile.id, "yaml": yaml.replacingOccurrences(of: wrongKey, with: key)]))
        let recovered = try await transport.execute(CommandEnvelope(command: "testNode", payload: ["nodeID": bad.id, "url": target.url]))
        #expect(recovered.nodes.first { $0.id == bad.id }?.latencyMS != nil)
        #expect(recovered.nodes.first { $0.id == bad.id }?.testError == nil)
        #expect(recovered.nodes.first { $0.id == bad.id }?.available == true)
        #expect(recovered.nodes.first { $0.id == bad.id }?.statusTitle == "可用")
        for index in 0..<8 {
            async let refreshing = transport.execute(CommandEnvelope(command: "refresh"))
            try await Task.sleep(for: .milliseconds(10))
            let mode = index.isMultiple(of: 2) ? "direct" : "rule"
            _ = try await transport.execute(CommandEnvelope(command: "setMode", payload: ["mode": mode]))
            #expect(try await refreshing.error == nil)
            #expect(try await transport.snapshot().mode == mode)
        }
        await transport.shutdown()
        try server.stop()
        #expect(!FileManager.default.fileExists(atPath: server.home.appendingPathComponent("c.sock").path))
        #expect(!FileManager.default.fileExists(atPath: directory.appendingPathComponent("client/c.sock").path))
        let ports = [port, server.port]
        for released in ports {
            let descriptor = socket(AF_INET, SOCK_STREAM, 0)
            defer { close(descriptor) }
            var reuse: Int32 = 1
            #expect(setsockopt(descriptor, SOL_SOCKET, SO_REUSEADDR, &reuse, socklen_t(MemoryLayout<Int32>.size)) == 0)
            var address = sockaddr_in()
            address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
            address.sin_family = sa_family_t(AF_INET)
            address.sin_addr.s_addr = inet_addr("127.0.0.1")
            address.sin_port = released.bigEndian
            #expect(withUnsafePointer(to: &address) { pointer in pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(descriptor, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) == 0 } })
        }
    } catch {
        await transport.shutdown()
        try server.stop()
        throw error
    }
}
