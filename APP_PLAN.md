# Cast tvOS App — Complete Implementation Plan

This document contains everything needed to build the Cast tvOS app from scratch. The app discovers a Cast media server on the local network via Bonjour, displays a series library with poster art, and plays video episodes with progress tracking.

---

## 1. Project Setup

### Creating the Xcode Project

1. Open Xcode. Select **File > New > Project**.
2. Choose **tvOS > App**. Click Next.
3. Configure:
   - **Product Name:** `Cast`
   - **Team:** (your Apple Developer team)
   - **Organization Identifier:** `com.edvin`
   - **Bundle Identifier:** `com.edvin.cast`
   - **Interface:** SwiftUI
   - **Lifecycle:** SwiftUI App
   - **Language:** Swift
   - **Include Tests:** No (not needed for v1)
4. Save the project into the `app/` directory so the `.xcodeproj` lands at `app/Cast.xcodeproj`.
5. In project settings, set **Minimum Deployments > tvOS** to **17.0**.
6. In **Signing & Capabilities**, ensure the following:
   - **Incoming Connections (Server)** is NOT needed (we are a client).
   - **Outgoing Connections (Client)** — enabled by default on tvOS.
   - **Bonjour Services** — add `_cast-media._tcp` to the Info.plist Bonjour services array.

### Info.plist Additions

Add to the app's Info.plist (or via Xcode target settings):

```xml
<key>NSBonjourServices</key>
<array>
    <string>_cast-media._tcp.</string>
</array>
<key>NSLocalNetworkUsageDescription</key>
<string>Cast needs local network access to find and stream from your media server.</string>
```

These keys are **required** on tvOS for NWBrowser/Bonjour to function. Without them, discovery will silently fail.

---

## 2. File Structure

Create the following files under `app/Cast/`:

```
app/Cast/
├── CastApp.swift                  — App entry point with NavigationStack
├── Models/
│   ├── Series.swift               — All Codable DTOs matching server JSON
│   └── ServerConnection.swift     — Observable server URL state
├── Services/
│   ├── APIClient.swift            — HTTP client for all server endpoints
│   ├── BonjourBrowser.swift       — NWBrowser wrapper for server discovery
│   └── ProgressReporter.swift     — Timer-based progress POST during playback
├── Views/
│   ├── ServerDiscoveryView.swift  — Find/select server on LAN
│   ├── SeriesGridView.swift       — Grid of series with posters
│   ├── SeriesDetailView.swift     — Episode list + "continue watching" button
│   └── PlayerView.swift           — AVPlayerViewController wrapper
└── Info.plist                     — Bonjour + local network keys
```

---

## 3. Data Models — `Models/Series.swift`

These structs must exactly match the JSON the server returns. All types are `Codable` and `Identifiable` for SwiftUI list/grid usage.

```swift
import Foundation

// MARK: - GET /api/series response

/// One item in the array returned by GET /api/series
struct SeriesListItem: Codable, Identifiable {
    let id: String
    let title: String
    let episodeCount: Int
    let hasArt: Bool
    let hasBackdrop: Bool
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
        case watchedCount = "watched_count"
        case totalCount = "total_count"
    }
}

// MARK: - GET /api/series/{id} response

/// Full series detail with episodes
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

/// One episode within a SeriesDetail
struct EpisodeItem: Codable, Identifiable {
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
    }

    /// Display label like "S1 E3" or "Episode 3" or just the index
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

/// Watch progress for an episode
struct EpisodeProgress: Codable {
    let positionSecs: Double
    let durationSecs: Double
    let completed: Bool

    enum CodingKeys: String, CodingKey {
        case completed
        case positionSecs = "position_secs"
        case durationSecs = "duration_secs"
    }

    /// Progress as 0.0–1.0
    var fraction: Double {
        guard durationSecs > 0 else { return 0 }
        return positionSecs / durationSecs
    }
}

// MARK: - GET /api/series/{id}/next response

struct NextEpisodeResponse: Codable {
    let episode: EpisodeItem?
    let reason: String  // "resume", "next", "first", "all_watched"
}

// MARK: - POST /api/episodes/{id}/progress request body

struct ProgressUpdate: Codable {
    let positionSecs: Double
    let durationSecs: Double

    enum CodingKeys: String, CodingKey {
        case positionSecs = "position_secs"
        case durationSecs = "duration_secs"
    }
}
```

**Key notes:**
- All server JSON uses `snake_case`. Swift models use `camelCase` with `CodingKeys` enums for mapping.
- `id` fields are 12-character hex strings (truncated UUID v5), e.g. `"6ba7b8109dad"`.
- `progress` on `EpisodeItem` is `Optional` — it is `null` when an episode has never been watched.
- `episode` on `NextEpisodeResponse` is `Optional` — it is `null` when all episodes are watched (reason = `"all_watched"`).

### Error Responses

All server endpoints return structured JSON errors on failure:

```swift
/// Server error response — returned by all endpoints on failure
struct ApiError: Codable {
    let error: String      // Human-readable error message
    let code: Int          // HTTP status code (404, 403, 500, 503)
    let detail: String?    // Optional additional context
}
```

Example server error responses:
- `404`: `{"error": "Series not found", "code": 404, "detail": null}`
- `403`: `{"error": "Access denied", "code": 403, "detail": null}`
- `500`: `{"error": "Failed to read file", "code": 500, "detail": null}`
- `503`: `{"error": "TMDB API key not configured", "code": 503, "detail": "This feature requires a TMDB API key"}`

The app should handle these errors gracefully — see **Section 9.5: Error Handling** below.

---

## 4. Server Connection — `Models/ServerConnection.swift`

```swift
import Foundation
import Observation

@Observable
final class ServerConnection {
    var baseURL: URL?

    /// Builds a base URL from host and port
    func connect(host: String, port: UInt16) {
        baseURL = URL(string: "http://\(host):\(port)")
    }

    /// For manual IP entry — expects "host:port" or just "host" (default port 3456)
    func connect(address: String) {
        let parts = address.split(separator: ":")
        let host = String(parts[0])
        let port: UInt16 = parts.count > 1 ? UInt16(parts[1]) ?? 3456 : 3456
        connect(host: host, port: port)
    }
}
```

