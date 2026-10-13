import SwiftUI

struct NodesView: View {
    @Bindable var model: AppModel
    @ViewState private var selection: Set<String> = []
    @ViewState private var sortOrder = [KeyPathComparator(\NodeRecord.name)]
    private var nodes: [NodeRecord] {
        (model.snapshot?.nodes ?? []).filter {
            model.query.isEmpty || "\($0.name) \($0.kind) \($0.group)".localizedCaseInsensitiveContains(model.query)
        }.sorted(using: sortOrder)
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            pageHeader("节点", subtitle: "单击查看详情，双击或按回车使用节点。")
            if nodes.isEmpty {
                emptyState("暂无匹配节点", "尝试其他搜索词，或导入包含节点的配置。", symbol: "point.3.connected.trianglepath.dotted")
            } else {
                Table(nodes, selection: $selection, sortOrder: $sortOrder) {
                    TableColumn("名称", value: \.name) { node in
                        HStack(spacing: 8) {
                            Image(systemName: model.snapshot?.selectedNodeID == node.id ? "checkmark.circle.fill" : "circle")
                                .foregroundStyle(model.snapshot?.selectedNodeID == node.id ? Color.accentColor : Color.secondary)
                            Text(node.name).lineLimit(1).help(node.name)
                        }
                    }.width(min: 240, ideal: 340)
                    TableColumn("类型", value: \.kind).width(min: 80, ideal: 100)
                    TableColumn("分组", value: \.group).width(min: 100, ideal: 130)
                    TableColumn("延迟", value: \.latencySort) { node in
                        Text(node.latencyMS.map { "\($0) ms" } ?? "—")
                            .monospacedDigit().foregroundStyle(node.available ? Color.primary : Color.secondary)
                    }.width(80)
                    TableColumn("状态") { node in Text(node.available ? "可用" : "不可用") }.width(70)
                }
                .contextMenu(forSelectionType: String.self) { ids in
                    Button("使用节点") { use(ids) }.disabled(ids.count != 1 || model.busy)
                    Button("查看详情") { model.inspectedNodeID = ids.first }.disabled(ids.count != 1)
                } primaryAction: { ids in use(ids) }
                .onChange(of: selection) { _, ids in model.inspectedNodeID = ids.first }
                .onKeyPress(.return) { use(selection); return .handled }
            }
            if let node = model.inspectedNode {
                SurfaceCard {
                    HStack(alignment: .top, spacing: 20) {
                        VStack(alignment: .leading, spacing: 8) {
                            Text(node.name).font(.system(size: 14, weight: .semibold)).textSelection(.enabled)
                            Text("\(node.kind) · \(node.group) · \(node.available ? "可用" : "不可用")")
                                .font(.system(size: 12)).foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button(model.snapshot?.selectedNodeID == node.id ? "正在使用" : "使用节点") { use([node.id]) }
                            .disabled(!node.available || model.busy || model.snapshot?.selectedNodeID == node.id)
                    }
                }
            }
        }.padding(24)
    }
    private func use(_ ids: Set<String>) {
        guard ids.count == 1, let id = ids.first else { return }
        Task { await model.command("selectNode", payload: ["nodeID": id]) }
    }
}

struct ProfilesView: View {
    @Bindable var model: AppModel
    private var profiles: [ProfileRecord] {
        (model.snapshot?.profiles ?? []).filter { model.query.isEmpty || $0.name.localizedCaseInsensitiveContains(model.query) }
    }
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    pageHeader("配置", subtitle: "由 Rust 内核统一管理配置事务。")
                    Spacer()
                    Button("导入配置…") { model.showImport = true }
                }
                if profiles.isEmpty { emptyState("暂无配置", "导入本地 YAML 或订阅地址后显示在这里。", symbol: "doc.badge.plus") }
                ForEach(profiles) { profile in
                    SurfaceCard {
                        HStack(alignment: .top, spacing: 16) {
                            Image(systemName: "doc.text").font(.title2).foregroundStyle(.tint)
                            VStack(alignment: .leading, spacing: 8) {
                                Text(profile.name).font(.system(size: 14, weight: .semibold))
                                Text(profile.source).font(.system(size: 12)).foregroundStyle(.secondary)
                                    .lineLimit(2).textSelection(.enabled)
                                if let date = profile.updatedAt {
                                    Text(Date(timeIntervalSince1970: date), style: .date).font(.system(size: 12)).foregroundStyle(.secondary)
                                }
                            }
                            Spacer()
                            if profile.active { Label("当前配置", systemImage: "checkmark.circle.fill").font(.system(size: 12)).foregroundStyle(.tint) }
                            else { Button("使用") { Task { await model.command("selectProfile", payload: ["profileID": profile.id]) } }.disabled(model.busy) }
                        }
                    }
                }
            }.padding(24)
        }
    }
}

