import SwiftUI
import AVKit

// MARK: - Movie Player Container

struct MoviePlayerContainerView: View {
    let client: APIClient
    let movie: MovieDetail
    let resumePosition: Double

    @Environment(\.dismiss) private var dismiss
    @State private var playerRef: AVPlayer?

    var body: some View {
        MoviePlayerView(
            client: client,
            movie: movie,
            resumePosition: resumePosition,
            onDismiss: { stopAndDismiss() },
            onPlayerReady: { playerRef = $0 }
        )
        .ignoresSafeArea()
        .onExitCommand { stopAndDismiss() }
        .onDisappear { stopPlayer() }
    }

    private func stopPlayer() {
        playerRef?.pause()
        playerRef?.replaceCurrentItem(with: nil)
    }

    private func stopAndDismiss() {
        stopPlayer()
        dismiss()
    }
}

// MARK: - Movie Player

struct MoviePlayerView: UIViewControllerRepresentable {
    let client: APIClient
    let movie: MovieDetail
    let resumePosition: Double
    var onDismiss: (() -> Void)?
    var onPlayerReady: ((AVPlayer) -> Void)?

    @Environment(\.dismiss) private var dismiss

    func makeUIViewController(context: Context) -> AVPlayerViewController {
        let controller = AVPlayerViewController()
        let url = client.movieStreamURL(movieId: movie.id)
        let player = AVPlayer(url: url)
        controller.player = player
        controller.delegate = context.coordinator

        if movie.hasExternalSubtitles {
            let subsVC = ExternalMovieSubtitleViewController(
                client: client,
                movie: movie,
                player: player
            )
            controller.customInfoViewControllers = [subsVC]
        }

        context.coordinator.startPlayback(player: player)
        context.coordinator.controller = controller
        onPlayerReady?(player)
        return controller
    }

    func updateUIViewController(_ controller: AVPlayerViewController, context: Context) {}

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    class Coordinator: NSObject, AVPlayerViewControllerDelegate {
        let parent: MoviePlayerView
        private var progressReporter: ProgressReporter?
        private var playerRef: AVPlayer?
        weak var controller: AVPlayerViewController?

        init(_ parent: MoviePlayerView) {
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

            loadSubtitles(player: player)

            let reporter = ProgressReporter()
            let client = parent.client
            let movieId = parent.movie.id
            reporter.start(
                updater: { position, duration in
                    try? await client.updateMovieProgress(
                        movieId: movieId, position: position, duration: duration
                    )
                },
                positionProvider: { [weak player] in
                    guard let player, let item = player.currentItem else { return nil }
                    let pos = player.currentTime().seconds
                    let dur = item.duration.seconds
                    guard pos.isFinite, dur.isFinite, dur > 0 else { return nil }
                    return (position: pos, duration: dur)
                }
            )
            self.progressReporter = reporter

            NotificationCenter.default.addObserver(
                self,
                selector: #selector(playerDidFinish),
                name: .AVPlayerItemDidPlayToEndTime,
                object: player.currentItem
            )

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
                    let english = AVMediaSelectionGroup.mediaSelectionOptions(
                        from: group.options,
                        with: Locale(identifier: "en")
                    ).first
                    if let track = english {
                        await MainActor.run { item.select(track, in: group) }
                    }
                }
            }
        }

        @objc private func menuPressed() {
            reportFinalProgress()
            Task { @MainActor in parent.onDismiss?() ?? parent.dismiss() }
        }

        @objc private func playerDidFinish(_ notification: Notification) {
            reportFinalProgress()
            Task { @MainActor in parent.onDismiss?() ?? parent.dismiss() }
        }

        nonisolated func playerViewControllerDidEndDismissalTransition(_ playerViewController: AVPlayerViewController) {
            MainActor.assumeIsolated {
                reportFinalProgress()
                parent.onDismiss?() ?? parent.dismiss()
            }
        }

        nonisolated func playerViewControllerShouldDismiss(_ playerViewController: AVPlayerViewController) -> Bool {
            MainActor.assumeIsolated {
                playerViewController.player?.pause()
            }
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

// MARK: - External Subtitle Panel (movies)

final class ExternalMovieSubtitleViewController: UIViewController {
    private let client: APIClient
    private let movie: MovieDetail
    private weak var player: AVPlayer?
    private var subtitles: [SubtitleInfo] = []
    private var activeLanguage: String?
    private var stackView: UIStackView!

    init(client: APIClient, movie: MovieDetail, player: AVPlayer) {
        self.client = client
        self.movie = movie
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
                let subs = try await client.listMovieSubtitles(movieId: movie.id)
                await MainActor.run {
                    self.subtitles = subs
                    self.buildButtons()
                }
            } catch {}
        }
    }

    private func buildButtons() {
        stackView.arrangedSubviews.forEach { $0.removeFromSuperview() }
        stackView.addArrangedSubview(makeButton(title: "Off", language: nil))
        for sub in subtitles {
            stackView.addArrangedSubview(makeButton(title: sub.label, language: sub.language))
        }
    }

    private func makeButton(title: String, language: String?) -> UIButton {
        var config = UIButton.Configuration.plain()
        config.title = title
        config.contentInsets = NSDirectionalEdgeInsets(top: 12, leading: 24, bottom: 12, trailing: 24)
        let isActive = (language == nil && activeLanguage == nil) || (language == activeLanguage)
        config.baseForegroundColor = isActive ? .white : .gray
        config.background.backgroundColor = isActive ? UIColor.white.withAlphaComponent(0.2) : .clear
        config.background.cornerRadius = 12
        let button = UIButton(configuration: config)
        button.addAction(UIAction { [weak self] _ in
            self?.selectSubtitle(language: language)
        }, for: .primaryActionTriggered)
        return button
    }

    private func selectSubtitle(language: String?) {
        activeLanguage = language
        buildButtons()
        guard let player, let item = player.currentItem else { return }

        if let lang = language {
            Task {
                if let group = try? await item.asset.loadMediaSelectionGroup(for: .legible) {
                    let normalized = lang.lowercased()
                    let options = group.options.filter { option in
                        guard let id = option.locale?.language.languageCode?.identifier.lowercased() else { return false }
                        return id == normalized || id.hasPrefix(normalized) || normalized.hasPrefix(id)
                    }
                    if let option = options.first {
                        await MainActor.run { item.select(option, in: group) }
                    }
                }
            }
        } else {
            Task {
                if let group = try? await item.asset.loadMediaSelectionGroup(for: .legible) {
                    await MainActor.run { item.select(nil, in: group) }
                }
            }
        }
    }
}
