import SwiftUI

struct PlayerInfo: Identifiable {
    let id = UUID()
    let episode: EpisodeItem
    let resumePosition: Double
}

struct SeriesDetailView: View {
    let seriesId: String
    let seriesTitle: String

    @Environment(ServerConnection.self) private var connection
    @State private var detail: SeriesDetail?
    @State private var nextEpisode: NextEpisodeResponse?
    @State private var isLoading = true
    @State private var error: CastError?
    @State private var playerEpisode: PlayerInfo?
    @State private var preparingEpisode: EpisodeItem?
    @State private var prepareProgress: Int?

    /// Solid surface used both as the page background and as the target colour for
    /// the hero's bottom-fade gradient. Keeping these in one place means the seam
    /// between hero and content is invisible.
    static let pageBackground = Color(white: 0.06)

    private var client: APIClient? {
        guard let url = connection.baseURL else { return nil }
        return APIClient(baseURL: url)
    }

    var body: some View {
        Group {
            if let error {
                ErrorView(
                    title: "Unable to Load Series",
                    message: error.errorDescription ?? "An unknown error occurred",
                    detail: error.detail,
                    onRetry: { self.error = nil; Task { await loadData() } }
                )
            } else if isLoading {
                ProgressView()
            } else if let detail {
                ZStack {
                    // Solid dark surface. The hero image sits in its own frame inside the
                    // ScrollView, and its bottom gradient fades directly into this colour,
                    // so scrolling past the hero reveals a clean background — no blur.
                    Self.pageBackground.ignoresSafeArea()

                    ScrollView {
                        VStack(alignment: .leading, spacing: 0) {
                            heroSection(detail)
                            contentSection(detail)
                        }
                    }
                    .ignoresSafeArea(edges: [.top, .horizontal])
                }
            }
        }
        .navigationBarHidden(true)
        .fullScreenCover(item: $playerEpisode) {
            Task { await loadData() }
        } content: { info in
            if let client {
                PlayerContainerView(
                    client: client,
                    episode: info.episode,
                    resumePosition: info.resumePosition
                )
            }
        }
        .overlay {
            if let ep = preparingEpisode {
                ZStack {
                    Color.black.opacity(0.85).ignoresSafeArea()
                    VStack(spacing: 20) {
                        ProgressView()
                            .scaleEffect(1.5)
                        Text("Preparing \(ep.episodeLabel)...")
                            .font(.title3)
                        if let progress = prepareProgress {
                            Text("\(progress)%")
                                .font(.headline)
                                .foregroundStyle(.secondary)
                            ProgressView(value: Double(progress), total: 100)
                                .frame(width: 300)
                                .tint(.white)
                        }
                        Text("Converting for Apple TV playback")
                            .font(.caption)
                            .foregroundStyle(Color(white: 0.5))

                        Button("Cancel") {
                            preparingEpisode = nil
                            prepareProgress = nil
                        }
                        .padding(.top, 12)
                    }
                }
            }
        }
        .task { await loadData() }
    }

    // MARK: - Hero Section (Infuse-style full-bleed)

