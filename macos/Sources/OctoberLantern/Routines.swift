import AppKit
import SwiftUI

struct RoutineDraft: Codable, Equatable, Hashable {
    var name = ""
    var prompt = ""
    var cwd = FileManager.default.homeDirectoryForCurrentUser.path
    var kind = "october"
    var model: String?
    var timezone = TimeZone.current.identifier
    var cadence = "daily"
    var hour = 9
    var minute = 0
    var weekday = 0
    var preferences = "Ask me before spending money, sending messages, deleting files outside the project, installing apps or changing system settings."
}

struct LocalRoutine: Decodable, Identifiable {
    let id: String
    let revision: Int
    let draft: RoutineDraft
    let enabled: Bool
    let nextRun: String?
    let error: String?
    let lastRun: Run?
    struct Run: Decodable {
        let id: String
        let state: String
        let launch: String?
        let message: String?
        let at: Double
    }
}

@MainActor
final class LocalRoutines: ObservableObject {
    static let shared = LocalRoutines()
    @Published var items: [LocalRoutine] = []
    @Published var error: String?
    @Published var suggestedDraft: RoutineDraft?
    @Published var pending: Set<String> = []
    var send: (([String: Any]) -> Bool)?

    func request(_ fields: [String: Any]) {
        let id = UUID().uuidString.lowercased()
        var value = fields
        value["requestId"] = id
        error = nil
        guard send?(value) == true else { error = "Lantern's engine isn't available. Try again."; return }
        pending.insert(id)
    }
    func finish(_ id: String, ok: Bool, message: String?) {
        pending.remove(id)
        if !ok { error = message ?? "The routine couldn't be changed. Try again." }
    }
    func save(_ draft: RoutineDraft, editing: LocalRoutine?) {
        guard let data = try? JSONEncoder().encode(draft), let value = try? JSONSerialization.jsonObject(with: data) else { return }
        var fields: [String: Any] = ["action": "save", "draft": value]
        if let editing { fields["id"] = editing.id; fields["revision"] = editing.revision }
        request(fields)
    }
    func change(_ routine: LocalRoutine, _ action: String) {
        request(["action": "change", "id": routine.id, "revision": routine.revision, "change": action])
    }
    func run(_ routine: LocalRoutine) {
        request(["action": "run", "id": routine.id, "revision": routine.revision])
    }
}

