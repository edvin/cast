import SwiftUI

struct SeriesGridView: View {
    @Environment(ServerConnection.self) private var connection
    @State private var seriesList: [SeriesListItem] = []
    @State private var continueWatchingItems: [ContinueWatchingItem] = []
    @State private var isLoading = true
    @State private var error: CastError?
    @State private var isRefreshing = false
    @State private var showUnwatchedOnly = false

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
                ProgressView("Loading series...")
            } else if seriesList.isEmpty {
                VStack(spacing: 20) {
                    Image(systemName: "tv")
                        .font(.system(size: 60))
                        .foregroundStyle(.secondary)
                    Text("No series found")
                        .font(.title2)
                    Text("Add media folders to your Cast server.")
                        .foregroundStyle(.secondary)
                }
            } else {
                ZStack {
                    Color(white: 0.08).ignoresSafeArea()

                    ScrollView {
                        VStack(alignment: .leading, spacing: 24) {
                            if !continueWatchingItems.isEmpty {
                                continueWatchingSection
                            }

                            librarySection
                                .padding(.bottom, 40)

                            // Actions at the bottom
                            HStack(spacing: 20) {
                                Button {
                                    showUnwatchedOnly.toggle()
                                } label: {
                                    HStack(spacing: 6) {
                                        Image(systemName: showUnwatchedOnly ? "eye.slash" : "eye")
                                        Text(showUnwatchedOnly ? "Show All" : "Unwatched Only")
                                            .font(.caption)
                                    }
                                }

                                Spacer()

                                Button {
                                    Task { await refreshMetadata() }
                                } label: {
                                    HStack(spacing: 8) {
                                        Image(systemName: "arrow.triangle.2.circlepath")
                                        Text(isRefreshing ? "Refreshing..." : "Refresh Metadata")
                                            .font(.caption)
                                    }
                                }
                                .disabled(isRefreshing)

                                Button {
                                    connection.disconnect()
                                } label: {
                                    HStack(spacing: 8) {
                                        Image(systemName: "server.rack")
                                        Text("Change Server")
                                            .font(.caption)
                                    }
                                }
                                Spacer()
                            }
                            .padding(.horizontal, 80)
                        }
                        .padding(.top, 20)
                        .padding(.bottom, 80)
                    }
                }
            }
        }
        .navigationTitle("")
        .navigationBarHidden(true)
        .navigationDestination(for: SeriesListItem.self) { series in
            SeriesDetailView(seriesId: series.id, seriesTitle: series.title)
        }
        .task { await loadData() }
    }

    // MARK: - Continue Watching

    private var continueWatchingSection: some View {
        VStack(alignment: .leading, spacing: 24) {
            Text("Continue Watching")
                .font(.title3)
                .bold()
                .padding(.leading, 80)

            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 40) {
                    ForEach(continueWatchingItems) { item in
                        NavigationLink(value: SeriesListItem.from(continueWatching: item)) {
                            ContinueWatchingCard(item: item, client: client)
                        }
                        .buttonStyle(NoChromeFocusButtonStyle())
                    }
                }
                .padding(.horizontal, 80)
            }
        }
    }

    // MARK: - Library

    private var filteredSeries: [SeriesListItem] {
        if showUnwatchedOnly {
            return seriesList.filter { $0.watchedCount < $0.totalCount }
        }
        return seriesList
    }

    private var librarySection: some View {
        LazyVGrid(
            columns: [GridItem(.adaptive(minimum: 300, maximum: 380), spacing: 50)],
            spacing: 60
        ) {
            ForEach(filteredSeries) { series in
                NavigationLink(value: series) {
                    SeriesCard(
                        series: series,
                        artURL: client?.artURL(seriesId: series.id)
                    )
                }
                .buttonStyle(NoChromeFocusButtonStyle())
            }
        }
        .padding(.horizontal, 80)
    }

    // MARK: - Data Loading

    private func loadData() async {
        guard let client else { return }
        isLoading = seriesList.isEmpty
        error = nil
        do {
            async let seriesReq = client.listSeries()
            async let continueReq = client.continueWatching()
            seriesList = try await seriesReq
            continueWatchingItems = (try? await continueReq) ?? []
        } catch let err as CastError {
            error = err
        } catch {
            self.error = .networkError(error.localizedDescription)
        }
        isLoading = false
    }

    private func refreshMetadata() async {
        guard let client else { return }
        isRefreshing = true
        defer { isRefreshing = false }
        do {
            try await client.fetchMetadata()
            await loadData()
        } catch let err as CastError {
            self.error = err
        } catch {
            self.error = .networkError(error.localizedDescription)
        }
    }
}

// MARK: - No-chrome button style

struct NoChromeFocusButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .opacity(configuration.isPressed ? 0.7 : 1.0)
    }
}

// MARK: - Continue Watching Card

private struct ContinueWatchingCard: View {
    let item: ContinueWatchingItem
    let client: APIClient?
    @Environment(\.isFocused) private var isFocused

