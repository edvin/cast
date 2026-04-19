import Foundation

enum CastError: LocalizedError {
    case serverError(ApiError)
    case networkError(String)
    case decodingError(String)

    var errorDescription: String? {
        switch self {
        case .serverError(let apiError):
            return apiError.error
        case .networkError(let msg):
            return msg
        case .decodingError(let msg):
            return "Data error: \(msg)"
        }
    }

    var detail: String? {
        switch self {
        case .serverError(let apiError):
            return apiError.detail
        default:
            return nil
        }
    }
}

struct APIClient {
    let baseURL: URL

    // MARK: - URL Building

    /// Build an API URL by appending percent-encoded path components.
    /// The server is single-user and paths only contain UUIDs/language codes, but encoding
    /// guards against any server-generated IDs that may include characters needing escaping.
    private func url(_ components: String...) -> URL {
        var u = baseURL
        for c in components {
            u = u.appendingPathComponent(c)
        }
        return u
    }

    private func requestURL(_ url: URL, method: String = "GET") -> URLRequest {
        var req = URLRequest(url: url)
        req.httpMethod = method
        return req
    }

    private func request<T: Decodable>(_ url: URL) async throws -> T {
        let (data, response) = try await URLSession.shared.data(from: url)
        guard let http = response as? HTTPURLResponse else {
            throw CastError.networkError("Invalid response")
        }
        if http.statusCode >= 400 {
            if let apiError = try? JSONDecoder().decode(ApiError.self, from: data) {
                throw CastError.serverError(apiError)
            }
            throw CastError.networkError("Server returned \(http.statusCode)")
        }
        do {
            return try JSONDecoder().decode(T.self, from: data)
        } catch {
            throw CastError.decodingError(error.localizedDescription)
        }
    }

    // MARK: - Continue Watching

    func continueWatching() async throws -> [ContinueWatchingItem] {
        try await request(url("api", "continue-watching"))
    }

    // MARK: - Series

    func listSeries() async throws -> [SeriesListItem] {
        try await request(url("api", "series"))
    }

    func getSeries(id: String) async throws -> SeriesDetail {
        try await request(url("api", "series", id))
    }

    func getNextEpisode(seriesId: String) async throws -> NextEpisodeResponse {
        try await request(url("api", "series", seriesId, "next"))
    }

    func artURL(seriesId: String) -> URL {
        url("api", "series", seriesId, "art")
    }

    func backdropURL(seriesId: String) -> URL {
        url("api", "series", seriesId, "backdrop")
    }

    func fetchMetadata() async throws {
        let req = requestURL(url("api", "metadata", "fetch"), method: "POST")
        let _ = try await URLSession.shared.data(for: req)
    }

    // MARK: - Episodes

    func streamURL(episodeId: String) -> URL {
        url("api", "episodes", episodeId, "stream")
    }

    func thumbnailURL(episodeId: String) -> URL {
        url("api", "episodes", episodeId, "thumbnail")
    }

    func getProgress(episodeId: String) async throws -> EpisodeProgress? {
        try await request(url("api", "episodes", episodeId, "progress"))
    }

    func updateProgress(episodeId: String, position: Double, duration: Double) async throws {
        var req = requestURL(url("api", "episodes", episodeId, "progress"), method: "POST")
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let body = ProgressUpdate(positionSecs: position, durationSecs: duration)
        req.httpBody = try JSONEncoder().encode(body)
        let (_, response) = try await URLSession.shared.data(for: req)
        guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
            throw URLError(.badServerResponse)
        }
    }

    func deleteProgress(episodeId: String) async throws {
        let req = requestURL(url("api", "episodes", episodeId, "progress"), method: "DELETE")
        let _ = try await URLSession.shared.data(for: req)
    }

    func deleteSeriesProgress(seriesId: String) async throws {
        let req = requestURL(url("api", "series", seriesId, "progress"), method: "DELETE")
        let _ = try await URLSession.shared.data(for: req)
    }

    // MARK: - Playback Preparation

    func prepareEpisode(episodeId: String) async throws -> PrepareResponse {
        let req = requestURL(url("api", "episodes", episodeId, "prepare"), method: "POST")
        let (data, response) = try await URLSession.shared.data(for: req)
        guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
            throw CastError.networkError("Failed to prepare episode")
        }
        return try JSONDecoder().decode(PrepareResponse.self, from: data)
    }

    // MARK: - People

    func getPersonDetail(personId: Int) async throws -> PersonDetail {
        try await request(url("api", "person", String(personId)))
    }

    // MARK: - Credits

    func getEpisodeCredits(episodeId: String) async throws -> EpisodeCredits {
        try await request(url("api", "episodes", episodeId, "credits"))
    }

    // MARK: - Subtitles

    func listSubtitles(episodeId: String) async throws -> [SubtitleInfo] {
        try await request(url("api", "episodes", episodeId, "subtitles"))
    }

    func subtitleURL(episodeId: String, language: String) -> URL {
        url("api", "episodes", episodeId, "subtitles", language)
    }
}