struct RoutinesView: View {
    @ObservedObject private var routines = LocalRoutines.shared
    @ObservedObject private var metrics = PanelMetrics.shared
    @State private var draft = RoutineDraft()
    @State private var editing: LocalRoutine?
    @State private var creating = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Text("Run useful tasks on this computer at a time you choose.")
                    .font(.system(size: 12)).foregroundStyle(Theme.muted)
                Text("This computer and Lantern must stay available. Missed runs aren't caught up automatically. Existing work continues when you pause or delete a schedule.")
                    .font(.system(size: 11)).foregroundStyle(Theme.muted)
                if let error = routines.error { Text(error).font(.system(size: 12)).foregroundStyle(Theme.amber).accessibilityLabel("Routine error: \(error)") }
                ForEach(routines.items) { routine in
                    VStack(alignment: .leading, spacing: 6) {
                        Text(routine.draft.name).font(.system(size: 13, weight: .semibold))
                        Text(routine.draft.prompt).font(.system(size: 12)).lineLimit(3)
                        Text("\(routine.enabled ? "On" : "Paused") · \(routine.draft.cadence.capitalized) at \(String(format: "%02d:%02d", routine.draft.hour, routine.draft.minute)) · \(routine.draft.timezone)")
                            .font(.system(size: 11)).foregroundStyle(Theme.muted)
                        if let error = routine.error { Text(error).font(.system(size: 11)).foregroundStyle(Theme.amber) }
                        if let next = routine.nextRun { Text("Next: \(next)").font(.system(size: 11)).textSelection(.enabled) }
                        if let run = routine.lastRun {
                            Text("Last run: \(run.state)").font(.system(size: 11, weight: .medium))
                            if let message = run.message { Text(message).font(.system(size: 11)).foregroundStyle(Theme.muted) }
                        }
                        HStack {
                            Button("Run now") { routines.run(routine) }
                            Button(routine.enabled ? "Pause" : "Enable") { routines.change(routine, routine.enabled ? "pause" : "enable") }
                            Button("Edit") { editing = routine; draft = routine.draft; creating = true }
                            Button("Delete", role: .destructive) { routines.change(routine, "delete") }
                        }.font(.system(size: 11)).disabled(!routines.pending.isEmpty)
                    }.padding(10).frame(maxWidth: .infinity, alignment: .leading)
                        .background(RoundedRectangle(cornerRadius: 10).fill(Theme.faint))
                }
                if creating { form }
                else {
                    Button("Create a routine") { editing = nil; draft = RoutineDraft(); creating = true }
                        .buttonStyle(AmberButtonStyle())
                }
            }.padding(16)
        }
        .frame(maxHeight: max(260, metrics.maxHeight - 80))
        .fixedSize(horizontal: false, vertical: true)
        .onAppear { takeSuggestion() }
        .onChange(of: routines.suggestedDraft) { _, _ in takeSuggestion() }
        .onChange(of: routines.pending) { old, new in
            if !old.isEmpty && new.isEmpty && routines.error == nil { creating = false; editing = nil }
        }
    }

    private func takeSuggestion() {
        if let suggested = routines.suggestedDraft {
            draft = suggested; editing = nil; creating = true
            routines.suggestedDraft = nil
        }
    }
    private var form: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(editing == nil ? "New routine" : "Edit routine").font(.system(size: 13, weight: .semibold))
            TextField("Name", text: $draft.name)
            TextField("What should the agent do?", text: $draft.prompt, axis: .vertical).lineLimit(3...6)
            HStack {
                TextField("Project folder", text: $draft.cwd)
                Button("Choose…") {
                    let panel = NSOpenPanel(); panel.canChooseDirectories = true; panel.canChooseFiles = false
                    if panel.runModal() == .OK, let url = panel.url { draft.cwd = url.path }
                }
            }
            Picker("Agent", selection: $draft.kind) {
                Text("October").tag("october"); Text("Claude Code").tag("claude"); Text("Codex").tag("codex")
            }
            Picker("Repeat", selection: $draft.cadence) {
                Text("Daily").tag("daily"); Text("Weekdays").tag("weekdays"); Text("Weekly").tag("weekly")
            }
            if draft.cadence == "weekly" {
                Picker("Day", selection: $draft.weekday) {
                    ForEach(Array(["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"].enumerated()), id: \.offset) { index, name in Text(name).tag(index) }
                }
            }
            DatePicker("Time", selection: Binding(get: {
                var calendar = Calendar(identifier: .gregorian)
                calendar.timeZone = TimeZone(identifier: draft.timezone) ?? .current
                return calendar.date(from: DateComponents(year: 2001, month: 1, day: 1, hour: draft.hour, minute: draft.minute)) ?? Date()
            }, set: { value in
                var calendar = Calendar(identifier: .gregorian)
                calendar.timeZone = TimeZone(identifier: draft.timezone) ?? .current
                draft.hour = calendar.component(.hour, from: value)
                draft.minute = calendar.component(.minute, from: value)
            }), displayedComponents: .hourAndMinute)
                .environment(\.timeZone, TimeZone(identifier: draft.timezone) ?? .current)
            TextField("Timezone (e.g. America/Los_Angeles)", text: $draft.timezone)
            TextField("Preferences for this routine", text: $draft.preferences, axis: .vertical).lineLimit(2...5)
            Text("Preferences are instructions to the agent, not enforced restrictions. Each run uses the selected agent's account and may use its credits. No overlapping runs. A repeated daylight-saving time runs once; a nonexistent time is skipped.")
                .font(.system(size: 11)).foregroundStyle(Theme.muted)
            HStack {
                Button(editing == nil ? "Confirm and create" : "Confirm changes") { routines.save(draft, editing: editing) }
                    .buttonStyle(AmberButtonStyle()).disabled(draft.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || draft.prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !routines.pending.isEmpty)
                Button("Cancel") { creating = false; editing = nil }
            }
        }.textFieldStyle(.roundedBorder)
    }
}

struct RoutineSnapshot: Decodable { let items: [LocalRoutine]; let error: String? }
struct RoutineReceipt: Decodable { let requestId: String; let ok: Bool; let message: String? }
