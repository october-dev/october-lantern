import AVFoundation
import LanternCore
import Speech

/// Dictation with Apple's on-device speech recognizer. Audio never leaves the Mac: when the
/// recognizer for the current language can't run on this Mac, dictation says so instead of
/// falling back to Apple's servers.
@MainActor
final class Dictation: ObservableObject {
    @Published private(set) var isRecording = false
    /// Waiting for Microphone and Speech Recognition permission. Pressing again cancels.
    @Published private(set) var isAuthorizing = false
    @Published private(set) var level: Float = 0
    /// Recognized text (with the prefix) and the draft it belongs to: the recipient id given to
    /// `start`, so text never lands in another conversation's draft.
    var onText: ((String, String) -> Void)?
    var onError: ((String) -> Void)?

    private let recognizer = SFSpeechRecognizer(locale: Locale.current) ?? SFSpeechRecognizer(locale: Locale(identifier: "en-US"))
    private let audio = AVAudioEngine()
    private var request: SFSpeechAudioBufferRecognitionRequest?
    private var task: SFSpeechRecognitionTask?
    private var prefix = ""
    private var owner = ""
    /// One press of the mic: stopping (or sending, or switching conversation) cancels it, so a
    /// permission answer that arrives later starts nothing.
    private var attempts = Attempts()

    var isActive: Bool { isRecording || isAuthorizing }

    /// Starts dictating into the draft of `owner` (a recipient id, or "" for no recipient).
    func start(prefix: String, owner: String) {
        guard !isActive else { return }
        self.prefix = prefix.isEmpty || prefix.hasSuffix(" ") ? prefix : prefix + " "
        self.owner = owner
        let attempt = attempts.begin()
        isAuthorizing = true
        Task {
            let allowed = await Self.authorize()
            guard attempts.isCurrent(attempt) else { return }
            isAuthorizing = false
            guard allowed else {
                attempts.finish(attempt)
                onError?("Lantern needs Microphone and Speech Recognition access. Turn them on in System Settings › Privacy & Security.")
                return
            }
            begin(attempt)
        }
    }

    /// The draft this dictation writes to, while it's active.
    var target: String? { isActive ? owner : nil }

    func stop() {
        attempts.cancel()
        isAuthorizing = false
        guard isRecording else { return }
        audio.inputNode.removeTap(onBus: 0)
        audio.stop()
        request?.endAudio()
        // A late result from this task must not touch the draft or stop a later recording.
        task?.cancel()
        task = nil
        request = nil
        isRecording = false
        level = 0
    }

    private static func authorize() async -> Bool {
        let speech = await withCheckedContinuation { cont in
            SFSpeechRecognizer.requestAuthorization { cont.resume(returning: $0 == .authorized) }
        }
        let mic = await AVCaptureDevice.requestAccess(for: .audio)
        return speech && mic
    }

    private func begin(_ attempt: Int) {
        guard let recognizer, recognizer.isAvailable else {
            attempts.finish(attempt)
            onError?("Speech recognition isn't available right now.")
            return
        }
        guard recognizer.supportsOnDeviceRecognition else {
            let language = Locale.current.localizedString(forIdentifier: recognizer.locale.identifier) ?? recognizer.locale.identifier
            attempts.finish(attempt)
            onError?("On-device dictation isn't available for \(language) on this Mac, and Lantern doesn't send audio to Apple.")
            return
        }
        let request = SFSpeechAudioBufferRecognitionRequest()
        request.shouldReportPartialResults = true
        request.requiresOnDeviceRecognition = true
        self.request = request

        let input = audio.inputNode
        let format = input.outputFormat(forBus: 0)
        input.installTap(onBus: 0, bufferSize: 1024, format: format) { [weak self] buffer, _ in
            request.append(buffer)
            let level = Self.rms(buffer)
            Task { @MainActor in self?.level = level }
        }
        do {
            audio.prepare()
            try audio.start()
        } catch {
            input.removeTap(onBus: 0)
            attempts.finish(attempt)
            onError?("Couldn't start the microphone: \(error.localizedDescription)")
            return
        }
        isRecording = true
        task = recognizer.recognitionTask(with: request) { [weak self] result, error in
            let text = result?.bestTranscription.formattedString
            let final = result?.isFinal ?? false
            Task { @MainActor in
                guard let self, self.request === request, self.attempts.isCurrent(attempt) else { return }
                if let text { self.onText?(self.prefix + text, self.owner) }
                if final || error != nil { self.stop() }
            }
        }
    }

    private nonisolated static func rms(_ buffer: AVAudioPCMBuffer) -> Float {
        guard let data = buffer.floatChannelData?[0], buffer.frameLength > 0 else { return 0 }
        var sum: Float = 0
        for i in 0..<Int(buffer.frameLength) { sum += data[i] * data[i] }
        return min(1, sqrt(sum / Float(buffer.frameLength)) * 8)
    }
}
