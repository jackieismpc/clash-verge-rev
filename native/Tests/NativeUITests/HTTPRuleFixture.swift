import Foundation
import Network

final class HTTPRuleFixture: @unchecked Sendable {
    private let listener: NWListener
    private let queue = DispatchQueue(label: "native-test.rules")
    private let lock = NSLock()
    private var ready = false
    private var requests = 0
    private let responseDelay: TimeInterval
    var requestCount: Int { lock.withLock { requests } }
    var url: String { "http://127.0.0.1:\(listener.port!.rawValue)/rules.yaml" }

    private init(responseDelay: TimeInterval) throws {
        self.responseDelay = responseDelay
        let parameters = NWParameters.tcp
        parameters.requiredLocalEndpoint = .hostPort(host: .ipv4(.loopback), port: .any)
        listener = try NWListener(using: parameters)
    }

    static func start(responseDelay: TimeInterval = 0) async throws -> HTTPRuleFixture {
        let fixture = try HTTPRuleFixture(responseDelay: responseDelay)
        try await fixture.startListening()
        return fixture
    }

    private func startListening() async throws {
        listener.newConnectionHandler = { [weak self] connection in
            guard let self else { connection.cancel(); return }
            connection.start(queue: self.queue)
            connection.receive(minimumIncompleteLength: 1, maximumLength: 4096) { [weak self] request, _, _, error in
                guard error == nil, let self else { connection.cancel(); return }
                self.lock.withLock { self.requests += 1 }
                let body = "payload:\n  - DOMAIN-SUFFIX,example.net\n  - DOMAIN-SUFFIX,example.org\n"
                let head = request.map { String(decoding: $0, as: UTF8.self).hasPrefix("HEAD ") } ?? false
                let response = "HTTP/1.1 200 OK\r\nContent-Type: application/yaml\r\nContent-Length: \(body.utf8.count)\r\nConnection: close\r\n\r\n\(head ? "" : body)"
                self.queue.asyncAfter(deadline: .now() + self.responseDelay) {
                    connection.send(content: Data(response.utf8), completion: .contentProcessed { _ in connection.cancel() })
                }
            }
        }
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, any Error>) in
            listener.stateUpdateHandler = { [weak self] state in
                guard let self else { return }
                let outcome: Result<Void, any Error>
                switch state {
                case .ready: outcome = .success(())
                case .failed(let error): outcome = .failure(error)
                default: return
                }
                let first = self.lock.withLock { if self.ready { return false }; self.ready = true; return true }
                if first { continuation.resume(with: outcome) }
            }
            listener.start(queue: queue)
        }
    }

    func stop() { listener.cancel() }
    deinit { listener.cancel() }
}
