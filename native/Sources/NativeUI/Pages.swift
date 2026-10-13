import SwiftUI
import UniformTypeIdentifiers

struct NodesView: View {
    @Bindable var model: AppModel
    @ViewState private var selection: Set<String> = []
    @ViewState private var sortOrder = [KeyPathComparator(\NodeRecord.name)]
    @ViewState private var frozenOrder: [String] = []
    @ViewState private var showProviders = false
    private var nodes: [NodeRecord] {
        let records = (model.snapshot?.nodes ?? []).filter {
            model.query.isEmpty || "\($0.name) \($0.kind) \($0.group)".localizedCaseInsensitiveContains(model.query)
        }
        if model.testingNodes {
            let positions = Dictionary(uniqueKeysWithValues: frozenOrder.enumerated().map { ($0.element, $0.offset) })
            return records.sorted { (positions[$0.id] ?? Int.max) < (positions[$1.id] ?? Int.max) }
        }
        return records.sorted(using: sortOrder)
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                pageHeader("节点", subtitle: "单击查看详情，双击或按回车使用节点。")
                Spacer()
                Button("节点源…") { showProviders = true }
                if model.testingNodes {
                    Button("取消测速") { model.cancelNodeTests() }
                }
                Button(model.testingNodes ? "测速中…" : model.isDemo ? "演示测速" : "全部测速") { model.testNodes() }
                    .disabled(model.busy || (!model.isDemo && model.snapshot?.coreStatus != "running"))
            }
            if model.testingNodes {
                HStack {
                    ProgressView(value: Double(model.testCompleted), total: Double(max(1, model.testTotal)))
                    Text("\(model.testCompleted) / \(model.testTotal)").monospacedDigit().font(.caption)
                }.accessibilityLabel("已检测 \(model.testCompleted) 个，共 \(model.testTotal) 个节点")
            }
            if nodes.isEmpty {
                emptyState("暂无匹配节点", "尝试其他搜索词，或导入包含节点的配置。", symbol: "point.3.connected.trianglepath.dotted")
            } else {
                Table(nodes, selection: $selection, sortOrder: $sortOrder) {
                    TableColumn("名称", value: \.name) { node in
                        HStack(spacing: 8) {
                            Image(systemName: model.snapshot?.selectedNodeID == node.id ? "checkmark.circle.fill" : "circle")
                                .foregroundStyle(model.snapshot?.selectedNodeID == node.id ? Color.accentColor : Color.secondary)
                            Text(node.name).lineLimit(1).truncationMode(.middle).help(node.name)
                                .padding(.vertical, 4)
                        }
                    }.width(min: 180, ideal: 340, max: 520)
                    TableColumn("类型", value: \.kind).width(min: 80, ideal: 100)
                    TableColumn("分组", value: \.group).width(min: 100, ideal: 130)
                    TableColumn("延迟", value: \.latencySort) { node in
                        Text(node.latencyMS.map { "\($0) ms" } ?? "—")
                            .monospacedDigit().foregroundStyle(node.available ? Color.primary : Color.secondary)
                    }.width(80)
                    TableColumn("状态") { node in Text(node.available ? (node.latencyMS == nil ? "未检测" : "可用") : "不可用") }.width(70)
                }
                .contextMenu(forSelectionType: String.self) { ids in
                    Button("使用节点") { use(ids) }.disabled(ids.count != 1 || model.busy)
                    Button("查看详情") { model.inspectedNodeID = ids.first }.disabled(ids.count != 1)
                    Button("测速") { if let id = ids.first { model.testNodes(nodeID: id) } }
                        .disabled(ids.count != 1 || model.busy)
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
                            if let date = node.testedAt {
                                Text("\(node.testURL ?? "") · \(Date(timeIntervalSince1970: date).formatted(date: .omitted, time: .standard))")
                                    .font(.system(size: 12)).foregroundStyle(.secondary).textSelection(.enabled)
                            }
                        }
                        Spacer()
                        Button(model.snapshot?.selectedNodeID == node.id ? "正在使用" : "使用节点") { use([node.id]) }
                            .disabled(!node.available || model.busy || model.snapshot?.selectedNodeID == node.id)
                    }
                }
            }
        }.padding(24)
            .sheet(isPresented: $showProviders) {
                ProviderSheet(model: model, category: "proxies")
            }
            .onChange(of: model.testingNodes) { _, testing in
                if testing { frozenOrder = (model.snapshot?.nodes ?? []).sorted(using: sortOrder).map(\.id) }
            }
    }
    private func use(_ ids: Set<String>) {
        guard ids.count == 1, let id = ids.first else { return }
        Task { await model.command("selectNode", payload: ["nodeID": id]) }
    }
}