    var body: some View {
        // Outer ZStack — NOT clipped — holds the border overlay
        ZStack {
            // Inner content — clipped to rounded rect
            ZStack(alignment: .bottomLeading) {
                if let client {
                    let imageURL = item.hasBackdrop
                        ? client.backdropURL(seriesId: item.seriesId)
                        : client.artURL(seriesId: item.seriesId)
                    AsyncImage(url: imageURL) { image in
                        image.resizable().aspectRatio(contentMode: .fill)
                    } placeholder: {
                        Rectangle().fill(Color(white: 0.12))
                    }
                    .frame(width: 500, height: 280)
                    .clipped()
                }

                LinearGradient(
                    stops: [
                        .init(color: .clear, location: 0.3),
                        .init(color: .black.opacity(0.95), location: 1.0)
                    ],
                    startPoint: .top, endPoint: .bottom
                )
                .frame(width: 500, height: 280)

                VStack(alignment: .leading, spacing: 6) {
                    Text(item.seriesTitle)
                        .font(.subheadline).lineLimit(1).shadow(radius: 4)
                    HStack(spacing: 8) {
                        Text(item.nextEpisode.episodeLabel)
                            .font(.caption).foregroundStyle(Color(white: 0.7))
                        if item.reason == "resume", let p = item.nextEpisode.progress {
                            Text("·").foregroundStyle(Color(white: 0.5))
                            Text("\(Int(p.fraction * 100))%")
                                .font(.caption).foregroundStyle(Color(white: 0.7))
                        }
                    }
                    if let progress = item.nextEpisode.progress, !progress.completed {
                        GeometryReader { geo in
                            ZStack(alignment: .leading) {
                                Capsule().fill(Color.white.opacity(0.2)).frame(height: 4)
                                Capsule().fill(Color.white)
                                    .frame(width: geo.size.width * progress.fraction, height: 4)
                            }
                        }
                        .frame(width: 200, height: 4)
                    }
                }
                .padding(24)
            }
            .frame(width: 500, height: 280)
            .clipped()
            .cornerRadius(16)
        }
        .shadow(color: .black.opacity(isFocused ? 0.7 : 0.3), radius: isFocused ? 30 : 10, y: isFocused ? 15 : 5)
        .scaleEffect(isFocused ? 1.05 : 1.0)
        .animation(.easeInOut(duration: 0.2), value: isFocused)
    }
}

// MARK: - Series Card (reports focus to parent for reactive background)

private struct SeriesCard: View {
    let series: SeriesListItem
    let artURL: URL?
    @Environment(\.isFocused) private var isFocused

    var body: some View {
        VStack(spacing: 14) {
            ZStack {
                if series.hasArt, let url = artURL {
                    AsyncImage(url: url) { phase in
                        switch phase {
                        case .success(let image):
                            image.resizable().aspectRatio(2/3, contentMode: .fill)
                        case .failure:
                            posterPlaceholder
                        case .empty:
                            Rectangle().fill(Color(white: 0.1))
                                .aspectRatio(2/3, contentMode: .fill)
                                .overlay(ProgressView())
                        @unknown default:
                            posterPlaceholder
                        }
                    }
                    .aspectRatio(2/3, contentMode: .fit)
                    .clipped()
                    .cornerRadius(12)
                } else {
                    posterPlaceholder
                }
            }
            .overlay(
                RoundedRectangle(cornerRadius: 12)
                    .stroke(isFocused ? Color.white.opacity(0.5) : Color.clear, lineWidth: 2)
            )
            .shadow(color: .black.opacity(isFocused ? 0.7 : 0.2), radius: isFocused ? 24 : 6, y: isFocused ? 12 : 3)

            VStack(spacing: 3) {
                Text(series.title)
                    .font(.caption)
                    .lineLimit(1)
                    .foregroundStyle(isFocused ? .white : Color(white: 0.8))
                if let year = series.year {
                    Text(year)
                        .font(.caption2)
                        .foregroundStyle(Color(white: 0.4))
                }
            }
        }
        .scaleEffect(isFocused ? 1.05 : 1.0)
        .animation(.interactiveSpring(response: 0.3, dampingFraction: 0.8), value: isFocused)
    }

    private var posterPlaceholder: some View {
        ZStack {
            LinearGradient(
                colors: [Color(white: 0.15), Color(white: 0.08)],
                startPoint: .topLeading, endPoint: .bottomTrailing
            )
            Text(String(series.title.prefix(1)))
                .font(.system(size: 60, weight: .bold))
                .foregroundColor(.white.opacity(0.2))
        }
        .aspectRatio(2/3, contentMode: .fit)
        .clipShape(RoundedRectangle(cornerRadius: 12))
    }
}

// MARK: - Helper

extension SeriesListItem {
    static func from(continueWatching item: ContinueWatchingItem) -> SeriesListItem {
        SeriesListItem(
            id: item.seriesId, title: item.seriesTitle, episodeCount: 0,
            hasArt: item.hasArt, hasBackdrop: item.hasBackdrop, hasMetadata: false,
            overview: nil, genres: nil, rating: nil, year: nil,
            watchedCount: 0, totalCount: 0
        )
    }
}
