import SwiftUI
import Charts

struct SurfaceCard<Content: View>: View {
    @Environment(\.colorScheme) private var scheme
    @Environment(\.colorSchemeContrast) private var contrast
    @ViewBuilder var content: Content
    var body: some View {
        content
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(scheme == .dark ? Color(nsColor: .controlBackgroundColor) : .white, in: RoundedRectangle(cornerRadius: 20))
            .overlay(RoundedRectangle(cornerRadius: 20).strokeBorder(Color.primary.opacity(contrast == .increased ? 0.45 : 0.06)))
            .shadow(color: .black.opacity(scheme == .dark ? 0.12 : 0.035), radius: 12, y: 4)
    }
}

struct OverviewView: View {
    @Bindable var model: AppModel
    var body: some View {
        GeometryReader { geometry in
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Text("概览").font(.system(size: 28, weight: .semibold))
                    HStack(alignment: .top, spacing: 24) {
                        summary("当前配置", model.snapshot?.profiles.first(where: \.active)?.name ?? "未选择")
                        summary("运行模式", modeName(model.snapshot?.mode))
                        summary("当前节点", model.selectedNode?.name ?? "未选择")
                    }.padding(.bottom, 8)
                    if geometry.size.width >= 900 {
                        HStack(alignment: .top, spacing: 16) {
                            latencyCard.frame(width: max(0, (geometry.size.width - 80) / 2))
                            rateCard("上传", upload: true)
                            rateCard("下载", upload: false)
                        }
                    } else if geometry.size.width >= 600 {
                        latencyCard
                        HStack(spacing: 16) { rateCard("上传", upload: true); rateCard("下载", upload: false) }
                    } else {
                        latencyCard
                        rateCard("上传", upload: true)
                        rateCard("下载", upload: false)
                    }
                    if geometry.size.width >= 760 {
                        HStack(alignment: .top, spacing: 16) {
                            connectionsCard.frame(width: max(0, (geometry.size.width - 64) * 0.38))
                            trafficCard
                        }
                    } else {
                        connectionsCard
                        trafficCard
                    }
                    if model.isDemo {
                        Label("此处为演示数据，交互仅用于验证界面；代理内核和系统网络均未启动。", systemImage: "info.circle")
                            .font(.system(size: 12)).foregroundStyle(.secondary)
                            .padding(.top, 4)
                    }
                }
                .padding(24)
            }
        }
    }

    private var latencyCard: some View {
        SurfaceCard {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    cardLabel("节点延迟", symbol: "timer")
                    Spacer()
                    Button(model.isDemo ? "演示测速" : "测速") { if let id = model.selectedNode?.id { model.testNodes(nodeID: id) } }
                        .disabled(model.busy || model.selectedNode == nil)
                }
                number(model.selectedNode?.latencyMS.map(String.init) ?? "尚未测试", unit: model.selectedNode?.latencyMS == nil ? "" : "ms")
                Spacer(minLength: 12)
                Text(model.selectedNode?.name ?? "暂无节点").font(.system(size: 14)).lineLimit(2)
                Text(model.isDemo ? "来源：演示节点测试样本" : model.selectedNode?.testURL ?? "尚未测试")
                    .font(.system(size: 12)).foregroundStyle(.secondary).lineLimit(1)
                if let date = model.selectedNode?.testedAt {
                    Text(Date(timeIntervalSince1970: date), style: .time).font(.system(size: 12)).foregroundStyle(.secondary)
                }
            }.frame(height: 184, alignment: .top)
        }
    }
    private var connectionsCard: some View {
        SurfaceCard {
            VStack(alignment: .leading, spacing: 18) {
                cardLabel("活动连接", symbol: "link")
                number(String(model.snapshot?.connections.count ?? 0), unit: "条")
                Text("仅显示内核当前报告的连接。").font(.system(size: 12)).foregroundStyle(.secondary)
                Divider()
                ForEach(Array((model.snapshot?.connections ?? []).prefix(3))) { connection in
                    HStack {
                        Text(connection.host).lineLimit(1).help(connection.host)
                        Spacer()
                        Text(connection.node).foregroundStyle(.secondary).lineLimit(1)
                    }.font(.system(size: 12))
                }
                Spacer(minLength: 0)
                Button("查看全部连接") { model.page = .activity }
            }.frame(height: 260, alignment: .top)
        }
    }
    private var trafficCard: some View {
        SurfaceCard {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    cardLabel("流量趋势", symbol: "chart.xyaxis.line")
                    Spacer()
                    Text("最近 5 分钟 · 1 Hz").font(.system(size: 12)).foregroundStyle(.secondary)
                }
                TrafficChart(samples: model.snapshot?.traffic ?? [], compact: false).frame(height: 194)
                HStack(spacing: 16) {
                    Label("上传", systemImage: "circle.fill").foregroundStyle(.indigo)
                    Label("下载", systemImage: "circle.fill").foregroundStyle(.teal)
                    Spacer()
                    Text("缺失采样保留断点")
                }.font(.system(size: 12)).foregroundStyle(.secondary)
            }.frame(height: 260)
        }
    }

    private func rateCard(_ title: String, upload: Bool) -> some View {
        let rate = upload ? model.snapshot?.traffic.last?.uploadBytesPerSecond : model.snapshot?.traffic.last?.downloadBytesPerSecond
        return SurfaceCard {
            VStack(alignment: .leading, spacing: 14) {
                cardLabel(title, symbol: upload ? "arrow.up" : "arrow.down")
                number(rate.map { String(format: "%.0f", $0 / 1024) } ?? "—", unit: "KB/s")
                Spacer(minLength: 0)
                TrafficChart(samples: model.snapshot?.traffic ?? [], compact: true, uploadOnly: upload)
                    .frame(height: 68)
            }.frame(height: 184, alignment: .top)
        }
    }

    private func summary(_ title: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title).font(.system(size: 12)).foregroundStyle(.secondary)
            Text(value).font(.system(size: 14, weight: .medium)).lineLimit(2).help(value)
        }.frame(maxWidth: .infinity, alignment: .leading)
    }
}

