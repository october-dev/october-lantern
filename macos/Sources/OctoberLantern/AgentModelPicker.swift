import SwiftUI

/// One picker for a new session and a saved routine. Selection is an explicit harness model id;
/// nil deliberately keeps the harness's own saved configuration.
struct AgentModelPicker: View {
    let kind: AgentKind
    @ObservedObject var model: AppModel
    @Binding var selection: String?
    @State private var searching = false
    @State private var query = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if let list = model.models[kind.rawValue] {
                if let error = list.error {
                    Text(error).font(.system(size: 11.5)).foregroundStyle(Theme.amber)
                    Button("Retry loading models") { model.loadModels(kind) }
                }
                if list.choosable {
                    Menu {
                        Button("Default (agent configuration)") { selection = nil }
                        if list.models.count <= 25 {
                            ForEach(list.models, id: \.id) { option in Button(option.label) { selection = option.id } }
                        }
                        Divider()
                        Button(list.models.count > 25 ? "Search \(list.models.count) models…" : "Other model…") { searching = true }
                    } label: {
                        HStack {
                            Image(systemName: "cpu").accessibilityHidden(true)
                            Text(label(list)).lineLimit(1).truncationMode(.middle)
                            Spacer()
                            Image(systemName: "chevron.up.chevron.down").font(.system(size: 9, weight: .semibold)).accessibilityHidden(true)
                        }
                        .font(.system(size: 12.5)).foregroundStyle(Theme.ink.opacity(0.9))
                        .padding(.horizontal, 12).padding(.vertical, 8)
                        .background(RoundedRectangle(cornerRadius: 10).fill(Theme.faint))
                        .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(Theme.hairline))
                        .contentShape(Rectangle()).accessibilityElement(children: .combine)
                    }
                    .menuStyle(.button).buttonStyle(.plain).menuIndicator(.hidden)
                    .accessibilityLabel("Model: \(label(list))")
                    if searching { search(list) }
                    Text(selection == nil ? "Default follows the agent's saved configuration." : "Model: \(selection ?? "")")
                        .font(.system(size: 10.5)).foregroundStyle(Theme.muted).textSelection(.enabled)
                } else {
                    Text("\(kind.displayName) starts with its own default model.").font(.system(size: 11.5)).foregroundStyle(Theme.muted)
                }
            } else {
                HStack {
                    ProgressView().controlSize(.small)
                    Text("Loading \(kind.displayName)'s models…").font(.system(size: 11.5)).foregroundStyle(Theme.muted)
                }
            }
        }
        .onAppear { model.loadModels(kind) }
        .onChange(of: kind) { _, _ in searching = false; query = ""; model.loadModels(kind) }
    }

    private func label(_ list: EngineMessage.ModelList) -> String {
        guard let selection else { return "Default model" }
        return list.models.first { $0.id == selection }.map { option in
            option.group.map { "\(option.label) · \($0)" } ?? option.label
        } ?? selection
    }

    private func pick(_ id: String) { selection = id; searching = false; query = "" }

    private func search(_ list: EngineMessage.ModelList) -> some View {
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        let matches = Array(list.models.filter {
            trimmed.isEmpty || $0.id.localizedCaseInsensitiveContains(trimmed) || $0.label.localizedCaseInsensitiveContains(trimmed)
        }.prefix(trimmed.isEmpty ? 6 : 8))
        return VStack(alignment: .leading, spacing: 2) {
            TextField("Model name or id", text: $query).textFieldStyle(.roundedBorder)
                .onSubmit { if !trimmed.isEmpty { pick(trimmed) } }
            ForEach(matches, id: \.id) { option in
                Button { pick(option.id) } label: {
                    HStack {
                        Text(option.label).lineLimit(1)
                        Spacer()
                        if let group = option.group { Text(group).foregroundStyle(Theme.muted) }
                    }.font(.system(size: 12)).padding(.vertical, 4).contentShape(Rectangle())
                }.buttonStyle(.plain)
            }
            if !trimmed.isEmpty && !list.models.contains(where: { $0.id == trimmed }) {
                Button("Use “\(trimmed)”") { pick(trimmed) }.font(.system(size: 12)).foregroundStyle(Theme.amber)
            }
        }
    }
}
