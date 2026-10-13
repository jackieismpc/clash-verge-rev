import Foundation
import Testing
@testable import NativeUI

@Test func snapshotOrderingProtectsNewerGeneration() async throws {
    let demo = DemoTransport()
    let first = try await demo.snapshot()
    var older = first
    older.generation = 0
    older.sequence = UInt64.max
    #expect(!SnapshotOrdering.accepts(older, after: first))
    var newer = first
    newer.generation += 1
    newer.sequence = 0
    #expect(SnapshotOrdering.accepts(newer, after: first))
    var oldSequence = first
    oldSequence.sequence -= 1
    #expect(!SnapshotOrdering.accepts(oldSequence, after: first))
    var unsupported = first
    unsupported.schemaVersion = 2
    #expect(!SnapshotOrdering.accepts(unsupported, after: nil))
}

@Test func trafficSegmentsPreserveMissingSamplesAndTimeGaps() {
    let samples = [
        TrafficSample(timestamp: 1, uploadBytesPerSecond: 100, downloadBytesPerSecond: 200),
        TrafficSample(timestamp: 2, uploadBytesPerSecond: nil, downloadBytesPerSecond: 210),
        TrafficSample(timestamp: 3, uploadBytesPerSecond: 110, downloadBytesPerSecond: 220),
        TrafficSample(timestamp: 8, uploadBytesPerSecond: 120, downloadBytesPerSecond: 230),
    ]
    let points = chartPoints(samples, uploadOnly: true)
    #expect(points.count == 3)
    #expect(Set(points.map(\.series)).count == 3)
    #expect(chartPoints(samples, uploadOnly: false).count == 4)
}

@Test func demoRejectsUnavailableNodesAndStaleCommands() async throws {
    let demo = DemoTransport()
    let first = try await demo.snapshot()
    await #expect(throws: ClientFailure.self) {
        try await demo.execute(CommandEnvelope(expectedGeneration: 99, command: "selectNode", payload: ["nodeID": "jp-01"]))
    }
    await #expect(throws: ClientFailure.self) {
        try await demo.execute(CommandEnvelope(expectedGeneration: first.generation, command: "selectNode", payload: ["nodeID": "offline"]))
    }
    let selected = try await demo.execute(CommandEnvelope(expectedGeneration: first.generation, command: "selectNode", payload: ["nodeID": "jp-01"]))
    #expect(selected.selectedNodeID == "jp-01")
    #expect(selected.coreStatus == "demo")
    #expect(demo.isDemo)
}

@Test func demoSnapshotRoundTripsJSON() async throws {
    let state = try await DemoTransport().snapshot()
    let data = try JSONEncoder().encode(state)
    let decoded = try JSONDecoder().decode(CoreSnapshot.self, from: data)
    #expect(decoded.traffic.count == 300)
    #expect(decoded.nodes.count == state.nodes.count)
    #expect(decoded.generation == state.generation)
}

@Test func logFiltersCombineLevelAndSearch() {
    let records = [LogRecord(id: "1", timestamp: 0, level: "error", message: "DNS timeout"),
                   LogRecord(id: "2", timestamp: 0, level: "info", message: "DNS ready")]
    #expect(LogPresentation.filter(records, level: "error", query: "dns").map(\.id) == ["1"])
    #expect(LogPresentation.filter(records, level: "all", query: "ready").map(\.id) == ["2"])
    #expect(LogPresentation.filter(records, level: "warning", query: "").isEmpty)
}

@Test func demoNetworkStatesAndProviderUpdatesHaveNoCoreSideEffects() async throws {
    for scenario in [DemoScenario.networkApplying, .networkFailed, .networkOn] {
        let demo = DemoTransport(scenario: scenario)
        let state = try await demo.snapshot()
        #expect(state.coreStatus == "demo")
        #expect(state.networkFeatures.systemProxy != .unavailable)
        #expect(demo.isDemo)
    }
    let demo = DemoTransport()
    let initial = try await demo.snapshot()
    let provider = try #require(initial.providers.first)
    let updated = try await demo.execute(CommandEnvelope(command: "updateProvider", payload: ["providerID": provider.id]))
    #expect(updated.providers.first?.updatedAt != nil)
    #expect(updated.coreStatus == "demo")
    let empty = try await DemoTransport(scenario: .empty).snapshot()
    #expect(empty.nodes.isEmpty && empty.providers.isEmpty)
}

@Test @MainActor func latencyProgressIgnoresEarlierBatchAndFinishesCurrentRequest() async throws {
    let demo = DemoTransport()
    _ = try await demo.execute(CommandEnvelope(requestID: "earlier", command: "testNode", payload: ["nodeID": "hk-01"]))
    let model = AppModel(transport: demo)
    await model.refresh()
    model.testNodes(nodeID: "hk-01")
    #expect(model.testCompleted == 0)
    #expect(model.testTotal == 1)
    let deadline = Date().addingTimeInterval(3)
    while model.testingNodes && Date() < deadline {
        try await Task.sleep(for: .milliseconds(20))
        await model.refresh()
    }
    #expect(!model.testingNodes)
    #expect(model.testCompleted == 1)
    #expect(model.snapshot?.nodes.first { $0.id == "hk-01" }?.testRequestID != "earlier")
    await model.shutdown()
}
