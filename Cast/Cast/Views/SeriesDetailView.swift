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
                    // Full-page blurred backdrop background
                    if detail.hasBackdrop, let client {
                        AsyncImage(url: client.backdropURL(seriesId: detail.id)) { image in
                            image.resizable()
                                .aspectRatio(contentMode: .fill)
                                .blur(radius: 30)
                                .overlay(Color.black.opacity(0.4))
                        } placeholder: {
                            Color.black
                        }
                        .ignoresSafeArea()
                    } else {
                        Color.black.ignoresSafeArea()
                    }

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

                // Gradient overlays — bottom fade + left fade for text readability
                VStack(spacing: 0) {
                    Color.clear
                    LinearGradient(
                        stops: [
                            .init(color: .clear, location: 0),
                            .init(color: .black.opacity(0.7), location: 0.4),
                            .init(color: .black, location: 1.0)
                        ],
                        startPoint: .top, endPoint: .bottom
                    )
                    .frame(height: 350)
                }
                .frame(height: 620)

                // Left-side gradient for text contrast
                HStack(spacing: 0) {
                    LinearGradient(
                        colors: [.black.opacity(0.6), .clear],
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

        // Check if episode needs preparation
        do {
            let status = try await client.prepareEpisode(episodeId: episode.id)
            if status.ready {
                // Ready to play immediately
                playerEpisode = PlayerInfo(episode: episode, resumePosition: episode.progress?.positionSecs ?? 0)
                return
            }

            // Show preparing overlay and poll for progress
            preparingEpisode = episode
            prepareProgress = status.progressPercent

            while true {
                try await Task.sleep(for: .seconds(2))
                let status = try await client.prepareEpisode(episodeId: episode.id)
                prepareProgress = status.progressPercent
                if status.ready {
                    preparingEpisode = nil
                    prepareProgress = nil
                    // Reload data since the file changed
                    await loadData()
                    playerEpisode = PlayerInfo(episode: episode, resumePosition: episode.progress?.positionSecs ?? 0)
                    return
                }
            }
        } catch {
            // If prepare fails, try playing anyway
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
            // Thumbnail — prefer TMDB still, then server thumbnail, then placeholder
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
                } else if episode.hasThumbnail, let client {
                    AsyncImage(url: client.thumbnailURL(episodeId: episode.id)) { image in
                        image.resizable().aspectRatio(16/9, contentMode: .fill)
                    } placeholder: {
                        thumbnailPlaceholder
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
            if isFocused {
                RoundedRectangle(cornerRadius: 20)
                    .fill(.thinMaterial)
                    .brightness(0.1)
            }
        }
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
