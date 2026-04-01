import SwiftUI
import AVKit

// MARK: - Player Container (handles Menu button dismiss on real Apple TV)

struct PlayerContainerView: View {
    let client: APIClient
    let episode: EpisodeItem
    let resumePosition: Double

    @Environment(\.dismiss) private var dismiss
    @State private var showCast = false

    var body: some View {
        PlayerView(
            client: client,
            episode: episode,
            resumePosition: resumePosition,
            onDismiss: { dismiss() },
            onShowCast: { showCast = true }
        )
        .ignoresSafeArea()
        .onExitCommand { dismiss() }
        .fullScreenCover(isPresented: $showCast) {
            NavigationStack {
                EpisodeCastView(episode: episode, client: client)
            }
        }
    }
}

// MARK: - Player (UIViewControllerRepresentable)

struct PlayerView: UIViewControllerRepresentable {
    let client: APIClient
    let episode: EpisodeItem
    let resumePosition: Double
    var onDismiss: (() -> Void)?
    var onShowCast: (() -> Void)?

    @Environment(\.dismiss) private var dismiss

    func makeUIViewController(context: Context) -> AVPlayerViewController {
        let controller = AVPlayerViewController()
        let url = client.streamURL(episodeId: episode.id)
        let player = AVPlayer(url: url)
        controller.player = player
        controller.delegate = context.coordinator

        // Add Cast & Crew button to the transport bar
        let castAction = UIAction(title: "Cast & Crew", image: UIImage(systemName: "person.2")) { _ in
            // Pause playback while showing cast
            player.pause()
            context.coordinator.parent.onShowCast?()
        }
        controller.transportBarCustomMenuItems = [castAction]

        // External subtitles panel (if episode has external .srt files)
        if episode.hasExternalSubtitles {
            let subsVC = ExternalSubtitleViewController(
                client: client,
                episode: episode,
                player: player
            )
            controller.customInfoViewControllers = [subsVC]
        }

        context.coordinator.startPlayback(player: player)
        context.coordinator.controller = controller
        return controller
    }

    func updateUIViewController(_ controller: AVPlayerViewController, context: Context) {}

    func makeCoordinator() -> Coordinator {
        Coordinator(self)
    }

    class Coordinator: NSObject, AVPlayerViewControllerDelegate {
        let parent: PlayerView
        private var progressReporter: ProgressReporter?
        private var playerRef: AVPlayer?
        weak var controller: AVPlayerViewController?

        init(_ parent: PlayerView) {
            self.parent = parent
            super.init()
        }

        func startPlayback(player: AVPlayer) {
            self.playerRef = player

            if parent.resumePosition > 0 {
                let time = CMTime(seconds: parent.resumePosition, preferredTimescale: 600)
                player.seek(to: time) { [weak player] _ in
                    player?.play()
                }
            } else {
                player.play()
            }

            // Auto-select English subtitles
            loadSubtitles(player: player)

            // Start progress reporting
            let reporter = ProgressReporter()
            reporter.start(client: parent.client, episodeId: parent.episode.id) { [weak player] in
                guard let player, let item = player.currentItem else { return nil }
                let pos = player.currentTime().seconds
                let dur = item.duration.seconds
                guard pos.isFinite, dur.isFinite, dur > 0 else { return nil }
                return (position: pos, duration: dur)
            }
            self.progressReporter = reporter

            // Observe end of playback
            NotificationCenter.default.addObserver(
                self,
                selector: #selector(playerDidFinish),
                name: .AVPlayerItemDidPlayToEndTime,
                object: player.currentItem
            )

            // Add Menu button press recognizer as fallback dismiss
            if let controller {
                let menuPress = UITapGestureRecognizer(target: self, action: #selector(menuPressed))
                menuPress.allowedPressTypes = [NSNumber(value: UIPress.PressType.menu.rawValue)]
                controller.view.addGestureRecognizer(menuPress)
            }
        }

        private func loadSubtitles(player: AVPlayer) {
            guard let item = player.currentItem else { return }

            Task {
                if let group = try? await item.asset.loadMediaSelectionGroup(for: .legible) {
                    // Auto-select English subtitles if available
                    let english = AVMediaSelectionGroup.mediaSelectionOptions(
                        from: group.options,
                        with: Locale(identifier: "en")
                    ).first
                    if let track = english {
                        await MainActor.run {
                            item.select(track, in: group)
                        }
                    }
                }
            }
        }

        @objc private func menuPressed() {
            reportFinalProgress()
            Task { @MainActor in
                parent.onDismiss?() ?? parent.dismiss()
            }
        }

        @objc private func playerDidFinish(_ notification: Notification) {
            reportFinalProgress()
            Task { @MainActor in
                parent.onDismiss?() ?? parent.dismiss()
            }
        }

        nonisolated func playerViewControllerDidEndDismissalTransition(_ playerViewController: AVPlayerViewController) {
            MainActor.assumeIsolated {
                reportFinalProgress()
                parent.onDismiss?() ?? parent.dismiss()
            }
        }

        nonisolated func playerViewControllerShouldDismiss(_ playerViewController: AVPlayerViewController) -> Bool {
            // Pause immediately so audio stops before the dismiss animation
            playerViewController.player?.pause()
            return true
        }

        private func reportFinalProgress() {
            guard let player = playerRef, let item = player.currentItem else {
                progressReporter?.stop(finalPosition: 0, finalDuration: 0)
                return
            }
            let pos = player.currentTime().seconds
            let dur = item.duration.seconds
            let safePos = pos.isFinite ? pos : 0
            let safeDur = dur.isFinite ? dur : 0
            player.pause()
            player.replaceCurrentItem(with: nil)
            progressReporter?.stop(finalPosition: safePos, finalDuration: safeDur)
        }

        deinit {
            NotificationCenter.default.removeObserver(self)
        }
    }
}

// MARK: - External Subtitle Panel (shown in player info view)

final class ExternalSubtitleViewController: UIViewController {
    private let client: APIClient
    private let episode: EpisodeItem
    private weak var player: AVPlayer?
    private var subtitles: [SubtitleInfo] = []
    private var activeLanguage: String?
    private var legibleOutput: AVPlayerItemLegibleOutput?
    private var stackView: UIStackView!