struct RulesView: View {
    @Bindable var model: AppModel
    private var rules: [RuleRecord] {
        (model.snapshot?.rules ?? []).filter { model.query.isEmpty || "\($0.kind) \($0.value) \($0.target)".localizedCaseInsensitiveContains(model.query) }
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            pageHeader("规则", subtitle: "按内核返回顺序显示；较早匹配的规则优先生效。")
            if rules.isEmpty { emptyState("暂无匹配规则", "规则由当前配置和覆写内容生成。", symbol: "list.bullet.rectangle") }
            else {
                Table(rules) {
                    TableColumn("类型", value: \.kind).width(min: 140, ideal: 180)
                    TableColumn("匹配内容", value: \.value)
                    TableColumn("目标", value: \.target).width(min: 120, ideal: 160)
                }
            }
        }.padding(24)
    }
}

struct ActivityView: View {
    @Bindable var model: AppModel
    @ViewState private var segment = "connections"
    @ViewState private var selectedConnections: Set<String> = []
    private var connections: [ConnectionRecord] {
        (model.snapshot?.connections ?? []).filter { model.query.isEmpty || "\($0.host) \($0.rule) \($0.node)".localizedCaseInsensitiveContains(model.query) }
    }
    private var logs: [LogRecord] {
        (model.snapshot?.logs ?? []).filter { model.query.isEmpty || $0.message.localizedCaseInsensitiveContains(model.query) }
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                pageHeader("活动", subtitle: "当前连接与内核日志。")
                Spacer()
                Picker("活动类型", selection: $segment) {
                    Text("连接").tag("connections")
                    Text("日志").tag("logs")
                }.pickerStyle(.segmented).frame(width: 180)
            }
            if segment == "connections" {
                if connections.isEmpty { emptyState("暂无活动连接", "内核报告的连接会显示在这里。", symbol: "link") }
                else {
                    Table(connections, selection: $selectedConnections) {
                        TableColumn("目标", value: \.host).width(min: 180, ideal: 250)
                        TableColumn("节点", value: \.node)
                        TableColumn("规则", value: \.rule).width(min: 120, ideal: 160)
                        TableColumn("下载") { item in Text(ByteCountFormatter.string(fromByteCount: Int64(clamping: item.downloadBytes), countStyle: .binary)) }.width(90)
                    }
                    .contextMenu(forSelectionType: String.self) { ids in
                        Button("关闭连接") {
                            if let id = ids.first { Task { await model.command("closeConnection", payload: ["connectionID": id]) } }
                        }.disabled(ids.count != 1 || model.busy)
                    }
                }
            } else {
                HStack { Text("\(logs.count) 条日志").font(.system(size: 12)).foregroundStyle(.secondary); Spacer(); Button("清空当前日志") { Task { await model.command("clearLogs") } } }
                if logs.isEmpty { emptyState("暂无日志", "新的内核事件会显示在这里。", symbol: "text.alignleft") }
                else {
                    List(logs) { log in
                        HStack(alignment: .top, spacing: 16) {
                            Text(Date(timeIntervalSince1970: log.timestamp), style: .time).monospacedDigit().foregroundStyle(.secondary).frame(width: 85, alignment: .leading)
                            Text(log.level.uppercased()).foregroundStyle(log.level == "error" ? Color.red : Color.secondary).frame(width: 55, alignment: .leading)
                            Text(log.message).textSelection(.enabled)
                        }.font(.system(size: 12)).padding(.vertical, 5)
                    }
                }
            }
        }.padding(24)
    }
}

