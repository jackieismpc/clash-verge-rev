import SwiftUI

struct ProviderSheet: View {
    @Bindable var model: AppModel
    var category: String
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(category == "proxies" ? "节点源" : "规则源").font(.title2)
            ProviderListView(model: model, category: category)
            HStack { Spacer(); Button("完成") { dismiss() }.keyboardShortcut(.defaultAction) }
        }.padding(24).frame(width: 650, height: 480)
    }
}

struct ProviderListView: View {
    @Bindable var model: AppModel
    var category: String
    private var providers: [ProviderRecord] {
        (model.snapshot?.providers ?? []).filter {
            $0.category == category && (model.query.isEmpty || $0.name.localizedCaseInsensitiveContains(model.query))
        }
    }
    var body: some View {
        if providers.isEmpty {
            emptyState("暂无数据源", "当前配置未提供此类数据源，或没有符合搜索条件的项目。", symbol: "externaldrive")
        } else {
            List(providers) { provider in
                HStack(spacing: 16) {
                    Image(systemName: provider.vehicle.lowercased() == "file" ? "doc.text" : "arrow.down.doc")
                        .foregroundStyle(.secondary)
                    VStack(alignment: .leading, spacing: 6) {
                        Text(provider.name).font(.system(size: 14, weight: .medium)).textSelection(.enabled)
                        Text("\(provider.vehicle) · \(provider.behavior) · \(provider.count) 条")
                            .font(.caption).foregroundStyle(.secondary)
                        if let updated = provider.updatedAt {
                            Text("更新于 \(updated)").font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    Spacer()
                    Button("更新") { Task { await model.command("updateProvider", payload: ["providerID": provider.id]) } }
                        .disabled(model.busy || provider.vehicle.lowercased() == "file" || model.snapshot?.coreStatus == "stopped")
                        .help(provider.vehicle.lowercased() == "file" ? "本地文件无需远程更新" : "更新当前数据源")
                }.padding(.vertical, 8)
            }
        }
    }
}

struct ConnectionDetailView: View {
    @Bindable var model: AppModel
    var connection: ConnectionRecord
    @Environment(\.dismiss) private var dismiss
    private var current: ConnectionRecord { model.snapshot?.connections.first { $0.id == connection.id } ?? connection }
    private var active: Bool { model.snapshot?.connections.contains { $0.id == connection.id } ?? false }
    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack {
                Label("连接详情", systemImage: "link").font(.title2)
                Spacer()
                Text(active ? "连接中" : "已关闭").foregroundStyle(active ? Color.green : .secondary)
            }
            Form {
                LabeledContent("目标域名", value: current.host)
                LabeledContent("目标地址", value: current.destination)
                LabeledContent("来源", value: current.source ?? "—")
                LabeledContent("协议", value: current.network ?? "—")
                LabeledContent("进程", value: current.process?.isEmpty == false ? current.process! : "—")
                LabeledContent("命中规则", value: current.rule)
                LabeledContent("匹配内容", value: current.rulePayload ?? "—")
                LabeledContent("代理链", value: current.node)
                LabeledContent("开始时间", value: current.startedAt ?? "—")
                LabeledContent("上传", value: ByteCountFormatter.string(fromByteCount: Int64(clamping: current.uploadBytes), countStyle: .binary))
                LabeledContent("下载", value: ByteCountFormatter.string(fromByteCount: Int64(clamping: current.downloadBytes), countStyle: .binary))
            }.formStyle(.grouped).textSelection(.enabled)
            HStack {
                Button("关闭连接", role: .destructive) { Task { await model.command("closeConnection", payload: ["connectionID": connection.id]) } }
                    .disabled(!active || model.busy)
                Spacer()
                Button("完成") { dismiss() }.keyboardShortcut(.defaultAction)
            }
        }.padding(24).frame(width: 660, height: 570)
    }
}

struct NetworkFeatureControl: View {
    var title: String
    var status: NetworkFeatureStatus
    var demo: Bool
    var action: () -> Void
    var body: some View {
        Button(action: action) {
            HStack(spacing: 6) {
                if status == .applying { ProgressView().controlSize(.mini) }
                else { Image(systemName: status == .on ? "checkmark.circle.fill" : status == .failed ? "exclamationmark.circle" : "circle") }
                Text(title)
                Text(status.title).foregroundStyle(.secondary)
            }
        }
        .foregroundStyle(status == .on ? Color.green : status == .failed ? Color.orange : Color.primary)
        .disabled(status == .unavailable || status == .applying || !demo)
        .help(status == .unavailable ? "Mac 权限服务尚未接通" : demo ? "演示状态，不修改系统网络" : status.title)
        .accessibilityLabel("\(title)，\(status.title)\(demo ? "，演示模式" : "")")
    }
}