Use the `@Observable` macro (iOS 17+ / tvOS 17+) instead of `ObservableObject`. This is injected into the environment at the app level.

---

## 5. API Client — `Services/APIClient.swift`

A stateless struct that takes a base URL and provides async methods for every server endpoint. Uses `URLSession.shared`.

```swift
import Foundation

struct APIClient {
    let baseURL: URL

    // MARK: - Series

    /// GET /api/series
    /// Returns all series with watch progress summary, sorted alphabetically.
    func listSeries() async throws -> [SeriesListItem] {
        let url = baseURL.appendingPathComponent("api/series")
        let (data, _) = try await URLSession.shared.data(from: url)
        return try JSONDecoder().decode([SeriesListItem].self, from: data)
    }

    /// GET /api/series/{id}
    /// Returns series detail with all episodes and per-episode progress.
    func getSeries(id: String) async throws -> SeriesDetail {
        let url = baseURL.appendingPathComponent("api/series/\(id)")
        let (data, _) = try await URLSession.shared.data(from: url)
        return try JSONDecoder().decode(SeriesDetail.self, from: data)
    }

    /// GET /api/series/{id}/next
    /// Returns the smart "next episode" recommendation.
    /// Reason will be one of: "resume", "next", "first", "all_watched", "No episodes"
    func getNextEpisode(seriesId: String) async throws -> NextEpisodeResponse {
        let url = baseURL.appendingPathComponent("api/series/\(seriesId)/next")
        let (data, _) = try await URLSession.shared.data(from: url)
        return try JSONDecoder().decode(NextEpisodeResponse.self, from: data)
    }

    /// GET /api/series/{id}/art
    /// Returns the URL for the series artwork image (for use with AsyncImage).
    /// Does NOT fetch — just builds the URL.
    func artURL(seriesId: String) -> URL {
        baseURL.appendingPathComponent("api/series/\(seriesId)/art")
    }

    /// GET /api/series/{id}/backdrop
    /// Returns the URL for the series backdrop image.
    func backdropURL(seriesId: String) -> URL {
        baseURL.appendingPathComponent("api/series/\(seriesId)/backdrop")
    }

    /// POST /api/metadata/fetch
    /// Triggers TMDB metadata fetch for all series.
    func fetchMetadata() async throws {
        var request = URLRequest(url: baseURL.appendingPathComponent("api/metadata/fetch"))
        request.httpMethod = "POST"
        let _ = try await URLSession.shared.data(for: request)
    }

    // MARK: - Episodes

    /// GET /api/episodes/{id}/stream
    /// Returns the URL for video streaming (for use with AVPlayer).
    /// Does NOT fetch — just builds the URL.
    func streamURL(episodeId: String) -> URL {
        baseURL.appendingPathComponent("api/episodes/\(episodeId)/stream")
    }

    /// GET /api/episodes/{id}/thumbnail
    /// Returns the URL for the episode thumbnail image.
    func thumbnailURL(episodeId: String) -> URL {
        baseURL.appendingPathComponent("api/episodes/\(episodeId)/thumbnail")
    }

    /// GET /api/episodes/{id}/progress
    /// Returns watch progress for a single episode (nil if never watched).
    func getProgress(episodeId: String) async throws -> EpisodeProgress? {
        let url = baseURL.appendingPathComponent("api/episodes/\(episodeId)/progress")
        let (data, _) = try await URLSession.shared.data(from: url)
        return try JSONDecoder().decode(EpisodeProgress?.self, from: data)
    }

    /// POST /api/episodes/{id}/progress
    /// Updates watch progress. Server auto-marks completed at 90%.
    /// Request body: {"position_secs": Double, "duration_secs": Double}
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
}
```

**Usage example:**
```swift
let client = APIClient(baseURL: URL(string: "http://192.168.1.50:3456")!)
let series = try await client.listSeries()
let detail = try await client.getSeries(id: series[0].id)
let streamURL = client.streamURL(episodeId: detail.episodes[0].id)
```

---

## 6. Bonjour Discovery — `Services/BonjourBrowser.swift`

Uses the Network framework's `NWBrowser` to discover `_cast-media._tcp` services on the local network.

```swift
import Foundation
import Network
import Observation

/// A discovered Cast server on the LAN
struct DiscoveredServer: Identifiable, Hashable {
    let id: String        // NWBrowser result hash or name
    let name: String      // mDNS service name, e.g. "Cast Server"
    let host: String      // Resolved hostname or IP
    let port: UInt16      // TCP port
}

@Observable
final class BonjourBrowser {
    var servers: [DiscoveredServer] = []
    var isSearching = false

    private var browser: NWBrowser?

    func startBrowsing() {
        let params = NWParameters()
        params.includePeerToPeer = true

        browser = NWBrowser(for: .bonjour(type: "_cast-media._tcp", domain: nil), using: params)

        browser?.stateUpdateHandler = { [weak self] state in
            Task { @MainActor in
                switch state {
                case .ready:
                    self?.isSearching = true
                case .failed, .cancelled:
                    self?.isSearching = false
                default:
                    break
                }
            }
        }

        browser?.browseResultsChangedHandler = { [weak self] results, _ in
            Task { @MainActor in
                self?.handleResults(results)
            }
        }

        browser?.start(queue: .main)
    }

    func stopBrowsing() {
        browser?.cancel()
        browser = nil
        isSearching = false
    }

    private func handleResults(_ results: Set<NWBrowser.Result>) {
        var discovered: [DiscoveredServer] = []

        for result in results {
            // Extract service name from the endpoint
            if case .service(let name, let type, let domain, _) = result.endpoint {
                // To get host/port, we need to resolve the endpoint using NWConnection
                // For now, store the endpoint info; resolution happens on selection
                discovered.append(DiscoveredServer(
                    id: "\(name).\(type)\(domain)",
                    name: name,
                    host: "",  // resolved on connect
                    port: 0
                ))
            }
        }

        servers = discovered
    }

    /// Resolve a discovered server to get its IP address and port.
    /// Uses NWConnection to resolve the Bonjour endpoint.
    func resolve(_ server: DiscoveredServer, completion: @escaping (String, UInt16) -> Void) {
        // Re-find the NWBrowser.Result for this server
        guard let results = browser?.browseResults else { return }
        guard let result = results.first(where: {
            if case .service(let name, _, _, _) = $0.endpoint {
                return name == server.name
            }
            return false
        }) else { return }

        let connection = NWConnection(to: result.endpoint, using: .tcp)
        connection.stateUpdateHandler = { state in
            if case .ready = state {
                // Extract the resolved endpoint
                if let path = connection.currentPath,
                   let endpoint = path.remoteEndpoint,
                   case .hostPort(let host, let port) = endpoint {
                    let hostString: String
                    switch host {
                    case .ipv4(let addr):
                        hostString = "\(addr)"
                    case .ipv6(let addr):
                        hostString = "\(addr)"
                    case .name(let name, _):
                        hostString = name
                    @unknown default:
                        hostString = "\(host)"
                    }
                    DispatchQueue.main.async {
                        completion(hostString, port.rawValue)
                    }
                }
                connection.cancel()
            }
        }
        connection.start(queue: .global())
    }
}
```

