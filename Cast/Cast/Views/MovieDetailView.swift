import SwiftUI

struct MoviePlayerInfo: Identifiable {
    let id = UUID()
    let movie: MovieDetail
    let resumePosition: Double
}

struct MovieDetailView: View {
    let movieId: String

    @Environment(ServerConnection.self) private var connection
    @State private var detail: MovieDetail?
    @State private var isLoading = true
    @State private var error: CastError?
    @State private var playerInfo: MoviePlayerInfo?
    @State private var preparing = false
    @State private var prepareProgress: Int?

    /// Same dark surface the Series detail uses, so scrolling past the hero reveals a
    /// clean background (no blur).
    static let pageBackground = Color(white: 0.06)

    private var client: APIClient? {
        guard let url = connection.baseURL else { return nil }
        return APIClient(baseURL: url)
    }

    var body: some View {
        Group {
            if let error {
                ErrorView(
                    title: "Unable to Load Movie",
                    message: error.errorDescription ?? "An unknown error occurred",
                    detail: error.detail,
                    onRetry: { self.error = nil; Task { await loadData() } }
                )
            } else if isLoading {
                ProgressView()
            } else if let detail {
                ZStack {
                    Self.pageBackground.ignoresSafeArea()

                    ScrollView {
                        VStack(alignment: .leading, spacing: 0) {
                            hero(detail)
                            meta(detail)
                        }
                    }
                    .ignoresSafeArea(edges: [.top, .horizontal])
                }
            }
        }
        .navigationBarHidden(true)
        .fullScreenCover(item: $playerInfo) {
            Task { await loadData() }
        } content: { info in
            if let client {
                MoviePlayerContainerView(
                    client: client,
                    movie: info.movie,
                    resumePosition: info.resumePosition
                )
            }
        }
        .overlay {
            if preparing {
                ZStack {
                    Color.black.opacity(0.85).ignoresSafeArea()
                    VStack(spacing: 20) {
                        ProgressView().scaleEffect(1.5)
                        Text("Preparing \(detail?.title ?? "movie")...")
                            .font(.title3)
                        if let progress = prepareProgress {
                            Text("\(progress)%")
                                .font(.headline)
                                .foregroundStyle(.secondary)
                            ProgressView(value: Double(progress), total: 100)
                                .frame(width: 300).tint(.white)
                        }
                        Text("Converting for Apple TV playback")
                            .font(.caption)
                            .foregroundStyle(Color(white: 0.5))
                        Button("Cancel") {
                            preparing = false
                            prepareProgress = nil
                        }
                        .padding(.top, 12)
                    }
                }
            }
        }
        .task { await loadData() }
    }

    // MARK: - Hero

