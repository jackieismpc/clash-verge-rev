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
    var setupTitle: String? = nil
    var disabled = false
    var action: () -> Void
    var body: some View {
        Button(action: action) {
            HStack(spacing: 6) {
                if status == .applying { ProgressView().controlSize(.mini) }
                else { Image(systemName: status == .on ? "checkmark.circle.fill" : status == .failed ? "exclamationmark.circle" : "circle") }
                Text(title)
                Text(setupTitle ?? status.title).foregroundStyle(.secondary)
            }
        }
        .foregroundStyle(status == .on ? Color.green : status == .failed ? Color.orange : Color.primary)
        .disabled(disabled || (status == .unavailable && setupTitle == nil) || status == .applying)
        .help(setupTitle ?? (status == .unavailable ? "隔离测试禁用系统网络操作" : demo ? "演示状态，不修改系统网络" : status.title))
        .accessibilityLabel("\(title)，\(setupTitle ?? status.title)\(demo ? "，演示模式" : "")")
    }
}


struct SubscriptionScheduleEditor: View {
    @Bindable var model: AppModel
    let profile: ProfileRecord
    @Environment(\.dismiss) private var dismiss
    @ViewState private var minutes = "1440"
    private var valid: Bool { Int(minutes).map { (0...525600).contains($0) } ?? false }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("自动更新 · \(profile.name)").font(.title2)
            TextField("更新间隔（分钟）", text: $minutes)
            Text("0 停用自动更新；默认每天更新。失败会有限重试，原有配置继续生效。")
                .font(.caption).foregroundStyle(.secondary)
            if let error = model.failure { Text(error.detail).font(.caption).foregroundStyle(.red) }
            HStack {
                Spacer()
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("保存") {
                    Task {
                        await model.command("configureSubscription", payload: ["profileID": profile.id, "intervalMinutes": minutes])
                        if model.failure == nil { dismiss() }
                    }
                }.keyboardShortcut(.defaultAction).disabled(!valid || model.busy || model.isDemo)
            }
        }.padding(24).frame(width: 500)
            .onAppear { minutes = String(profile.updateIntervalMinutes ?? 1440) }
    }
}

struct LocalRuleEditor: View {
    @Bindable var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @ViewState private var prepend = ""
    @ViewState private var append = ""
    @ViewState private var delete = ""
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("持久本地规则覆盖").font(.title2)
            Text("每行一条 mihomo 规则。删除忽略逗号两侧空白；后置规则放在源 MATCH 前，末尾 MATCH 可替换默认出口。订阅刷新保留覆盖。")
                .font(.caption).foregroundStyle(.secondary)
            ruleInput("前置规则（优先匹配）", text: $prepend)
            ruleInput("后置规则", text: $append)
            ruleInput("从源规则删除", text: $delete)
            if let error = model.failure { Text(error.detail).font(.caption).foregroundStyle(.red).textSelection(.enabled) }
            HStack {
                Spacer()
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("保存") {
                    Task {
                        await model.command("setRuleOverrides", payload: ["prepend": prepend, "append": append, "delete": delete])
                        if model.failure == nil { dismiss() }
                    }
                }.keyboardShortcut(.defaultAction).disabled(model.busy || model.isDemo)
            }
        }.padding(24).frame(width: 700, height: 600)
            .onAppear {
                prepend = model.snapshot?.ruleOverrides.prepend.joined(separator: "\n") ?? ""
                append = model.snapshot?.ruleOverrides.append.joined(separator: "\n") ?? ""
                delete = model.snapshot?.ruleOverrides.delete.joined(separator: "\n") ?? ""
            }
    }
    private func ruleInput(_ title: String, text: Binding<String>) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.caption)
            TextEditor(text: text).font(.system(size: 12, design: .monospaced))
                .accessibilityLabel(title)
        }
    }
}