    init(client: APIClient, episode: EpisodeItem, player: AVPlayer) {
        self.client = client
        self.episode = episode
        self.player = player
        super.init(nibName: nil, bundle: nil)
        self.title = "External Subtitles"
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()

        stackView = UIStackView()
        stackView.axis = .horizontal
        stackView.spacing = 20
        stackView.alignment = .center
        stackView.translatesAutoresizingMaskIntoConstraints = false

        view.addSubview(stackView)
        NSLayoutConstraint.activate([
            stackView.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            stackView.centerYAnchor.constraint(equalTo: view.centerYAnchor),
            stackView.heightAnchor.constraint(equalToConstant: 80),
        ])

        loadSubtitles()
    }

    private func loadSubtitles() {
        Task {
            do {
                let subs = try await client.listSubtitles(episodeId: episode.id)
                await MainActor.run {
                    self.subtitles = subs
                    self.buildButtons()
                }
            } catch {}
        }
    }

    private func buildButtons() {
        stackView.arrangedSubviews.forEach { $0.removeFromSuperview() }

        // "Off" button
        let offButton = makeButton(title: "Off", language: nil)
        stackView.addArrangedSubview(offButton)

        for sub in subtitles {
            let button = makeButton(title: sub.label, language: sub.language)
            stackView.addArrangedSubview(button)
        }
    }

    private func makeButton(title: String, language: String?) -> UIButton {
        let button = UIButton(type: .system)
        button.setTitle(title, for: .normal)
        button.titleLabel?.font = .systemFont(ofSize: 28, weight: .medium)
        let isActive = (language == nil && activeLanguage == nil) || (language == activeLanguage)
        button.tintColor = isActive ? .white : .gray
        button.backgroundColor = isActive ? UIColor.white.withAlphaComponent(0.2) : .clear
        button.layer.cornerRadius = 12
        button.contentEdgeInsets = UIEdgeInsets(top: 12, left: 24, bottom: 12, right: 24)
        button.tag = language.hashValue
        button.addAction(UIAction { [weak self] _ in
            self?.selectSubtitle(language: language)
        }, for: .primaryActionTriggered)
        return button
    }

    private func selectSubtitle(language: String?) {
        activeLanguage = language
        buildButtons() // Refresh button states

        guard let player, let item = player.currentItem else { return }

        if let lang = language {
            // Load external WebVTT subtitle
            let url = client.subtitleURL(episodeId: episode.id, language: lang)
            // Add as external subtitle using AVPlayerItem
            let asset = AVURLAsset(url: url)
            Task {
                // For external WebVTT, we need to set it up as a timed metadata
                // The most reliable tvOS approach: use AVPlayerItemLegibleOutput
                // For now, try the media selection approach
                if let group = try? await item.asset.loadMediaSelectionGroup(for: .legible) {
                    // Look for a matching external subtitle
                    let options = group.options.filter { option in
                        option.locale?.languageCode == lang
                    }
                    if let option = options.first {
                        await MainActor.run {
                            item.select(option, in: group)
                        }
                    }
                }
            }
        } else {
            // Turn off subtitles
            Task {
                if let group = try? await item.asset.loadMediaSelectionGroup(for: .legible) {
                    await MainActor.run {
                        item.select(nil, in: group)
                    }
                }
            }
        }
    }
}