func cardLabel(_ title: String, symbol: String) -> some View {
    Label(title, systemImage: symbol).font(.system(size: 12, weight: .medium)).foregroundStyle(.secondary)
}

func number(_ value: String, unit: String) -> some View {
    HStack(alignment: .firstTextBaseline, spacing: 6) {
        Text(value).font(.system(size: 40, weight: .semibold, design: .default)).monospacedDigit()
            .minimumScaleFactor(0.7).lineLimit(1)
        Text(unit).font(.system(size: 14)).foregroundStyle(.secondary)
    }.accessibilityElement(children: .combine)
}

func modeName(_ mode: String?) -> String {
    switch mode { case "rule": "规则模式"; case "global": "全局模式"; case "direct": "直连模式"; default: "未知" }
}

struct ChartPoint: Identifiable {
    var timestamp: Double
    var rate: Double
    var series: String
    var upload: Bool
    var id: String { "\(timestamp)-\(upload)" }
}

func chartPoints(_ samples: [TrafficSample], uploadOnly: Bool? = nil) -> [ChartPoint] {
    var result: [ChartPoint] = []
    for upload in [true, false] where uploadOnly == nil || uploadOnly == upload {
        var segment = 0
        var previous: Double?
        for sample in samples {
            let rate = upload ? sample.uploadBytesPerSecond : sample.downloadBytesPerSecond
            if let previous, sample.timestamp - previous > 1.8 { segment += 1 }
            if let rate, rate.isFinite, rate >= 0 {
                result.append(ChartPoint(timestamp: sample.timestamp, rate: rate / 1024, series: "\(upload)-\(segment)", upload: upload))
            } else { segment += 1 }
            previous = sample.timestamp
        }
    }
    return result
}

struct TrafficChart: View {
    var samples: [TrafficSample]
    var compact: Bool
    var uploadOnly: Bool? = nil
    var body: some View {
        let end = samples.last?.timestamp ?? Date().timeIntervalSince1970
        Chart(chartPoints(samples, uploadOnly: uploadOnly)) { point in
            LineMark(x: .value("时间", Date(timeIntervalSince1970: point.timestamp)), y: .value("KB/s", point.rate), series: .value("采样段", point.series))
                .foregroundStyle(point.upload ? Color.indigo : Color.teal)
                .lineStyle(StrokeStyle(lineWidth: compact ? 1.5 : 2))
        }
        .chartXScale(domain: Date(timeIntervalSince1970: end - 300)...Date(timeIntervalSince1970: end))
        .chartYAxis { if !compact { AxisMarks(position: .leading) } }
        .chartXAxis { if !compact { AxisMarks(values: .stride(by: .minute)) { AxisValueLabel(format: .dateTime.hour().minute(), anchor: .top) } } }
        .chartLegend(.hidden)
        .accessibilityLabel(compact ? "速率趋势" : "最近五分钟上传和下载速率，缺失采样显示为断点")
    }
}
