import Foundation

// MARK: - GET /api/series response

struct SeriesListItem: Codable, Identifiable, Hashable {
    let id: String
    let title: String
    let episodeCount: Int
    let hasArt: Bool
    let hasBackdrop: Bool
    let hasMetadata: Bool
    let overview: String?
    let genres: String?
    let rating: Double?
    let year: String?
    let watchedCount: Int
    let totalCount: Int

    enum CodingKeys: String, CodingKey {
        case id, title, overview, genres, rating, year
        case episodeCount = "episode_count"
        case hasArt = "has_art"
        case hasBackdrop = "has_backdrop"
        case hasMetadata = "has_metadata"
        case watchedCount = "watched_count"
        case totalCount = "total_count"
    }
}

// MARK: - GET /api/series/{id} response

struct SeriesDetail: Codable, Identifiable {
    let id: String
    let title: String
    let hasArt: Bool
    let hasBackdrop: Bool
    let overview: String?
    let genres: String?
    let rating: Double?
    let year: String?
    let episodes: [EpisodeItem]

    enum CodingKeys: String, CodingKey {
        case id, title, episodes, overview, genres, rating, year
        case hasArt = "has_art"
        case hasBackdrop = "has_backdrop"
    }
}

// MARK: - Episode

struct EpisodeItem: Codable, Identifiable, Hashable {
    let id: String
    let title: String
    let index: Int
    let seasonNumber: Int?
    let episodeNumber: Int?
    let sizeBytes: Int64
    let durationSecs: Double?
    let overview: String?
    let airDate: String?
    let runtimeMinutes: Int?
    let hasThumbnail: Bool
    let stillUrl: String?
    let subtitleLanguages: [String]
    let progress: EpisodeProgress?

    enum CodingKeys: String, CodingKey {
        case id, title, index, progress, overview
        case seasonNumber = "season_number"
        case episodeNumber = "episode_number"
        case sizeBytes = "size_bytes"
        case durationSecs = "duration_secs"
        case airDate = "air_date"
        case runtimeMinutes = "runtime_minutes"
        case hasThumbnail = "has_thumbnail"
        case stillUrl = "still_url"
        case subtitleLanguages = "subtitle_languages"
    }

    var hasExternalSubtitles: Bool { !subtitleLanguages.isEmpty }

    var episodeLabel: String {
        if let s = seasonNumber, let e = episodeNumber {
            return "S\(s) E\(e)"
        }
        if let e = episodeNumber {
            return "Episode \(e)"
        }
        return "Episode \(index + 1)"
    }
}

// MARK: - Watch Progress

struct EpisodeProgress: Codable, Hashable {
    let positionSecs: Double
    let durationSecs: Double
    let completed: Bool

    enum CodingKeys: String, CodingKey {
        case completed
        case positionSecs = "position_secs"
        case durationSecs = "duration_secs"
    }

    var fraction: Double {
        guard durationSecs > 0 else { return 0 }
        return positionSecs / durationSecs
    }
}

// MARK: - GET /api/series/{id}/next response

struct NextEpisodeResponse: Codable {
    let episode: EpisodeItem?
    let reason: String
}

// MARK: - GET /api/continue-watching response

struct ContinueWatchingItem: Codable, Identifiable {
    let seriesId: String
    let seriesTitle: String
    let hasArt: Bool
    let hasBackdrop: Bool
    let nextEpisode: EpisodeItem
    let reason: String

    var id: String { seriesId }

    enum CodingKeys: String, CodingKey {
        case reason
        case seriesId = "series_id"
        case seriesTitle = "series_title"
        case hasArt = "has_art"
        case hasBackdrop = "has_backdrop"
        case nextEpisode = "next_episode"
    }
}

// MARK: - Subtitles

struct SubtitleInfo: Codable, Identifiable {
    let language: String
    let label: String

    var id: String { language }
}

// MARK: - Progress Update (POST body)

struct ProgressUpdate: Codable {
    let positionSecs: Double
    let durationSecs: Double

    enum CodingKeys: String, CodingKey {
        case positionSecs = "position_secs"
        case durationSecs = "duration_secs"
    }
}

// MARK: - Episode Credits

struct CastMember: Codable, Identifiable {
    let name: String
    let character: String?
    let profileUrl: String?
    let order: Int
    let isGuest: Bool

    var id: String { "\(name)-\(order)" }

    enum CodingKeys: String, CodingKey {
        case name, character, order
        case profileUrl = "profile_url"
        case isGuest = "is_guest"
    }
}

struct EpisodeCredits: Codable {
    let cast: [CastMember]
    let guestStars: [CastMember]

    enum CodingKeys: String, CodingKey {
        case cast
        case guestStars = "guest_stars"
    }
}

// MARK: - Server Error Response

struct ApiError: Codable {
    let error: String
    let code: Int
    let detail: String?
}
