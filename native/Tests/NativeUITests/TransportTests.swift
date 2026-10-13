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