**Key implementation notes:**
- `NWBrowser` does not directly give you IP + port. It gives you `NWBrowser.Result` with an `NWEndpoint.service(...)`.
- To resolve to IP:port, create a temporary `NWConnection` to that endpoint. When it reaches `.ready`, inspect `currentPath.remoteEndpoint` which will be `.hostPort(host, port)`.
- The resolution approach above is the standard pattern for tvOS/iOS. An alternative is to use the endpoint directly with `NWConnection`, but since we need a plain `http://host:port` URL for `URLSession`, we must resolve.
- `includePeerToPeer` should be set to `true` to ensure discovery works across network segments if needed.
- The `NSBonjourServices` and `NSLocalNetworkUsageDescription` Info.plist entries from Section 1 are **mandatory** — without them, `NWBrowser` will not function.

---

## 7. Progress Reporter — `Services/ProgressReporter.swift`

A class that runs a timer during playback to periodically report the playback position to the server.

```swift
import Foundation

@Observable
final class ProgressReporter {
    private var timer: Timer?
    private var client: APIClient?
    private var episodeId: String?

    /// Start reporting progress every 10 seconds.
    /// `positionProvider` is called each tick to get the current position and duration.
    func start(
        client: APIClient,
        episodeId: String,
        positionProvider: @escaping () -> (position: Double, duration: Double)?
    ) {
        self.client = client
        self.episodeId = episodeId

        timer = Timer.scheduledTimer(withTimeInterval: 10.0, repeats: true) { [weak self] _ in
            guard let self,
                  let client = self.client,
                  let episodeId = self.episodeId,
                  let pos = positionProvider() else { return }

            Task {
                try? await client.updateProgress(
                    episodeId: episodeId,
                    position: pos.position,
                    duration: pos.duration
                )
            }
        }
    }

    /// Stop the periodic timer and send one final progress report.
    func stop(finalPosition: Double, finalDuration: Double) {
        timer?.invalidate()
        timer = nil

        guard let client, let episodeId else { return }
        Task {
            try? await client.updateProgress(
                episodeId: episodeId,
                position: finalPosition,
                duration: finalDuration
            )
        }

        self.client = nil
        self.episodeId = nil
    }
}
```

---

## 8. App Entry Point — `CastApp.swift`

```swift
import SwiftUI

@main
struct CastApp: App {
    @State private var connection = ServerConnection()

    var body: some Scene {
        WindowGroup {
            NavigationStack {
                if connection.baseURL != nil {
                    SeriesGridView()
                } else {
                    ServerDiscoveryView()
                }
            }
            .environment(connection)
        }
    }
}
```

**Notes:**
- Uses `@State` (not `@StateObject`) because `ServerConnection` uses `@Observable`.
- Injects the connection via `.environment()`.
- The root view conditionally shows discovery or the main grid based on whether a server is connected.
- `NavigationStack` is the top-level navigation container. All subsequent navigation uses `NavigationLink` or programmatic `path` navigation within this stack.

---

## 9. Views — Detailed Specifications

### 9.1 ServerDiscoveryView — `Views/ServerDiscoveryView.swift`

**Purpose:** First screen the user sees. Discovers Cast servers on the LAN and lets the user pick one, or manually enter an IP address.

**Layout:**
```
╔══════════════════════════════════════════════╗
║                                              ║
║              🔍 Looking for                  ║
║           Cast servers...                    ║
║           (ProgressView spinner)             ║
║                                              ║
║    ┌──────────────────────────────────┐      ║
║    │  📺 Living Room Cast Server     │      ║  ← Focusable button
║    └──────────────────────────────────┘      ║
║    ┌──────────────────────────────────┐      ║
║    │  📺 Office Cast Server          │      ║
║    └──────────────────────────────────┘      ║
║                                              ║
║    ┌──────────────────────────────────┐      ║
║    │  Enter server address manually  │      ║  ← Opens text field
║    └──────────────────────────────────┘      ║
║                                              ║
╚══════════════════════════════════════════════╝
```

**Behavior:**
- On appear: start `BonjourBrowser.startBrowsing()`.
- On disappear: stop browsing.
- Display `ProgressView` with "Looking for Cast servers..." while `isSearching` is true and servers list is empty.
- When servers are discovered, show them in a `List` or `VStack`. Each server is a `Button`.
- Clicking a discovered server: call `BonjourBrowser.resolve(...)` to get IP+port, then set `ServerConnection.connect(host:port:)`.
- "Enter server address manually" button: shows a text field (use `.alert` with a `TextField` or a separate view). User types `192.168.1.50:3456`. Parse and connect.
- On successful connection, the root `CastApp` view automatically switches to `SeriesGridView` since `connection.baseURL` becomes non-nil.

**Focus behavior:** The list of servers and the manual entry button are all focusable via standard SwiftUI `Button` — tvOS focus engine handles them automatically.