    @ViewBuilder
    private func heroSection(_ detail: SeriesDetail) -> some View {
        GeometryReader { geo in
            ZStack(alignment: .bottom) {
                // Full-bleed backdrop
                if detail.hasBackdrop, let client {
                    AsyncImage(url: client.backdropURL(seriesId: detail.id)) { image in
                        image.resizable().aspectRatio(contentMode: .fill)
                    } placeholder: {
                        Rectangle().fill(Color(white: 0.08))
                    }
                    .frame(width: geo.size.width, height: 620, alignment: .top)
                    .clipped()
                } else if detail.hasArt, let client {
                    // Fallback: blurred poster as backdrop
                    AsyncImage(url: client.artURL(seriesId: detail.id)) { image in
                        image.resizable().aspectRatio(contentMode: .fill)
                            .blur(radius: 40)
                    } placeholder: {
                        Rectangle().fill(Color(white: 0.08))
                    }
                    .frame(width: geo.size.width, height: 620)
                    .clipped()
                } else {
                    Rectangle()
                        .fill(Color(white: 0.08))
                        .frame(width: geo.size.width, height: 620)
                }

                // Bottom fade — ends in the page background colour so there's no visible
                // seam between the hero and the episode list below.
                VStack(spacing: 0) {
                    Color.clear
                    LinearGradient(
                        stops: [
                            .init(color: Self.pageBackground.opacity(0), location: 0),
                            .init(color: Self.pageBackground.opacity(0.75), location: 0.45),
                            .init(color: Self.pageBackground, location: 1.0)
                        ],
                        startPoint: .top, endPoint: .bottom
                    )
                    .frame(height: 350)
                }
                .frame(height: 620)

                // Left-side darkening for text contrast — keep it subtle so the artwork
                // still reads as hero imagery, not a wash.
                HStack(spacing: 0) {
                    LinearGradient(
                        colors: [.black.opacity(0.55), .clear],
                        startPoint: .leading, endPoint: .trailing
                    )
                    .frame(width: geo.size.width * 0.5)
                    Spacer()
                }
                .frame(height: 620)

                // Content overlay
                HStack(alignment: .bottom, spacing: 0) {
                    // Left: title + buttons
                    VStack(alignment: .leading, spacing: 20) {
                        Text(detail.title)
                            .font(.largeTitle)
                            .bold()

                        // Play button
                        if let next = nextEpisode, let ep = next.episode {
                            Button {
                                playerEpisode = PlayerInfo(episode: ep, resumePosition: ep.progress?.positionSecs ?? 0)
                            } label: {
                                HStack(spacing: 10) {
                                    Image(systemName: "play.fill")
                                    Text(playButtonLabel(reason: next.reason, episode: ep))
                                        .fontWeight(.semibold)
                                }
                                .padding(.horizontal, 28)
                                .padding(.vertical, 14)
                            }
                            .buttonStyle(.borderedProminent)
                        }
                    }
                    .frame(maxWidth: geo.size.width * 0.45, alignment: .leading)

                    Spacer(minLength: 40)

                    // Right: metadata + overview
                    VStack(alignment: .leading, spacing: 12) {
                        HStack(spacing: 16) {
                            if let rating = detail.rating {
                                Label(String(format: "%.1f", rating), systemImage: "star.fill")
                                    .foregroundStyle(.yellow)
                            }
                            if let year = detail.year {
                                Text(year)
                            }
                            if let genres = detail.genres {
                                Text(genres)
                                    .lineLimit(1)
                            }
                        }
                        .font(.subheadline)
                        .foregroundStyle(.secondary)

                        if let overview = detail.overview {
                            Text(overview)
                                .font(.callout)
                                .foregroundColor(.secondary)
                                .lineLimit(5)
                        }
                    }
                    .frame(maxWidth: geo.size.width * 0.45, alignment: .leading)
                }
                .padding(.horizontal, 60)
                .padding(.bottom, 48)
            }
        }
        .frame(height: 620)
    }

    // MARK: - Content Section

    @ViewBuilder
    private func contentSection(_ detail: SeriesDetail) -> some View {
        VStack(alignment: .leading, spacing: 40) {
            // Episode list
            VStack(alignment: .leading, spacing: 24) {
                Text("Episodes")
                    .font(.title3)
                    .bold()

                ForEach(detail.episodes) { episode in
                    Button {
                        Task { await playEpisode(episode) }
                    } label: {
                        EpisodeRow(episode: episode, client: client)
                    }
                    .buttonStyle(NoChromeFocusButtonStyle())
                }
            }
        }
        .padding(48)
    }

    // MARK: - Data Loading

    private func loadData() async {
        guard let client else { return }
        isLoading = detail == nil
        error = nil
        do {
            async let detailReq = client.getSeries(id: seriesId)
            async let nextReq = client.getNextEpisode(seriesId: seriesId)
            detail = try await detailReq
            nextEpisode = try await nextReq
        } catch let err as CastError {
            error = err
        } catch {
            self.error = .networkError(error.localizedDescription)
        }
        isLoading = false
    }

    // MARK: - Helpers

    private func playButtonLabel(reason: String, episode: EpisodeItem) -> String {
        switch reason {
        case "resume":
            let pos = episode.progress?.positionSecs ?? 0
            return "Resume \(episode.episodeLabel) at \(formatTime(pos))"
        case "next":
            return "Play \(episode.episodeLabel)"
        case "first":
            return "Start Watching"
        default:
            return "Play"
        }
    }

    private func playEpisode(_ episode: EpisodeItem) async {
        guard let client else { return }

        // Cap total wait at ~20 minutes (600 * 2s); well beyond any realistic remux time
        // but bounded so a server that stops responding mid-prepare can't strand the UI.
        let maxAttempts = 600
        var consecutiveFailures = 0
        let maxConsecutiveFailures = 5

        do {
            let status = try await client.prepareEpisode(episodeId: episode.id)
            if status.ready {
                playerEpisode = PlayerInfo(episode: episode, resumePosition: episode.progress?.positionSecs ?? 0)
                return
            }

            preparingEpisode = episode
            prepareProgress = status.progressPercent

            for _ in 0..<maxAttempts {
                try await Task.sleep(for: .seconds(2))
                // User cancelled or view went away
                if Task.isCancelled || preparingEpisode == nil { return }
                do {
                    let status = try await client.prepareEpisode(episodeId: episode.id)
                    consecutiveFailures = 0
                    prepareProgress = status.progressPercent
                    if status.ready {
                        preparingEpisode = nil
                        prepareProgress = nil
                        await loadData()
                        playerEpisode = PlayerInfo(episode: episode, resumePosition: episode.progress?.positionSecs ?? 0)
                        return
                    }
                } catch is CancellationError {
                    preparingEpisode = nil
                    prepareProgress = nil
                    return
                } catch {
                    consecutiveFailures += 1
                    if consecutiveFailures >= maxConsecutiveFailures {
                        preparingEpisode = nil
                        prepareProgress = nil
                        self.error = (error as? CastError) ?? .networkError("Lost connection while preparing episode. Try again.")
                        return
                    }
                }
            }
            // Timed out
            preparingEpisode = nil
            prepareProgress = nil
            self.error = .networkError("Preparation timed out. The server may be overloaded — try again shortly.")
        } catch is CancellationError {
            preparingEpisode = nil
            prepareProgress = nil
        } catch {
            // If the initial prepare call fails, try playing anyway (may be a pre-existing MP4)
            preparingEpisode = nil
            prepareProgress = nil
            playerEpisode = PlayerInfo(episode: episode, resumePosition: episode.progress?.positionSecs ?? 0)
        }
    }