struct ProfilesView: View {
    @Bindable var model: AppModel
    @ViewState private var editing: ProfileRecord?
    @ViewState private var removing: ProfileRecord?
    @ViewState private var importFile = false
    private var profiles: [ProfileRecord] {
        (model.snapshot?.profiles ?? []).filter { model.query.isEmpty || $0.name.localizedCaseInsensitiveContains(model.query) }
    }
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    pageHeader("订阅", subtitle: "订阅与本地配置，验证通过后保存并生效。")
                    Spacer()
                    Button("导入文件…") { importFile = true }
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
                            Menu {
                                Button("编辑…") { editing = profile }
                                Button("更新订阅") { Task { await model.command("updateProfile", payload: ["profileID": profile.id]) } }
                                    .disabled(!profile.source.hasPrefix("http") || model.isDemo)
                                Divider()
                                Button("删除…", role: .destructive) { removing = profile }
                            } label: { Image(systemName: "ellipsis") }.disabled(model.busy)
                        }
                    }
                }
            }.padding(24)
        }
        .sheet(item: $editing) { profile in ProfileEditor(model: model, profile: profile) }
        .confirmationDialog("删除配置“\(removing?.name ?? "")”？", isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } })) {
            Button("删除", role: .destructive) {
                if let profile = removing { Task { await model.command("deleteProfile", payload: ["profileID": profile.id]) } }
                removing = nil
            }
        }
        .fileImporter(isPresented: $importFile, allowedContentTypes: [UTType(filenameExtension: "yaml") ?? .plainText, UTType(filenameExtension: "yml") ?? .plainText]) { result in
            Task {
                do {
                    let url = try result.get()
                    let access = url.startAccessingSecurityScopedResource()
                    defer { if access { url.stopAccessingSecurityScopedResource() } }
                    let size = try url.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
                    guard size <= 16 * 1024 * 1024 else { throw ClientFailure(code: "file_too_large", message: "配置文件超过 16 MiB", detail: url.lastPathComponent, retryable: false) }
                    let yaml = try String(contentsOf: url, encoding: .utf8)
                    await model.command("importProfile", payload: ["name": url.deletingPathExtension().lastPathComponent, "filename": url.lastPathComponent, "yaml": yaml])
                } catch { model.report(error) }
            }
        }
    }
}

struct RulesView: View {
    @Bindable var model: AppModel
    @ViewState private var segment = "rules"
    @ViewState private var showOverrides = false
    private var rules: [RuleRecord] {
        (model.snapshot?.rules ?? []).filter { model.query.isEmpty || "\($0.kind) \($0.value) \($0.target)".localizedCaseInsensitiveContains(model.query) }
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                pageHeader("规则", subtitle: "按实际生效顺序展示，覆写独立保存。")
                Spacer()
                Button("本地覆写…") { showOverrides = true }
            }
            Picker("规则内容", selection: $segment) {
                Text("生效规则").tag("rules")
                Text("规则源").tag("providers")
            }.pickerStyle(.segmented).frame(width: 240)
            if segment == "providers" { ProviderListView(model: model, category: "rules") }
            else if rules.isEmpty { emptyState("暂无匹配规则", "规则由当前配置和覆写内容生成。", symbol: "list.bullet.rectangle") }
            else {
                Table(rules) {
                    TableColumn("类型", value: \.kind).width(min: 140, ideal: 180)
                    TableColumn("匹配内容", value: \.value)
                    TableColumn("目标", value: \.target).width(min: 120, ideal: 160)
                }
            }
        }.padding(24).sheet(isPresented: $showOverrides) { OverridesEditor(model: model) }
            .onAppear { if model.isDemo && CommandLine.arguments.contains("--rules=providers") { segment = "providers" } }
    }
}