**Error states:**
- If no servers found after 5+ seconds, show "No servers found on your network" with the manual entry option still visible.
- If connection to selected server fails (test with a simple GET /api/series), show an alert and stay on this screen.

**Implementation:**

```swift
import SwiftUI

struct ServerDiscoveryView: View {
    @Environment(ServerConnection.self) private var connection
    @State private var browser = BonjourBrowser()
    @State private var showManualEntry = false
    @State private var manualAddress = ""
    @State private var isConnecting = false
    @State private var errorMessage: String?

    var body: some View {
        VStack(spacing: 40) {
            // Header
            VStack(spacing: 16) {
                ProgressView()
                    .opacity(browser.servers.isEmpty ? 1 : 0)
                Text("Looking for Cast servers...")
                    .font(.headline)
                    .foregroundStyle(.secondary)
            }
            .padding(.top, 60)

            // Discovered servers
            if !browser.servers.isEmpty {
                VStack(spacing: 20) {
                    ForEach(browser.servers) { server in
                        Button {
                            connectTo(server)
                        } label: {
                            HStack {
                                Image(systemName: "tv")
                                Text(server.name)
                                    .font(.title3)
                                Spacer()
                            }
                            .padding()
                        }
                    }
                }
                .padding(.horizontal, 80)
            }

            // Manual entry
            Button("Enter server address manually") {
                showManualEntry = true
            }

            Spacer()
        }
        .alert("Connect to Server", isPresented: $showManualEntry) {
            TextField("192.168.1.50:3456", text: $manualAddress)
            Button("Connect") {
                connection.connect(address: manualAddress)
            }
            Button("Cancel", role: .cancel) {}
        }
        .alert("Connection Error", isPresented: .init(
            get: { errorMessage != nil },
            set: { if !$0 { errorMessage = nil } }
        )) {
            Button("OK") { errorMessage = nil }
        } message: {
            Text(errorMessage ?? "")
        }
        .onAppear { browser.startBrowsing() }
        .onDisappear { browser.stopBrowsing() }
    }

    private func connectTo(_ server: DiscoveredServer) {
        browser.resolve(server) { host, port in
            connection.connect(host: host, port: port)
        }
    }
}
```

---

### 9.2 SeriesGridView — `Views/SeriesGridView.swift`

**Purpose:** Main browsing screen. Shows a grid of all series with poster art and watch progress.

**Layout:**
```
╔══════════════════════════════════════════════════════════╗
║  Cast                                          ⚙ Server ║
║                                                          ║
║  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐   ║
║  │         │  │         │  │         │  │         │   ║
║  │  poster │  │  poster │  │  poster │  │  poster │   ║
║  │         │  │         │  │         │  │         │   ║
║  ├─────────┤  ├─────────┤  ├─────────┤  ├─────────┤   ║
║  │Breaking │  │The Wire │  │Sopranos │  │  Lost   │   ║
║  │  Bad    │  │         │  │         │  │         │   ║
║  │ 3/62 ✓  │  │ 0/60    │  │12/86 ✓  │  │ 0/121   │   ║
║  └─────────┘  └─────────┘  └─────────┘  └─────────┘   ║
║                                                          ║
║  ┌─────────┐  ┌─────────┐                               ║
║  │         │  │         │                               ║
║  │  poster │  │  poster │                               ║
║  ...                                                     ║
╚══════════════════════════════════════════════════════════╝
```

**Behavior:**
- On appear: fetch `GET /api/series` via `APIClient.listSeries()`.
- Display as `LazyVGrid` with adaptive columns (minimum width ~220pt for tvOS).
- Each cell is a `NavigationLink` to `SeriesDetailView`.
- Poster images loaded with `AsyncImage(url: client.artURL(seriesId:))`.
- If `hasArt` is false, show a placeholder (gray rectangle with series title initials or a TV icon).
- Below the poster: series title (1-2 lines, `.title3` font) and "3/62 watched" subtitle (`.caption`, secondary color).
- Pull to refresh or re-fetch on appear to pick up server-side changes.

**Focus behavior:**
- tvOS focus engine automatically handles grid cell focus.
- When a cell gains focus, it should scale up slightly. Use the `.buttonStyle(.card)` style on each `NavigationLink`, which gives the standard tvOS lift/scale animation for free.
- If `.card` style is not sufficient, wrap each cell in a `Button` and apply `.hoverEffect(.lift)`.

**Grid configuration:**
```swift
let columns = [
    GridItem(.adaptive(minimum: 220, maximum: 300), spacing: 48)
]
```

**Error states:**
- Loading: show `ProgressView`.
- Empty library: "No series found. Add media folders to your Cast server."
- Network error: "Could not connect to server" with a Retry button.

**Implementation outline:**

```swift
import SwiftUI

struct SeriesGridView: View {
    @Environment(ServerConnection.self) private var connection
    @State private var seriesList: [SeriesListItem] = []
    @State private var isLoading = true
    @State private var error: String?

    private var client: APIClient? {
        guard let url = connection.baseURL else { return nil }
        return APIClient(baseURL: url)
    }

    let columns = [
        GridItem(.adaptive(minimum: 220, maximum: 300), spacing: 48)
    ]

    var body: some View {
        Group {
            if isLoading {
                ProgressView("Loading series...")
            } else if let error {
                VStack(spacing: 20) {
                    Text(error).foregroundStyle(.secondary)
                    Button("Retry") { Task { await loadSeries() } }
                }
            } else if seriesList.isEmpty {
                Text("No series found.")
                    .foregroundStyle(.secondary)
            } else {
                ScrollView {
                    LazyVGrid(columns: columns, spacing: 60) {
                        ForEach(seriesList) { series in
                            NavigationLink(value: series) {
                                SeriesCell(series: series, artURL: client?.artURL(seriesId: series.id))
                            }
                            .buttonStyle(.card)
                        }
                    }
                    .padding(60)
                }
            }
        }
        .navigationTitle("Cast")
        .navigationDestination(for: SeriesListItem.self) { series in
            SeriesDetailView(seriesId: series.id, seriesTitle: series.title)
        }
        .task { await loadSeries() }
    }

    private func loadSeries() async {
        guard let client else { return }
        isLoading = true
        error = nil
        do {
            seriesList = try await client.listSeries()
        } catch {
            self.error = "Could not load series."
        }
        isLoading = false
    }
}
```