    private func formatTime(_ seconds: Double) -> String {
        let mins = Int(seconds) / 60
        let secs = Int(seconds) % 60
        return String(format: "%d:%02d", mins, secs)
    }
}

// MARK: - Episode Row

private struct EpisodeRow: View {
    let episode: EpisodeItem
    let client: APIClient?
    @Environment(\.isFocused) private var isFocused

    var body: some View {
        HStack(spacing: 16) {
            // Thumbnail — prefer TMDB still, otherwise ask the server for a frame
            // (generated via ffmpeg on demand). `hasThumbnail` is *not* a gate:
            // the server lazily generates on first request, so gating on the cached
            // flag would prevent the first generation from ever happening for
            // episodes lacking a TMDB still. We always ask and let `AsyncImage`
            // show the placeholder while it waits (or if it 502s).
            ZStack(alignment: .bottom) {
                if let stillUrl = episode.stillUrl, let url = URL(string: stillUrl) {
                    AsyncImage(url: url) { image in
                        image.resizable().aspectRatio(16/9, contentMode: .fill)
                    } placeholder: {
                        thumbnailPlaceholder
                    }
                    .frame(width: 200, height: 112)
                    .clipped()
                    .cornerRadius(8)
                } else if let client {
                    AsyncImage(url: client.thumbnailURL(episodeId: episode.id)) { phase in
                        switch phase {
                        case .success(let image):
                            image.resizable().aspectRatio(16/9, contentMode: .fill)
                        default:
                            thumbnailPlaceholder
                        }
                    }
                    .frame(width: 200, height: 112)
                    .clipped()
                    .cornerRadius(8)
                } else {
                    thumbnailPlaceholder
                }

                // Progress bar overlay
                if let progress = episode.progress, !progress.completed, progress.fraction > 0 {
                    GeometryReader { geo in
                        VStack {
                            Spacer()
                            ZStack(alignment: .leading) {
                                Rectangle()
                                    .fill(Color.white.opacity(0.3))
                                    .frame(height: 3)
                                Rectangle()
                                    .fill(Color.white)
                                    .frame(width: geo.size.width * progress.fraction, height: 3)
                            }
                        }
                    }
                    .frame(width: 200, height: 112)
                    .cornerRadius(8)
                }
            }
            .frame(width: 200, height: 112)

            // Episode info
            VStack(alignment: .leading, spacing: 6) {
                Text(episode.episodeLabel)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)

                Text(episode.title)
                    .font(.headline)
                    .lineLimit(1)

                if let overview = episode.overview {
                    Text(overview)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }

                HStack(spacing: 12) {
                    if let runtime = episode.runtimeMinutes {
                        Text("\(runtime) min")
                    }
                    if let airDate = episode.airDate {
                        Text(airDate)
                    }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }

            Spacer()

            // Watch status
            episodeStatusIcon
        }
        .padding(.vertical, 14)
        .padding(.horizontal, 28)
        .background {
            RoundedRectangle(cornerRadius: 18)
                .fill(isFocused ? Color(white: 0.16) : Color(white: 0.10))
                .overlay(
                    RoundedRectangle(cornerRadius: 18)
                        .stroke(isFocused ? Color.white.opacity(0.35) : .clear, lineWidth: 1)
                )
        }
        .shadow(color: .black.opacity(isFocused ? 0.5 : 0), radius: isFocused ? 16 : 0, y: isFocused ? 8 : 0)
        .scaleEffect(isFocused ? 1.02 : 1.0)
        .animation(.easeInOut(duration: 0.15), value: isFocused)
    }

    @ViewBuilder
    private var episodeStatusIcon: some View {
        if let p = episode.progress {
            if p.completed {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(.green)
                    .font(.title3)
            } else {
                Image(systemName: "play.circle.fill")
                    .foregroundStyle(.blue)
                    .font(.title3)
            }
        }
    }

    private var thumbnailPlaceholder: some View {
        RoundedRectangle(cornerRadius: 8)
            .fill(Color(white: 0.15))
            .frame(width: 200, height: 112)
            .overlay {
                Text(episode.episodeLabel)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
    }
}
