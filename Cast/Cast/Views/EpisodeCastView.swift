import SwiftUI

struct EpisodeCastView: View {
    let episode: EpisodeItem
    let client: APIClient

    @State private var credits: EpisodeCredits?
    @State private var isLoading = true
    @State private var error: CastError?

    var body: some View {
        Group {
            if let error {
                ErrorView(
                    title: "Unable to Load Cast",
                    message: error.errorDescription ?? "An unknown error occurred",
                    detail: error.detail,
                    onRetry: { self.error = nil; Task { await loadCredits() } }
                )
            } else if isLoading {
                ProgressView("Loading cast...")
            } else if let credits {
                ScrollView {
                    VStack(alignment: .leading, spacing: 48) {
                        // Episode header
                        VStack(alignment: .leading, spacing: 8) {
                            Text(episode.episodeLabel)
                                .font(.headline)
                                .foregroundStyle(.secondary)
                            Text(episode.title)
                                .font(.title)
                                .bold()
                        }
                        .padding(.horizontal, 80)

                        // Main cast
                        if !credits.cast.isEmpty {
                            castGrid(title: "Cast", members: credits.cast)
                        }

                        // Guest stars
                        if !credits.guestStars.isEmpty {
                            castGrid(title: "Guest Stars", members: credits.guestStars)
                        }

                        if credits.cast.isEmpty && credits.guestStars.isEmpty {
                            Text("No cast information available.")
                                .foregroundStyle(.secondary)
                                .frame(maxWidth: .infinity)
                                .padding(.top, 40)
                        }
                    }
                    .padding(.vertical, 60)
                }
            }
        }
        .navigationTitle("Cast & Crew")
        .background(Color.black.ignoresSafeArea())
        .task { await loadCredits() }
    }

    @ViewBuilder
    private func castGrid(title: String, members: [CastMember]) -> some View {
        VStack(alignment: .leading, spacing: 30) {
            Text(title)
                .font(.title2)
                .bold()
                .padding(.horizontal, 80)

            LazyVGrid(
                columns: [GridItem(.adaptive(minimum: 180, maximum: 220), spacing: 40)],
                spacing: 50
            ) {
                ForEach(members) { member in
                    NavigationLink(value: member.tmdbId) {
                        CastCard(member: member)
                    }
                    .buttonStyle(NoChromeFocusButtonStyle())
                }
            }
            .padding(.horizontal, 80)
        }
    }

    private func loadCredits() async {
        isLoading = true
        error = nil
        do {
            credits = try await client.getEpisodeCredits(episodeId: episode.id)
        } catch let err as CastError {
            error = err
        } catch {
            self.error = .networkError(error.localizedDescription)
        }
        isLoading = false
    }
}

private struct CastCard: View {
    let member: CastMember
    @Environment(\.isFocused) private var isFocused

    var body: some View {
        VStack(spacing: 12) {
            if let urlString = member.profileUrl, let url = URL(string: urlString) {
                AsyncImage(url: url) { phase in
                    switch phase {
                    case .success(let image):
                        image.resizable().aspectRatio(contentMode: .fill)
                    case .failure:
                        profilePlaceholder
                    case .empty:
                        ProgressView()
                            .frame(width: 150, height: 150)
                    @unknown default:
                        profilePlaceholder
                    }
                }
                .frame(width: 150, height: 150)
                .clipShape(Circle())
                .overlay(
                    Circle()
                        .stroke(isFocused ? Color.white.opacity(0.8) : Color.clear, lineWidth: 3)
                )
            } else {
                profilePlaceholder
            }

            VStack(spacing: 4) {
                Text(member.name)
                    .font(.callout)
                    .fontWeight(.medium)
                    .lineLimit(2)
                    .multilineTextAlignment(.center)
                    .foregroundStyle(isFocused ? .white : Color(white: 0.8))

                if let character = member.character, !character.isEmpty {
                    Text(character)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .multilineTextAlignment(.center)
                }
            }
        }
        .frame(width: 180)
        .padding(.vertical, 16)
        .padding(.horizontal, 8)
        .background {
            if isFocused {
                RoundedRectangle(cornerRadius: 20)
                    .fill(.thinMaterial)
                    .brightness(0.1)
            }
        }
        .scaleEffect(isFocused ? 1.1 : 1.0)
        .animation(.easeInOut(duration: 0.2), value: isFocused)
    }

    private var profilePlaceholder: some View {
        Circle()
            .fill(Color(white: 0.2))
            .frame(width: 150, height: 150)
            .overlay {
                Image(systemName: "person.fill")
                    .font(.system(size: 44))
                    .foregroundStyle(.secondary)
            }
    }
}