**SeriesCell** (define as a private struct in the same file or a separate file):

```swift
struct SeriesCell: View {
    let series: SeriesListItem
    let artURL: URL?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            // Poster
            if series.hasArt, let url = artURL {
                AsyncImage(url: url) { image in
                    image.resizable().aspectRatio(2/3, contentMode: .fill)
                } placeholder: {
                    posterPlaceholder
                }
                .frame(width: 220, height: 330)
                .clipped()
                .cornerRadius(12)
            } else {
                posterPlaceholder
            }

            // Title
            Text(series.title)
                .font(.title3)
                .lineLimit(2)

            // Progress
            Text("\(series.watchedCount)/\(series.totalCount) watched")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .frame(width: 220)
    }

    private var posterPlaceholder: some View {
        RoundedRectangle(cornerRadius: 12)
            .fill(.quaternary)
            .frame(width: 220, height: 330)
            .overlay {
                Image(systemName: "tv")
                    .font(.largeTitle)
                    .foregroundStyle(.secondary)
            }
    }
}
```

**Note:** For `NavigationLink(value:)` to work, `SeriesListItem` must conform to `Hashable`. Add `Hashable` conformance to the struct definition.

---

### 9.3 SeriesDetailView — `Views/SeriesDetailView.swift`

**Purpose:** Shows episodes for a series with a prominent "continue watching" action and an episode list.

**Layout:**
```
╔══════════════════════════════════════════════════════════╗
║  ← Breaking Bad                                         ║
║                                                          ║
║  ┌──────────────────────────────────────────────────┐   ║
║  │  ▶ Continue watching: S01E04 Cancer Man          │   ║  ← Big focused button
║  │    Resume at 23:45                               │   ║
║  └──────────────────────────────────────────────────┘   ║
║                                                          ║
║  Episodes                                                ║
║  ┌──────────────────────────────────────────────────┐   ║
║  │ 1. S01E01 Pilot                          ✓      │   ║
║  ├──────────────────────────────────────────────────┤   ║
║  │ 2. S01E02 Cat's in the Bag              ✓      │   ║
║  ├──────────────────────────────────────────────────┤   ║
║  │ 3. S01E03 And the Bag's in the River    ✓      │   ║
║  ├──────────────────────────────────────────────────┤   ║
║  │ 4. S01E04 Cancer Man                    ◐ 23:45│   ║  ← In-progress
║  ├──────────────────────────────────────────────────┤   ║
║  │ 5. S01E05 Gray Matter                    ○      │   ║  ← Unwatched
║  └──────────────────────────────────────────────────┘   ║
╚══════════════════════════════════════════════════════════╝
```

**Behavior:**
- On appear: fetch both `GET /api/series/{id}` (for episode list) and `GET /api/series/{id}/next` (for smart recommendation) concurrently.
- **Continue button** at top:
  - If reason is `"resume"`: "Continue watching: {episode.title} — Resume at {MM:SS}" (format `position_secs` as `MM:SS`).
  - If reason is `"next"`: "Start next: {episode.title}".
  - If reason is `"first"`: "Start watching: {episode.title}".
  - If reason is `"all_watched"`: "All episodes watched" (dimmed, or offer to rewatch first episode).
  - Clicking this button navigates to `PlayerView` with the recommended episode. If resuming, pass the `position_secs` so the player can seek.

- **Episode list:**
  - Each row shows: episode index (1-based for display, i.e. `episode.index + 1`), episode title, and watch state icon on the right.
  - Watch state indicators:
    - Completed: checkmark icon (`checkmark.circle.fill`, green tint)
    - In-progress: half-filled circle (`circle.lefthalf.filled`, blue tint) with formatted time position
    - Unwatched: empty circle (`circle`, gray)
  - Clicking an episode row navigates to `PlayerView` for that episode. If the episode has progress, pass it for resume.

- **Refresh on return:** When the user returns from `PlayerView` (back navigation), the view should refresh its data. Use `.task(id:)` with a refresh trigger or `.onAppear` to re-fetch.

**Focus behavior:**
- The "Continue watching" button should have **initial focus** when the view appears. Use `@FocusState` and `.defaultFocus()` to achieve this.
- The episode list scrolls vertically; focus moves naturally between list items.

**Implementation outline:**

