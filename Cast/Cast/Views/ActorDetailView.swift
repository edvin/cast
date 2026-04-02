import SwiftUI

struct ActorDetailView: View {
    let personId: Int
    let client: APIClient

    @State private var person: PersonDetail?
    @State private var isLoading = true
    @State private var error: CastError?

    var body: some View {
        Group {
            if let error {
                ErrorView(
                    title: "Unable to Load",
                    message: error.errorDescription ?? "An unknown error occurred",
                    detail: error.detail,
                    onRetry: { self.error = nil; Task { await loadPerson() } }
                )
            } else if isLoading {
                ProgressView("Loading...")
            } else if let person {
                VStack(spacing: 0) {
                    heroSection(person)
                    ScrollView {
                        filmographySection(person)
                    }
                }
            }
        }
        .background(Color.black.ignoresSafeArea())
        .task { await loadPerson() }
    }

    // MARK: - Hero

    @ViewBuilder
    private func heroSection(_ person: PersonDetail) -> some View {
        HStack(alignment: .top, spacing: 48) {
            // Large profile photo
            if let urlString = person.profileUrl, let url = URL(string: urlString) {
                AsyncImage(url: url) { phase in
                    switch phase {
                    case .success(let image):
                        image.resizable().aspectRatio(contentMode: .fill)
                    case .failure:
                        profilePlaceholder
                    case .empty:
                        ProgressView()
                            .frame(width: 300, height: 400)
                    @unknown default:
                        profilePlaceholder
                    }
                }
                .frame(width: 200, height: 270)
                .clipShape(RoundedRectangle(cornerRadius: 16))
                .shadow(color: .black.opacity(0.5), radius: 15, y: 8)
            } else {
                profilePlaceholder
            }

            // Bio section
            VStack(alignment: .leading, spacing: 10) {
                Text(person.name)
                    .font(.title2)
                    .bold()

                HStack(spacing: 16) {
                    if let birthday = person.birthday {
                        Label(formatDate(birthday), systemImage: "calendar")
                    }
                    if let place = person.placeOfBirth {
                        Label(place, systemImage: "mappin")
                    }
                    if let deathday = person.deathday {
                        Label(formatDate(deathday), systemImage: "heart.slash")
                    }
                }
                .font(.caption)
                .foregroundStyle(.secondary)

                if let bio = person.biography {
                    Text(bio)
                        .font(.caption)
                        .foregroundStyle(Color(white: 0.75))
                        .lineLimit(5)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 60)
        .padding(.top, 30)
        .padding(.bottom, 16)
    }

    // MARK: - Filmography

    @ViewBuilder
    private func filmographySection(_ person: PersonDetail) -> some View {
        if !person.knownFor.isEmpty {
            VStack(alignment: .leading, spacing: 30) {
                Text("Filmography")
                    .font(.title3)
                    .bold()
                    .padding(.horizontal, 60)

                LazyVGrid(
                    columns: [GridItem(.adaptive(minimum: 240, maximum: 300), spacing: 40)],
                    spacing: 44
                ) {
                    ForEach(person.knownFor) { role in
                        Button {} label: {
                            FilmographyCard(role: role)
                        }
                        .buttonStyle(NoChromeFocusButtonStyle())
                    }
                }
                .padding(.horizontal, 60)
            }
            .padding(.bottom, 60)
        }
    }

    private var profilePlaceholder: some View {
        RoundedRectangle(cornerRadius: 16)
            .fill(Color(white: 0.15))
            .frame(width: 200, height: 270)
            .overlay {
                Image(systemName: "person.fill")
                    .font(.system(size: 50))
                    .foregroundStyle(Color(white: 0.3))
            }
    }

    private func formatDate(_ dateString: String) -> String {
        // Convert "1982-12-15" to something readable
        let parts = dateString.split(separator: "-")
        guard parts.count == 3 else { return dateString }
        let months = ["", "Jan", "Feb", "Mar", "Apr", "May", "Jun",
                      "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
        if let month = Int(parts[1]), month >= 1, month <= 12 {
            return "\(months[month]) \(parts[2]), \(parts[0])"
        }
        return dateString
    }

    private func loadPerson() async {
        isLoading = true
        error = nil
        do {
            person = try await client.getPersonDetail(personId: personId)
        } catch let err as CastError {
            error = err
        } catch {
            self.error = .networkError(error.localizedDescription)
        }
        isLoading = false
    }
}

// MARK: - Filmography Card

private struct FilmographyCard: View {
    let role: CreditRole
    @Environment(\.isFocused) private var isFocused

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            // Poster
            if let urlString = role.posterUrl, let url = URL(string: urlString) {
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
                .frame(width: 220, height: 330)
                .clipped()
                .cornerRadius(10)
            } else {
                posterPlaceholder
            }

            VStack(alignment: .leading, spacing: 3) {
                Text(role.title)
                    .font(.caption)
                    .fontWeight(.medium)
                    .lineLimit(2)
                    .foregroundStyle(isFocused ? .white : Color(white: 0.7))

                if let character = role.character, !character.isEmpty {
                    Text(character)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }

                HStack(spacing: 6) {
                    if let year = role.year {
                        Text(year)
                    }
                    Text(role.mediaType == "tv" ? "TV" : "Film")
                        .padding(.horizontal, 5)
                        .padding(.vertical, 1)
                        .background(Color(white: 0.2))
                        .cornerRadius(3)
                    if let rating = role.rating, rating > 0 {
                        Label(String(format: "%.1f", rating), systemImage: "star.fill")
                            .foregroundStyle(.yellow)
                    }
                }
                .font(.caption2)
                .foregroundStyle(.secondary)
            }
        }
        .frame(width: 220)
        .scaleEffect(isFocused ? 1.08 : 1.0)
        .animation(.easeInOut(duration: 0.2), value: isFocused)
    }

    private var posterPlaceholder: some View {
        RoundedRectangle(cornerRadius: 10)
            .fill(Color(white: 0.12))
            .frame(width: 140, height: 210)
            .overlay {
                Image(systemName: "film")
                    .font(.title2)
                    .foregroundStyle(Color(white: 0.3))
            }
    }
}