    @ViewBuilder
    private func hero(_ detail: MovieDetail) -> some View {
        GeometryReader { geo in
            ZStack(alignment: .bottom) {
                if detail.hasBackdrop, let client {
                    AsyncImage(url: client.movieBackdropURL(movieId: detail.id)) { image in
                        image.resizable().aspectRatio(contentMode: .fill)
                    } placeholder: {
                        Rectangle().fill(Color(white: 0.08))
                    }
                    .frame(width: geo.size.width, height: 620, alignment: .top)
                    .clipped()
                } else if detail.hasArt, let client {
                    AsyncImage(url: client.movieArtURL(movieId: detail.id)) { image in
                        image.resizable().aspectRatio(contentMode: .fill).blur(radius: 40)
                    } placeholder: {
                        Rectangle().fill(Color(white: 0.08))
                    }
                    .frame(width: geo.size.width, height: 620)
                    .clipped()
                } else {
                    Rectangle().fill(Color(white: 0.08))
                        .frame(width: geo.size.width, height: 620)
                }

                // Bottom fade into the page bg
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

                HStack(spacing: 0) {
                    LinearGradient(
                        colors: [.black.opacity(0.55), .clear],
                        startPoint: .leading, endPoint: .trailing
                    )
                    .frame(width: geo.size.width * 0.5)
                    Spacer()
                }
                .frame(height: 620)

                HStack(alignment: .bottom, spacing: 0) {
                    VStack(alignment: .leading, spacing: 20) {
                        Text(detail.title)
                            .font(.largeTitle).bold()
                        if let tagline = detail.tagline, !tagline.isEmpty {
                            Text(tagline)
                                .font(.title3)
                                .italic()
                                .foregroundStyle(Color(white: 0.8))
                        }
                        Button {
                            Task { await playMovie(detail) }
                        } label: {
                            HStack(spacing: 10) {
                                Image(systemName: "play.fill")
                                Text(playButtonLabel(detail))
                                    .fontWeight(.semibold)
                            }
                            .padding(.horizontal, 28)
                            .padding(.vertical, 14)
                        }
                        .buttonStyle(.borderedProminent)
                    }
                    .frame(maxWidth: geo.size.width * 0.45, alignment: .leading)

                    Spacer(minLength: 40)

                    VStack(alignment: .leading, spacing: 12) {
                        HStack(spacing: 16) {
                            if let rating = detail.rating {
                                Label(String(format: "%.1f", rating), systemImage: "star.fill")
                                    .foregroundStyle(.yellow)
                            }
                            if let year = detail.year {
                                Text(year)
                            }
                            if let runtime = detail.runtimeMinutes {
                                Text("\(runtime) min")
                            }
                            if let genres = detail.genres {
                                Text(genres).lineLimit(1)
                            }
                        }
                        .font(.subheadline)
                        .foregroundStyle(.secondary)

                        if let overview = detail.overview {
                            Text(overview)
                                .font(.callout)
                                .foregroundColor(.secondary)
                                .lineLimit(6)
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

    // MARK: - Meta block

    @ViewBuilder
    private func meta(_ detail: MovieDetail) -> some View {
        VStack(alignment: .leading, spacing: 18) {
            if let progress = detail.progress, !progress.completed, progress.fraction > 0 {
                HStack(spacing: 12) {
                    Image(systemName: "play.circle.fill").foregroundStyle(.blue)
                    Text("Resumes at \(formatTime(progress.positionSecs)) of \(formatTime(progress.durationSecs))")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
            }
            if detail.progress?.completed == true {
                HStack(spacing: 12) {
                    Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
                    Text("Watched").font(.callout).foregroundStyle(.secondary)
                }
            }
        }
        .padding(.horizontal, 60)
        .padding(.bottom, 60)
    }

    // MARK: - Actions

    private func playButtonLabel(_ detail: MovieDetail) -> String {
        if let p = detail.progress, !p.completed, p.positionSecs > 30 {
            return "Resume at \(formatTime(p.positionSecs))"
        }
        return "Play"
    }

    private func playMovie(_ movie: MovieDetail) async {
        guard let client else { return }
        do {
            let status = try await client.prepareMovie(movieId: movie.id)
            if status.ready {
                playerInfo = MoviePlayerInfo(movie: movie, resumePosition: movie.progress?.positionSecs ?? 0)
                return
            }
            preparing = true
            prepareProgress = status.progressPercent

            let maxAttempts = 600
            var consecutiveFailures = 0
            for _ in 0..<maxAttempts {
                try await Task.sleep(for: .seconds(2))
                if Task.isCancelled || !preparing { return }
                do {
                    let s = try await client.prepareMovie(movieId: movie.id)
                    consecutiveFailures = 0
                    prepareProgress = s.progressPercent
                    if s.ready {
                        preparing = false
                        prepareProgress = nil
                        await loadData()
                        let refreshed = detail ?? movie
                        playerInfo = MoviePlayerInfo(movie: refreshed, resumePosition: refreshed.progress?.positionSecs ?? 0)
                        return
                    }
                } catch is CancellationError {
                    preparing = false
                    prepareProgress = nil
                    return
                } catch {
                    consecutiveFailures += 1
                    if consecutiveFailures >= 5 {
                        preparing = false
                        prepareProgress = nil
                        self.error = (error as? CastError) ?? .networkError("Lost connection while preparing movie.")
                        return
                    }
                }
            }
            preparing = false
            prepareProgress = nil
            self.error = .networkError("Preparation timed out.")
        } catch is CancellationError {
            preparing = false
            prepareProgress = nil
        } catch {
            preparing = false
            prepareProgress = nil
            playerInfo = MoviePlayerInfo(movie: movie, resumePosition: movie.progress?.positionSecs ?? 0)
        }
    }

    private func loadData() async {
        guard let client else { return }
        isLoading = detail == nil
        error = nil
        do {
            detail = try await client.getMovie(id: movieId)
        } catch let err as CastError {
            error = err
        } catch {
            self.error = .networkError(error.localizedDescription)
        }
        isLoading = false
    }

    private func formatTime(_ seconds: Double) -> String {
        let h = Int(seconds) / 3600
        let m = (Int(seconds) % 3600) / 60
        let s = Int(seconds) % 60
        if h > 0 { return String(format: "%d:%02d:%02d", h, m, s) }
        return String(format: "%d:%02d", m, s)
    }
}