```swift
import SwiftUI

struct SeriesDetailView: View {
    let seriesId: String
    let seriesTitle: String

    @Environment(ServerConnection.self) private var connection
    @State private var detail: SeriesDetail?
    @State private var nextEpisode: NextEpisodeResponse?
    @State private var isLoading = true
    @State private var selectedEpisode: EpisodeItem?
    @State private var resumePosition: Double = 0
    @State private var showPlayer = false

    private var client: APIClient? {
        guard let url = connection.baseURL else { return nil }
        return APIClient(baseURL: url)
    }

    var body: some View {
        Group {
            if isLoading {
                ProgressView()
            } else if let detail {
                ScrollView {
                    VStack(alignment: .leading, spacing: 40) {
                        // Continue watching button
                        if let next = nextEpisode, let ep = next.episode {
                            Button {
                                selectedEpisode = ep
                                resumePosition = ep.progress?.positionSecs ?? 0
                                showPlayer = true
                            } label: {
                                VStack(alignment: .leading, spacing: 8) {
                                    Text(continueButtonTitle(reason: next.reason))
                                        .font(.headline)
                                    Text(ep.title)
                                        .font(.title3)
                                    if next.reason == "resume", let pos = ep.progress?.positionSecs {
                                        Text("Resume at \(formatTime(pos))")
                                            .font(.subheadline)
                                            .foregroundStyle(.secondary)
                                    }
                                }
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .padding(24)
                            }
                        } else if nextEpisode?.reason == "all_watched" {
                            Text("All episodes watched")
                                .font(.headline)
                                .foregroundStyle(.secondary)
                                .padding()
                        }

                        // Episode list
                        Text("Episodes")
                            .font(.title2)
                            .padding(.leading, 24)

                        ForEach(detail.episodes) { episode in
                            Button {
                                selectedEpisode = episode
                                resumePosition = episode.progress?.positionSecs ?? 0
                                showPlayer = true
                            } label: {
                                HStack {
                                    Text("\(episode.index + 1).")
                                        .font(.body)
                                        .foregroundStyle(.secondary)
                                        .frame(width: 50, alignment: .trailing)
                                    Text(episode.title)
                                        .font(.body)
                                    Spacer()
                                    episodeStatusIcon(episode.progress)
                                }
                                .padding(.horizontal, 24)
                                .padding(.vertical, 12)
                            }
                        }
                    }
                    .padding(48)
                }
            }
        }
        .navigationTitle(seriesTitle)
        .fullScreenCover(isPresented: $showPlayer) {
            // Re-load data when player is dismissed
            Task { await loadData() }
        } content: {
            if let episode = selectedEpisode, let client {
                PlayerView(
                    client: client,
                    episode: episode,
                    resumePosition: resumePosition
                )
            }
        }
        .task { await loadData() }
    }

    private func loadData() async {
        guard let client else { return }
        isLoading = true
        async let detailReq = client.getSeries(id: seriesId)
        async let nextReq = client.getNextEpisode(seriesId: seriesId)
        do {
            detail = try await detailReq
            nextEpisode = try await nextReq
        } catch {
            // Handle error
        }
        isLoading = false
    }

    @ViewBuilder
    private func episodeStatusIcon(_ progress: EpisodeProgress?) -> some View {
        if let p = progress {
            if p.completed {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(.green)
            } else {
                HStack(spacing: 6) {
                    Text(formatTime(p.positionSecs))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Image(systemName: "circle.lefthalf.filled")
                        .foregroundStyle(.blue)
                }
            }
        } else {
            Image(systemName: "circle")
                .foregroundStyle(.gray)
        }
    }

    private func continueButtonTitle(reason: String) -> String {
        switch reason {
        case "resume": return "Continue watching"
        case "next": return "Start next"
        case "first": return "Start watching"
        default: return "Play"
        }
    }

    private func formatTime(_ seconds: Double) -> String {
        let mins = Int(seconds) / 60
        let secs = Int(seconds) % 60
        return String(format: "%d:%02d", mins, secs)
    }
}
```

---

### 9.4 PlayerView — `Views/PlayerView.swift`

**Purpose:** Full-screen video player wrapping `AVPlayerViewController` with progress reporting.

**Critical design decisions:**
- Use `UIViewControllerRepresentable` to wrap `AVPlayerViewController`. This is **required** because `AVPlayerViewController` on tvOS provides the native transport controls (scrubbing, play/pause, info panel, skip) that cannot be replicated in pure SwiftUI.
- SwiftUI's `VideoPlayer` view does NOT provide the full tvOS transport UI. You must use `AVPlayerViewController` directly.

**Implementation:**

```swift
import SwiftUI
import AVKit

struct PlayerView: UIViewControllerRepresentable {
    let client: APIClient
    let episode: EpisodeItem
    let resumePosition: Double

    @Environment(\.dismiss) private var dismiss

    func makeUIViewController(context: Context) -> AVPlayerViewController {
        let controller = AVPlayerViewController()
        let url = client.streamURL(episodeId: episode.id)
        let player = AVPlayer(url: url)
        controller.player = player
        controller.delegate = context.coordinator
        return controller
    }

    func updateUIViewController(_ controller: AVPlayerViewController, context: Context) {
        // No updates needed — player is configured once in make
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(self)
    }

    class Coordinator: NSObject, AVPlayerViewControllerDelegate {
        let parent: PlayerView
        private var progressReporter: ProgressReporter?
        private var playerRef: AVPlayer?
        private var timeObserver: Any?

        init(_ parent: PlayerView) {
            self.parent = parent
            super.init()
        }

        /// Called after the AVPlayerViewController is presented.
        /// Start playback, seek if resuming, and begin progress reporting.
        func startPlayback(player: AVPlayer) {
            self.playerRef = player

            // Seek to resume position if > 0
            if parent.resumePosition > 0 {
                let time = CMTime(seconds: parent.resumePosition, preferredTimescale: 600)
                player.seek(to: time) { [weak player] _ in
                    player?.play()
                }
            } else {
                player.play()
            }

            // Auto-select English subtitles if available
            selectEnglishSubtitles(player: player)

            // Start progress reporting
            let reporter = ProgressReporter()
            reporter.start(client: parent.client, episodeId: parent.episode.id) { [weak player] in
                guard let player, let item = player.currentItem else { return nil }
                let pos = player.currentTime().seconds
                let dur = item.duration.seconds
                guard pos.isFinite, dur.isFinite, dur > 0 else { return nil }
                return (position: pos, duration: dur)
            }
            self.progressReporter = reporter

            // Observe end of playback to auto-dismiss
            NotificationCenter.default.addObserver(
                self,
                selector: #selector(playerDidFinish),
                name: .AVPlayerItemDidPlayToEndTime,
                object: player.currentItem
            )
        }

        /// Auto-select English subtitle track if one exists in the media file.
        /// Embedded subtitles (tx3g in MP4, SRT/ASS in MKV) are exposed by AVPlayer
        /// as AVMediaSelectionOptions in the .legible group.
        /// Users can always change subtitles via the standard tvOS player menu.
        private func selectEnglishSubtitles(player: AVPlayer) {
            guard let item = player.currentItem else { return }
            // Need to wait for the asset to be ready
            Task {
                let asset = item.asset
                if let group = try? await asset.loadMediaSelectionGroup(for: .legible) {
                    let english = AVMediaSelectionGroup.mediaSelectionOptions(
                        from: group.options,
                        with: Locale(identifier: "en")
                    ).first
                    if let track = english {
                        await MainActor.run {
                            item.select(track, in: group)
                        }
                    }
                }
            }
        }

        @objc private func playerDidFinish(_ notification: Notification) {
            reportFinalProgress()
            Task { @MainActor in
                parent.dismiss()
            }
        }

        /// Called when user dismisses the player (swipe down / Menu button)
        func playerViewControllerDidEndDismissalTransition(_ playerViewController: AVPlayerViewController) {
            reportFinalProgress()
        }

        /// AVPlayerViewControllerDelegate: the user wants to dismiss
        func playerViewControllerShouldDismiss(_ playerViewController: AVPlayerViewController) -> Bool {
            return true
        }

        private func reportFinalProgress() {
            guard let player = playerRef, let item = player.currentItem else {
                progressReporter?.stop(finalPosition: 0, finalDuration: 0)
                return
            }
            let pos = player.currentTime().seconds
            let dur = item.duration.seconds
            let safePos = pos.isFinite ? pos : 0
            let safeDur = dur.isFinite ? dur : 0
            progressReporter?.stop(finalPosition: safePos, finalDuration: safeDur)
        }

        deinit {
            NotificationCenter.default.removeObserver(self)
        }
    }
}
```