struct UnlockView: View {
    @Bindable var model: AppModel
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                pageHeader("解锁检测", subtitle: "检查结果需要真实网络请求，不从节点名称推断。")
                SurfaceCard {
                    VStack(alignment: .leading, spacing: 16) {
                        Label("尚未运行检测", systemImage: "lock.open").font(.system(size: 18, weight: .medium))
                        Text(model.isDemo ? "演示模式不会发起媒体服务检测请求。连接真实内核后，可按服务执行检测并查看结果。" : "选择服务后由 Rust 内核执行检查。")
                            .font(.system(size: 14)).foregroundStyle(.secondary)
                        HStack {
                            Button("检测 YouTube") { Task { await model.command("checkMedia", payload: ["service": "youtube"]) } }
                            Button("检测 Netflix") { Task { await model.command("checkMedia", payload: ["service": "netflix"]) } }
                        }.disabled(model.isDemo || model.busy)
                    }
                }
            }.padding(24)
        }
    }
}

func pageHeader(_ title: String, subtitle: String) -> some View {
    VStack(alignment: .leading, spacing: 8) {
        Text(title).font(.system(size: 28, weight: .semibold))
        Text(subtitle).font(.system(size: 12)).foregroundStyle(.secondary)
    }
}

func emptyState(_ title: String, _ description: String, symbol: String) -> some View {
    ContentUnavailableView { Label(title, systemImage: symbol) } description: { Text(description) }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
}

struct ImportSheet: View {
    @Bindable var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @ViewState private var name = ""
    @ViewState private var source = ""
    @ViewState private var kind = "yaml"
    private var valid: Bool {
        if kind == "yaml" { return !source.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        guard let url = URL(string: source), ["https", "http"].contains(url.scheme?.lowercased() ?? ""), url.host != nil else { return false }
        return true
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("导入配置").font(.system(size: 28, weight: .semibold))
            if model.isDemo {
                Label("演示导入只添加样本记录，不下载或解析配置。", systemImage: "info.circle")
                    .font(.system(size: 12)).foregroundStyle(.secondary)
            }
            TextField("配置名称（可选）", text: $name)
            Picker("来源类型", selection: $kind) {
                Text("YAML 内容").tag("yaml")
                Text("订阅地址").tag("url")
            }.pickerStyle(.segmented)
            if kind == "url" { TextField("https://example.com/subscription", text: $source) }
            else {
                TextEditor(text: $source).font(.system(size: 12, design: .monospaced))
                    .frame(height: 210)
                    .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Color.secondary.opacity(0.25)))
            }
            if !source.isEmpty && !valid { Text("请输入有效的 HTTP 或 HTTPS 订阅地址。 ").font(.system(size: 12)).foregroundStyle(.red) }
            if let failure = model.failure {
                Text(failure.message).font(.system(size: 12)).foregroundStyle(.red)
                DisclosureGroup("错误详情") {
                    Text("\(failure.code)\n\(failure.detail)")
                        .font(.system(size: 12, design: .monospaced)).textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            HStack {
                Spacer()
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction)
                Button(model.busy ? "正在导入…" : "导入") {
                    Task { await model.command("importProfile", payload: ["name": name.isEmpty ? "新配置" : name, "source": source, kind: source]); if model.failure == nil { dismiss() } }
                }.keyboardShortcut(.defaultAction).disabled(!valid || model.busy)
            }
        }.padding(24).frame(width: 580)
    }
}

struct FailureSheet: View {
    var failure: ClientFailure?
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Label(failure?.message ?? "暂无错误", systemImage: "exclamationmark.triangle")
                .font(.system(size: 20, weight: .semibold))
            LabeledContent("错误代码", value: failure?.code ?? "—").font(.system(size: 12))
            ScrollView { Text(failure?.detail ?? "没有可显示的错误详情。").font(.system(size: 12, design: .monospaced)).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading) }
                .frame(minHeight: 120, maxHeight: 260)
            HStack { Spacer(); Button("完成") { dismiss() }.keyboardShortcut(.defaultAction) }
        }.padding(24).frame(width: 580)
    }
}
