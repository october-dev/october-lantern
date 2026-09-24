import Foundation

/// The composer's drafts: one per conversation, keyed by recipient agent id ("" while no recipient
/// is chosen). Every edit gets a new revision, so a send acknowledgement clears exactly the text
/// that was sent to that agent, never another agent's draft or a later retype of the same words.
public struct Drafts: Equatable {
    public struct Draft: Equatable {
        public var text: String
        public var revision: Int
    }

    /// What a send carries, so its acknowledgement can clear only what was sent.
    public struct Ticket: Equatable, Hashable {
        public let recipient: String
        public let revision: Int
        public let text: String
    }

    /// The agent the composer is addressed to. Once chosen (or once you start typing) it stays
    /// chosen: a draft never quietly changes recipient because the agent list changed.
    public private(set) var recipient: String?
    private var drafts: [String: Draft] = [:]
    private var nextRevision = 0

    public init() {}

    private var key: String { recipient ?? "" }

    /// The draft shown for the current recipient.
    public var text: String { drafts[key]?.text ?? "" }

    public func text(for recipient: String) -> String { drafts[recipient]?.text ?? "" }

    public func revision(for recipient: String) -> Int? { drafts[recipient]?.revision }

    /// Typing. The first keystroke with no chosen recipient locks in `fallback` (the agent shown
    /// as the recipient at that moment).
    public mutating func type(_ text: String, fallback: String?) {
        if !text.isEmpty, recipient == nil, let fallback {
            // Text typed before any agent existed moves to the recipient it's now locked to.
            drafts[""] = nil
            recipient = fallback
        }
        set(text, for: key)
    }

    /// Sets one conversation's draft, whichever conversation is showing (dictation writes to the
    /// draft it started in).
    public mutating func set(_ text: String, for recipient: String) {
        guard drafts[recipient]?.text != text else { return }
        if text.isEmpty {
            drafts[recipient] = nil
        } else {
            nextRevision += 1
            drafts[recipient] = Draft(text: text, revision: nextRevision)
        }
    }

    /// Switches conversation (opening a chat, a card, a notification). Drafts stay with their agents.
    public mutating func address(_ recipient: String?) {
        self.recipient = recipient
    }

    /// Changes the recipient of the draft being written: an explicit choice in the recipient menu.
    /// The text moves only when the new recipient has no draft of its own.
    public mutating func readdress(_ recipient: String) {
        guard recipient != self.recipient else { return }
        let moving = drafts[key]
        if let moving, drafts[recipient] == nil {
            drafts[key] = nil
            drafts[recipient] = moving
        }
        self.recipient = recipient
    }

    /// The current draft, ready to send, or nil when there's nothing to send or no recipient.
    public func ticket() -> Ticket? {
        guard let recipient, let draft = drafts[recipient] else { return nil }
        let text = draft.text.trimmingCharacters(in: .whitespacesAndNewlines)
        return text.isEmpty ? nil : Ticket(recipient: recipient, revision: draft.revision, text: text)
    }

    /// A send finished: clears the draft only if it is still the revision that was sent.
    public mutating func sent(_ ticket: Ticket) {
        if drafts[ticket.recipient]?.revision == ticket.revision { drafts[ticket.recipient] = nil }
    }

    /// Forgets drafts for agents that are gone, except the one still addressed.
    public mutating func prune(keeping live: Set<String>) {
        drafts = drafts.filter { $0.key.isEmpty || $0.key == recipient || live.contains($0.key) }
    }
}