**IMPORTANT: Triggering playback start.** The `makeUIViewController` creates the player and controller, but playback should begin once the view is presented. To handle this, observe the player's status or use the coordinator pattern. A practical approach is to start playback in `makeUIViewController` itself right after creating the player:

In `makeUIViewController`, after setting up the player, call:
```swift
context.coordinator.startPlayback(player: player)
```

This ensures the seek + play + progress reporter all start immediately.

**Key behaviors:**
1. **Seek on resume:** If `resumePosition > 0`, seek to that position before starting playback. The seek completion handler then calls `play()`.
2. **Progress reporting:** Timer fires every 10 seconds, reads `player.currentTime().seconds` and `player.currentItem.duration.seconds`, POSTs to server.
3. **Dismiss on finish:** When `AVPlayerItemDidPlayToEndTime` fires, send final progress and dismiss. The server will mark the episode as completed (>= 90% = completed).
4. **Dismiss on user action:** When the user presses Menu/Back on the Siri Remote, `AVPlayerViewController` triggers dismissal. The delegate method `playerViewControllerDidEndDismissalTransition` fires — send final progress there.
5. **Transport controls:** `AVPlayerViewController` on tvOS automatically provides: play/pause (click), scrub (swipe left/right), 10-second skip (click edges), info panel, subtitle selection — all for free.
6. **Subtitles:** Embedded subtitle tracks (tx3g in MP4, SRT/ASS in MKV) are automatically available. The player auto-selects English subtitles on load. Users can change the subtitle/audio track via the standard tvOS player info panel (swipe down on Siri Remote during playback). No server-side subtitle API is needed — AVPlayer handles this from the stream.

**Presenting the player:**
The player is shown via `.fullScreenCover()` from `SeriesDetailView`. This gives a full-screen modal presentation, which is the standard for video playback on tvOS. When dismissed (by user or by end-of-playback), control returns to `SeriesDetailView` which re-fetches data to show updated progress.

---

## 9.5 Error Handling — `Views/ErrorView.swift`

**Purpose:** A reusable error view shown when any API call fails. Displays a clear error message with Retry and Cancel/Back actions.

**The server returns structured JSON errors** (see Section 3 — Error Responses). The app should parse these and display them nicely.

### APIClient Error Handling

Update `APIClient` to decode error responses:

```swift
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

// In APIClient, add a helper for all requests:
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
```

### ErrorView

A standalone view that can be shown anywhere an error occurs:

```swift
struct ErrorView: View {
    let title: String
    let message: String
    let detail: String?
    let onRetry: (() -> Void)?
    let onDismiss: (() -> Void)?

    var body: some View {
        VStack(spacing: 40) {
            // Error icon
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 80))
                .foregroundColor(.orange)

            // Title
            Text(title)
                .font(.title)
                .multilineTextAlignment(.center)

            // Message
            Text(message)
                .font(.body)
                .foregroundColor(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 600)

            // Detail (if present)
            if let detail {
                Text(detail)
                    .font(.caption)
                    .foregroundColor(.secondary)
                    .multilineTextAlignment(.center)
            }

            // Action buttons
            HStack(spacing: 40) {
                if let onRetry {
                    Button("Try Again") { onRetry() }
                        .buttonStyle(.borderedProminent)
                }
                if let onDismiss {
                    Button("Go Back") { onDismiss() }
                }
            }
        }
        .padding(60)
    }
}
```

### Usage Pattern in Views

Every view that loads data should follow this pattern:

```swift
struct SeriesGridView: View {
    @State private var series: [SeriesListItem] = []
    @State private var error: CastError?
    @State private var isLoading = true

    var body: some View {
        Group {
            if let error {
                ErrorView(
                    title: "Unable to Load Series",
                    message: error.errorDescription ?? "An unknown error occurred",
                    detail: error.detail,
                    onRetry: { self.error = nil; Task { await loadData() } },
                    onDismiss: nil  // or navigate back
                )
            } else if isLoading {
                ProgressView("Loading...")
            } else {
                // Normal content
                seriesGrid
            }
        }
        .task { await loadData() }
    }

    private func loadData() async {
        isLoading = true
        defer { isLoading = false }
        do {
            series = try await client.listSeries()
            error = nil
        } catch let err as CastError {
            error = err
        } catch {
            self.error = .networkError(error.localizedDescription)
        }
    }
}
```

**Apply this pattern to:** `SeriesGridView`, `SeriesDetailView`, and `ServerDiscoveryView`.

**For PlayerView:** If the stream fails to load, `AVPlayer` will report an error via its `status` property. Observe `AVPlayerItem.status` — if it becomes `.failed`, dismiss the player and show the error in the parent view.

---

## 10. Navigation Flow

```
CastApp (root)
  └── NavigationStack
        ├── ServerDiscoveryView    (if no server connected)
        │     └── [user selects server] → sets connection.baseURL
        └── SeriesGridView         (if server connected)
              └── NavigationLink → SeriesDetailView(seriesId, seriesTitle)
                    └── .fullScreenCover → PlayerView(client, episode, resumePosition)
                          └── [dismiss] → back to SeriesDetailView (refreshes)
```