struct ActivityView: View {
    @Bindable var model: AppModel
    @ViewState private var segment = "connections"
    @ViewState private var selectedConnections: Set<String> = []
    @ViewState private var detail: ConnectionRecord?
    @ViewState private var logLevel = "all"
    @ViewState private var paused = false
    @ViewState private var heldLogs: [LogRecord] = []
    private var connections: [ConnectionRecord] {
        (model.snapshot?.connections ?? []).filter { model.query.isEmpty || "\($0.host) \($0.rule) \($0.node)".localizedCaseInsensitiveContains(model.query) }
    }
    private var logs: [LogRecord] {
        LogPresentation.filter(paused ? heldLogs : model.snapshot?.logs ?? [], level: logLevel, query: model.query)
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
                        Button("查看详情") { detail = connections.first { ids.contains($0.id) } }.disabled(ids.count != 1)
                        Button("关闭连接") {
                            if let id = ids.first { Task { await model.command("closeConnection", payload: ["connectionID": id]) } }
                        }.disabled(ids.count != 1 || model.busy)
                    } primaryAction: { ids in detail = connections.first { ids.contains($0.id) } }
                    .onKeyPress(.return) { detail = connections.first { selectedConnections.contains($0.id) }; return .handled }
                }
            } else {
                HStack {
                    Picker("日志级别", selection: $logLevel) {
                        Text("全部级别").tag("all")
                        Text("调试").tag("debug")
                        Text("信息").tag("info")
                        Text("警告").tag("warning")
                        Text("错误").tag("error")
                    }.frame(width: 160)
                    Text("\(logs.count) 条\(paused ? " · 已暂停显示" : "")").font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    Button(paused ? "继续显示" : "暂停显示", systemImage: paused ? "play" : "pause") {
                        if !paused { heldLogs = model.snapshot?.logs ?? [] }
                        paused.toggle()
                        if !paused { heldLogs.removeAll() }
                    }
                    Button("清空当前日志") { heldLogs.removeAll(); Task { await model.command("clearLogs") } }
                }
                if logs.isEmpty { emptyState("暂无日志", "新的内核事件会显示在这里。", symbol: "text.alignleft") }
                else {
                    List(logs) { log in
                        HStack(alignment: .top, spacing: 16) {
                            Text(Date(timeIntervalSince1970: log.timestamp), style: .time).monospacedDigit().foregroundStyle(.secondary).frame(width: 85, alignment: .leading)
                            Text(log.level.uppercased()).foregroundStyle(log.level == "error" ? Color.red : Color.secondary)
                                .lineLimit(1).frame(width: 72, alignment: .leading)
                            Text(log.message).textSelection(.enabled)
                        }.font(.system(size: 12)).padding(.vertical, 5)
                    }
                }
            }
        }.padding(24).sheet(item: $detail) { ConnectionDetailView(model: model, connection: $0) }
            .onAppear {
                if model.isDemo && CommandLine.arguments.contains("--activity=logs") { segment = "logs" }
                if model.isDemo && CommandLine.arguments.contains("--connection-detail") { detail = model.snapshot?.connections.first }
            }
    }
}

struct UnlockView: View {
    @Bindable var model: AppModel
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    pageHeader("解锁检测", subtitle: "通过当前代理检测服务可用性与地区。")
                    Spacer()
                    Button("全部检测") { Task { await model.command("checkAllMedia") } }
                        .disabled(model.isDemo || model.busy || model.snapshot?.coreStatus != "running")
                }
                if model.isDemo { emptyState("演示模式", "真实检测需要启动内核。", symbol: "lock.open") }
                ForEach(model.snapshot?.unlock ?? []) { item in
                    SurfaceCard {
                        HStack {
                            VStack(alignment: .leading, spacing: 8) {
                                Text(item.id).font(.system(size: 14, weight: .semibold))
                                Text(item.region ?? "地区未知").font(.system(size: 12)).foregroundStyle(.secondary)
                            }
                            Spacer()
                            VStack(alignment: .trailing, spacing: 8) {
                                Text(item.status == "Pending" ? "尚未检测" : item.status)
                                Text(item.checkedAt ?? "").font(.system(size: 12)).foregroundStyle(.secondary)
                            }
                            Button("检测") { Task { await model.command("checkMedia", payload: ["service": item.id]) } }
                                .disabled(model.isDemo || model.busy || model.snapshot?.coreStatus != "running")
                        }
                    }
                }
            }.padding(24)
        }
    }
}

struct ProfileEditor: View {
    @Bindable var model: AppModel
    let profile: ProfileRecord
    @Environment(\.dismiss) private var dismiss
    @ViewState private var yaml = ""
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("编辑 \(profile.name)").font(.title2)
            TextEditor(text: $yaml).font(.system(size: 12, design: .monospaced)).frame(minHeight: 320)
            if let error = model.failure { Text(error.detail).font(.caption).foregroundStyle(.red).textSelection(.enabled) }
            HStack {
                Spacer()
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("保存") {
                    Task {
                        await model.command("updateProfile", payload: ["profileID": profile.id, "yaml": yaml])
                        if model.failure == nil { dismiss() }
                    }
                }.keyboardShortcut(.defaultAction).disabled(model.busy || model.isDemo)
            }
        }.padding(24).frame(width: 700, height: 480)
            .onAppear { yaml = model.snapshot?.profileDocuments[profile.id] ?? "" }
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
        if kind == "yaml" || kind == "uri" { return !source.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
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
                Text("节点链接").tag("uri")
            }.pickerStyle(.segmented)
            if kind == "url" { TextField("https://example.com/subscription", text: $source) }
            else {
                TextEditor(text: $source).font(.system(size: 12, design: .monospaced))
                    .frame(height: 210)
                    .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Color.secondary.opacity(0.25)))
            }
            if !source.isEmpty && !valid { Text("请输入有效的 HTTP 或 HTTPS 订阅地址。 ").font(.system(size: 12)).foregroundStyle(.red) }
            Text("自定义 JavaScript 不执行。旧脚本请保留原文件，并将配置变更迁到 YAML 覆盖。")
                .font(.system(size: 12)).foregroundStyle(.secondary)
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
