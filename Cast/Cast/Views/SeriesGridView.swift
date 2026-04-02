import SwiftUI

struct SeriesGridView: View {
    @Environment(ServerConnection.self) private var connection
    @State private var seriesList: [SeriesListItem] = []
    @State private var continueWatchingItems: [ContinueWatchingItem] = []
    @State private var isLoading = true
    @State private var error: CastError?
    @State private var isRefreshing = false
    @State private var focusedSeriesId: String?
    @State private var activeBackdropId: String?
    @State private var backdropDebounce: Task<Void, Never>?

    private var client: APIClient? {
        guard let url = connection.baseURL else { return nil }
        return APIClient(baseURL: url)
    }

    private var backgroundURL: URL? {
        guard let client else { return nil }
        if let activeId = activeBackdropId,
           let series = seriesList.first(where: { $0.id == activeId }),
           series.hasBackdrop {
            return client.backdropURL(seriesId: activeId)
        }
        if let first = seriesList.first(where: { $0.hasBackdrop }) {
            return client.backdropURL(seriesId: first.id)
        }
        return nil
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
                    // Focus-reactive blurred backdrop
                    Color.black.ignoresSafeArea()

                    if let url = backgroundURL {
                        AsyncImage(url: url) { image in
                            image.resizable()
                                .aspectRatio(contentMode: .fill)
                                .blur(radius: 40)
                                .overlay(Color.black.opacity(0.55))
                        } placeholder: {
                            Color.clear
                        }
                        .ignoresSafeArea()
                        .id(url) // force new view per URL for crossfade
                        .transition(.opacity)
                    }

                    ScrollView {
                        VStack(alignment: .leading, spacing: 24) {
                            // Top bar
                            HStack(spacing: 20) {
                                Spacer()
                                Button {
                                    Task { await refreshMetadata() }
                                } label: {
                                    if isRefreshing {
                                        ProgressView()
                                    } else {
                                        Image(systemName: "arrow.triangle.2.circlepath")
                                    }
                                }
                                .disabled(isRefreshing)

                                Button {
                                    connection.disconnect()
                                } label: {
                                    Image(systemName: "server.rack")
                                }
                            }
                            .focusSection()
                            .padding(.horizontal, 80)

                            if !continueWatchingItems.isEmpty {
                                continueWatchingSection
                            }
                            librarySection
                        }
                        .padding(.top, 0)
                        .padding(.bottom, 80)
                    }
                }
                .animation(.easeInOut(duration: 2.0), value: backgroundURL)
            }
        }
        .navigationTitle("")
        .navigationBarHidden(true)
        .navigationDestination(for: SeriesListItem.self) { series in
            SeriesDetailView(seriesId: series.id, seriesTitle: series.title)
        }
        .task { await loadData() }
        .onChange(of: focusedSeriesId) { _, newId in
            backdropDebounce?.cancel()
            backdropDebounce = Task {
                try? await Task.sleep(for: .milliseconds(500))
                if !Task.isCancelled {
                    activeBackdropId = newId
                }
            }
        }
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

    private var librarySection: some View {
        LazyVGrid(
            columns: [GridItem(.adaptive(minimum: 260, maximum: 320), spacing: 50)],
            spacing: 60
        ) {
            ForEach(seriesList) { series in
                NavigationLink(value: series) {
                    SeriesCard(
                        series: series,
                        artURL: client?.artURL(seriesId: series.id),
                        focusedSeriesId: $focusedSeriesId
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
        do {
            try await client.fetchMetadata()
            await loadData()
        } catch {}
        isRefreshing = false
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
                    .font(.headline).lineLimit(1).shadow(radius: 4)
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
        .clipShape(RoundedRectangle(cornerRadius: 16))
        .padding(3)
        .overlay(
            RoundedRectangle(cornerRadius: 18)
                .stroke(isFocused ? Color.white.opacity(0.5) : Color.clear, lineWidth: 3)
        )
        .shadow(color: .black.opacity(isFocused ? 0.7 : 0.3), radius: isFocused ? 30 : 10, y: isFocused ? 15 : 5)
        .scaleEffect(isFocused ? 1.05 : 1.0)
        .animation(.easeInOut(duration: 0.2), value: isFocused)
    }
}

// MARK: - Series Card (reports focus to parent for reactive background)

private struct SeriesCard: View {
    let series: SeriesListItem
    let artURL: URL?
    @Binding var focusedSeriesId: String?
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
                    .font(.callout)
                    .lineLimit(1)
                    .foregroundStyle(isFocused ? .white : Color(white: 0.6))
                if let year = series.year {
                    Text(year)
                        .font(.caption2)
                        .foregroundStyle(Color(white: 0.4))
                }
            }
        }
        .scaleEffect(isFocused ? 1.08 : 1.0)
        .animation(.easeInOut(duration: 0.2), value: isFocused)
        .onChange(of: isFocused) { _, focused in
            if focused {
                focusedSeriesId = series.id
            }
        }
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