- Navigation between discovery and series grid is handled by the conditional in `CastApp` based on `connection.baseURL`.
- Navigation from grid to detail uses `NavigationLink(value:)` with `.navigationDestination(for:)`.
- Navigation from detail to player uses `.fullScreenCover` for immersive full-screen video.
- Returning from the player triggers a data reload in `SeriesDetailView`.

---

## 11. Image/Art Loading

**Strategy:** Use SwiftUI's built-in `AsyncImage` for all poster/artwork loading. On tvOS 17+, `AsyncImage` handles caching via the shared `URLSession` URL cache automatically.

```swift
AsyncImage(url: client.artURL(seriesId: series.id)) { phase in
    switch phase {
    case .success(let image):
        image
            .resizable()
            .aspectRatio(2/3, contentMode: .fill)
    case .failure:
        posterPlaceholder
    case .empty:
        ProgressView()
    @unknown default:
        posterPlaceholder
    }
}
.frame(width: 220, height: 330)
.clipped()
.cornerRadius(12)
```

**Placeholder:** For series without art (`hasArt == false`), do NOT call the art URL at all. Show a placeholder directly — gray rounded rectangle with a TV icon and/or the first letter of the series title.

**Cache notes:**
- `URLSession.shared` uses `URLCache.shared` which provides in-memory and on-disk caching.
- For tvOS, the default cache is sufficient. Art images are typically small (< 1 MB).
- If needed in the future, a more sophisticated `URLCache` can be configured with a larger capacity.

---

## 12. tvOS-Specific Considerations

### Focus Engine
- **All interactive elements must be `Button`, `NavigationLink`, or explicitly `.focusable()`.**
- tvOS uses a focus engine — the user swipes on the Siri Remote to move focus between elements, then clicks to select.
- `LazyVGrid` items wrapped in `NavigationLink` with `.buttonStyle(.card)` get automatic focus lift animation.
- Use `@FocusState` when you need to programmatically set initial focus (e.g., the "Continue watching" button in `SeriesDetailView`).
- `List` rows are automatically focusable.

### Siri Remote
- **Click center** = select/activate
- **Swipe** = move focus
- **Menu button** = back/dismiss
- **Play/Pause button** = play/pause (in AVPlayerViewController, this is handled automatically)
- No touch gestures, no drag-and-drop.

### 10-Foot UI
- Minimum readable text: ~29pt (`.body` or larger).
- Use `.title`, `.title2`, `.title3`, `.headline` for primary text.
- Use `.body` for list items.
- Use `.caption` and `.footnote` sparingly (only for secondary metadata).
- All padding should be generous — 40-60pt between sections.
- Grid items should be at least 200pt wide.

### AVPlayerViewController
- On tvOS, this is the **only** proper way to play video. It provides the full native transport UI.
- Do NOT try to build custom player controls — the tvOS native experience is expected by users and tightly integrated with the Siri Remote.
- The `AVPlayerViewController` handles: play/pause, scrubbing, 10-sec skip forward/back, subtitle selection, audio track selection, and the info panel.

### No `TextField` on Main UI
- Text input on tvOS uses an on-screen keyboard which is cumbersome. Minimize its use.
- The manual IP entry in `ServerDiscoveryView` is the only text field in the entire app. Use an `.alert` with a `TextField` to present it.

---

## 13. Required Protocol Conformances

For navigation and SwiftUI collections to work, add `Hashable` conformance where needed:

```swift
struct SeriesListItem: Codable, Identifiable, Hashable { ... }
struct EpisodeItem: Codable, Identifiable, Hashable { ... }
struct EpisodeProgress: Codable, Hashable { ... }
```

These are needed because:
- `SeriesListItem` is used as a `NavigationLink` value (requires `Hashable`).
- `EpisodeItem` may be used in `ForEach` and as selection state.
- `EpisodeProgress` is nested inside `EpisodeItem` and must also be `Hashable` for the parent to conform.

---

## 14. Complete Build Checklist

1. **Create Xcode project**: tvOS App, SwiftUI lifecycle, deployment target tvOS 17.0, bundle ID `com.edvin.cast`, save in `app/`.
2. **Add Info.plist entries**: `NSBonjourServices` with `_cast-media._tcp.`, `NSLocalNetworkUsageDescription`.
3. **Create `Models/Series.swift`**: All DTOs from Section 3 with Codable + Identifiable + Hashable.
4. **Create `Models/ServerConnection.swift`**: Observable server URL state from Section 4.
5. **Create `Services/APIClient.swift`**: HTTP client from Section 5.
6. **Create `Services/BonjourBrowser.swift`**: NWBrowser wrapper from Section 6.
7. **Create `Services/ProgressReporter.swift`**: Timer-based reporter from Section 7.
8. **Create `CastApp.swift`**: App entry point from Section 8.
9. **Create `Views/ServerDiscoveryView.swift`**: Discovery UI from Section 9.1.
10. **Create `Views/SeriesGridView.swift`**: Series grid with poster art from Section 9.2 (include `SeriesCell`).
11. **Create `Views/SeriesDetailView.swift`**: Episode list + continue button from Section 9.3.
12. **Create `Views/PlayerView.swift`**: AVPlayerViewController wrapper from Section 9.4.
13. **Build and test** on Apple TV simulator or device on the same LAN as a running Cast server.

---

## 15. Testing Against the Server

Start the server:
```bash
cd server
cargo run -- --media /path/to/your/shows --port 3456
```

The server will:
- Scan the media directory for series and episodes.
- Advertise via Bonjour as `_cast-media._tcp` on port 3456.
- Serve the HTTP API at `http://<server-ip>:3456`.

The tvOS app should:
1. Discover the server automatically via Bonjour.
2. Load and display the series grid with artwork.
3. Allow navigating to a series and seeing the episode list.
4. Play episodes with full transport controls.
5. Report progress back — verify by calling `GET /api/progress` on the server or checking `cast.db`.
