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

    // MARK: - Generic Request Helper

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
        try await request(baseURL.appendingPathComponent("api/continue-watching"))
    }

    // MARK: - Series

    func listSeries() async throws -> [SeriesListItem] {
        try await request(baseURL.appendingPathComponent("api/series"))
    }

    func getSeries(id: String) async throws -> SeriesDetail {
        try await request(baseURL.appendingPathComponent("api/series/\(id)"))
    }

    func getNextEpisode(seriesId: String) async throws -> NextEpisodeResponse {
        try await request(baseURL.appendingPathComponent("api/series/\(seriesId)/next"))
    }

    func artURL(seriesId: String) -> URL {
        baseURL.appendingPathComponent("api/series/\(seriesId)/art")
    }

    func backdropURL(seriesId: String) -> URL {
        baseURL.appendingPathComponent("api/series/\(seriesId)/backdrop")
    }

    func fetchMetadata() async throws {
        var request = URLRequest(url: baseURL.appendingPathComponent("api/metadata/fetch"))
        request.httpMethod = "POST"
        let _ = try await URLSession.shared.data(for: request)
    }

    // MARK: - Episodes

    func streamURL(episodeId: String) -> URL {
        baseURL.appendingPathComponent("api/episodes/\(episodeId)/stream")
    }

    func thumbnailURL(episodeId: String) -> URL {
        baseURL.appendingPathComponent("api/episodes/\(episodeId)/thumbnail")
    }

    func getProgress(episodeId: String) async throws -> EpisodeProgress? {
        try await request(baseURL.appendingPathComponent("api/episodes/\(episodeId)/progress"))
    }

    func updateProgress(episodeId: String, position: Double, duration: Double) async throws {
        let url = baseURL.appendingPathComponent("api/episodes/\(episodeId)/progress")
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        let body = ProgressUpdate(positionSecs: position, durationSecs: duration)
        request.httpBody = try JSONEncoder().encode(body)
        let (_, response) = try await URLSession.shared.data(for: request)
        guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
            throw URLError(.badServerResponse)
        }
    }

    func deleteProgress(episodeId: String) async throws {
        let url = baseURL.appendingPathComponent("api/episodes/\(episodeId)/progress")
        var request = URLRequest(url: url)
        request.httpMethod = "DELETE"
        let _ = try await URLSession.shared.data(for: request)
    }

    func deleteSeriesProgress(seriesId: String) async throws {
        let url = baseURL.appendingPathComponent("api/series/\(seriesId)/progress")
        var request = URLRequest(url: url)
        request.httpMethod = "DELETE"
        let _ = try await URLSession.shared.data(for: request)
    }

    // MARK: - Credits

    func getEpisodeCredits(episodeId: String) async throws -> EpisodeCredits {
        try await request(baseURL.appendingPathComponent("api/episodes/\(episodeId)/credits"))
    }

    // MARK: - Subtitles

    func listSubtitles(episodeId: String) async throws -> [SubtitleInfo] {
        try await request(baseURL.appendingPathComponent("api/episodes/\(episodeId)/subtitles"))
    }

    func subtitleURL(episodeId: String, language: String) -> URL {
        baseURL.appendingPathComponent("api/episodes/\(episodeId)/subtitles/\(language)")
    }
}
