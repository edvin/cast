import SwiftUI

/// Used as a navigation value when we want to push the Movies grid without
/// passing any specific movie.
struct MoviesDestination: Hashable {}

struct MoviesGridView: View {
    @Environment(ServerConnection.self) private var connection
    @State private var movies: [MovieListItem] = []
    @State private var isLoading = true
    @State private var error: CastError?
    @State private var showUnwatchedOnly = false

    private var client: APIClient? {
        guard let url = connection.baseURL else { return nil }
        return APIClient(baseURL: url)
    }

    var body: some View {
        Group {
            if let error {
                ErrorView(
                    title: "Unable to Load Movies",
                    message: error.errorDescription ?? "An unknown error occurred",
                    detail: error.detail,
                    onRetry: { self.error = nil; Task { await loadData() } }
                )
            } else if isLoading {
                ProgressView("Loading movies...")
            } else if movies.isEmpty {
                VStack(spacing: 20) {
                    Image(systemName: "film.stack")
                        .font(.system(size: 60))
                        .foregroundStyle(.secondary)
                    Text("No movies found")
                        .font(.title2)
                    Text("Add films to a folder named Movies or Films in your media directory.")
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 80)
                }
            } else {
                ZStack {
                    Color(white: 0.08).ignoresSafeArea()

                    ScrollView {
                        VStack(alignment: .leading, spacing: 24) {
                            header
                            grid
                                .padding(.bottom, 60)
                        }
                        .padding(.top, 20)
                        .padding(.bottom, 80)
                    }
                }
            }
        }
        .navigationBarHidden(true)
        .task { await loadData() }
    }

    // MARK: - Header

    private var header: some View {
        HStack(spacing: 20) {
            Text("Movies")
                .font(.largeTitle)
                .bold()
            Text("\(filteredMovies.count) of \(movies.count)")
                .font(.caption)
                .foregroundStyle(.secondary)
            Spacer()
            Button {
                showUnwatchedOnly.toggle()
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: showUnwatchedOnly ? "eye.slash" : "eye")
                    Text(showUnwatchedOnly ? "Show All" : "Unwatched Only")
                        .font(.caption)
                }
            }
        }
        .padding(.horizontal, 80)
    }

    // MARK: - Grid

    private var filteredMovies: [MovieListItem] {
        if showUnwatchedOnly {
            return movies.filter { ($0.progress?.completed ?? false) == false }
        }
        return movies
    }

    private var grid: some View {
        LazyVGrid(
            columns: [GridItem(.adaptive(minimum: 260, maximum: 320), spacing: 50)],
            spacing: 60
        ) {
            ForEach(filteredMovies) { movie in
                NavigationLink(value: movie) {
                    MoviePosterCard(movie: movie, client: client)
                }
                .buttonStyle(NoChromeFocusButtonStyle())
            }
        }
        .padding(.horizontal, 80)
    }

    // MARK: - Data

    private func loadData() async {
        guard let client else { return }
        isLoading = movies.isEmpty
        error = nil
        do {
            movies = try await client.listMovies()
        } catch let err as CastError {
            error = err
        } catch {
            self.error = .networkError(error.localizedDescription)
        }
        isLoading = false
    }
}

// MARK: - Poster card

private struct MoviePosterCard: View {
    let movie: MovieListItem
    let client: APIClient?
    @Environment(\.isFocused) private var isFocused

    var body: some View {
        VStack(spacing: 14) {
            ZStack(alignment: .bottomLeading) {
                if movie.hasArt, let client {
                    AsyncImage(url: client.movieArtURL(movieId: movie.id)) { phase in
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

                if let progress = movie.progress, !progress.completed, progress.fraction > 0 {
                    GeometryReader { geo in
                        VStack { Spacer()
                            ZStack(alignment: .leading) {
                                Rectangle().fill(Color.white.opacity(0.25)).frame(height: 4)
                                Rectangle().fill(Color.white)
                                    .frame(width: geo.size.width * progress.fraction, height: 4)
                            }
                        }
                    }
                    .cornerRadius(12)
                }

                if let progress = movie.progress, progress.completed {
                    Image(systemName: "checkmark.circle.fill")
                        .font(.title3)
                        .foregroundStyle(.green)
                        .padding(10)
                }
            }
            .overlay(
                RoundedRectangle(cornerRadius: 12)
                    .stroke(isFocused ? Color.white.opacity(0.5) : Color.clear, lineWidth: 2)
            )
            .shadow(color: .black.opacity(isFocused ? 0.7 : 0.2), radius: isFocused ? 24 : 6, y: isFocused ? 12 : 3)

            VStack(spacing: 3) {
                Text(movie.title)
                    .font(.caption)
                    .lineLimit(1)
                    .foregroundStyle(isFocused ? .white : Color(white: 0.8))
                if let year = movie.year {
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
            Image(systemName: "film")
                .font(.system(size: 40))
                .foregroundColor(.white.opacity(0.2))
        }
        .aspectRatio(2/3, contentMode: .fit)
        .clipShape(RoundedRectangle(cornerRadius: 12))
    }
}
