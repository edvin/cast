# Cast — Local Network Video Streamer for Apple TV

Stream series from a server on your LAN to an Apple TV app.

## Architecture

```
┌─────────────┐        HTTP (LAN)        ┌──────────────┐
│  Apple TV   │ ◄──────────────────────► │  Cast Server  │
│  tvOS App   │   GET /api/series        │  (Rust/axum)  │
│  (AVPlayer) │   GET /api/episodes/…    │               │
│             │   Bonjour discovery      │  Advertises   │
│             │                          │  via mDNS     │
└─────────────┘                          └──────────────┘
```

## Media Directory Layout

```
media/
├── Breaking Bad/
│   ├── poster.jpg          ← series art (auto-detected)
│   ├── S01E01 Pilot.mp4
│   ├── S01E02 Cat's in the Bag.mp4
│   └── ...
├── The Wire/
│   ├── cover.png
│   ├── Episode 01.mp4
│   └── ...
```

Each direct subdirectory = a series. Video files inside = episodes (sorted by filename).
Art files detected: poster.jpg/png, folder.jpg/png, cover.jpg/png, banner.jpg/png.

## Monorepo Structure

```
cast/
├── server/           # Rust HTTP media server (BUILT)
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs       # CLI args, startup, periodic rescan
│       ├── library.rs    # Media directory scanner
│       ├── db.rs         # SQLite watch progress (stored in .cast.db)
│       ├── routes.rs     # All HTTP endpoints
│       └── mdns.rs       # Bonjour advertisement
├── app/              # tvOS Xcode project (Swift/SwiftUI)
├── PLAN.md
└── CLAUDE.md
```

## Server — COMPLETE

### Dependencies
axum, tokio, tower-http, serde, rusqlite (bundled), walkdir, mdns-sd, clap, uuid, mime_guess, hostname

### API

| Endpoint | Method | Description |
|----------|--------|-------------|
| `GET /api/series` | GET | List all series with watch progress summary |
| `GET /api/series/{id}` | GET | Series detail with all episodes + per-episode progress |
| `GET /api/series/{id}/next` | GET | Smart next episode (resume / next unwatched / first) |
| `GET /api/series/{id}/art` | GET | Series artwork image |
| `GET /api/episodes/{id}/stream` | GET | Stream video (byte-range support for seeking) |
| `GET /api/episodes/{id}/progress` | GET | Get watch progress for episode |
| `POST /api/episodes/{id}/progress` | POST | Update watch progress `{position_secs, duration_secs}` |
| `GET /api/progress` | GET | All watch progress entries |

### Smart "Next Episode" Logic (`/api/series/{id}/next`)
1. If any episode is in-progress (position > 0, not completed) → **resume** it
2. If last completed episode has a successor → return the **next** one
3. If nothing watched → return **first** episode
4. If all watched → returns null with reason `"all_watched"`

### Watch Progress
- Stored in `.cast.db` (SQLite) in the media directory
- Episode marked `completed` when position ≥ 90% of duration
- Single user, no auth

### Running
```bash
cd server
cargo run -- --media /path/to/media
# Optional: --port 3456 --name "Cast Server"
```

### Features
- Periodic library rescan (every 60s)
- Stable episode/series IDs (UUID v5 from relative path — survives rescans)
- Bonjour/mDNS advertisement as `_cast-media._tcp`

---

## tvOS App (`app/`) — TODO

SwiftUI app targeting tvOS 17+.

### Tech choices
- **SwiftUI** — UI framework
- **AVKit / AVPlayer** — video playback (native transport controls)
- **Network framework** — NWBrowser for Bonjour server discovery

### Screens

1. **Server Discovery / Connection**
   - Auto-discover Cast servers via Bonjour (`_cast-media._tcp`)
   - Manual IP entry fallback
   - Remember last connected server

2. **Series Grid**
   - Grid of series with poster art
   - Shows watched/total episode count per series
   - Focus-based navigation

3. **Series Detail → Auto-plays smart next**
   - On selecting a series, call `/api/series/{id}/next`
   - If resuming: show "Resume Episode X at MM:SS?"
   - Episode list visible for manual selection
   - Each episode shows watched/in-progress/unwatched state

4. **Player**
   - Full-screen AVPlayer
   - Native tvOS transport controls
   - Periodically POST progress back to server (every ~10s)
   - On close/finish: POST final progress

### Implementation order
1. Xcode project setup (tvOS target, SwiftUI lifecycle)
2. API client + data models matching server DTOs
3. Server discovery via NWBrowser (Bonjour)
4. Series grid UI with art loading
5. Series detail / episode list
6. Video player with AVPlayer + progress reporting
7. Smart resume flow

---

## Codec / Container Notes

**Natively supported by AVPlayer (no transcoding):**
- Containers: MP4, MOV, M4V
- Video codecs: H.264, HEVC (H.265)
- Audio codecs: AAC, MP3, ALAC, FLAC

**Needs transcoding (future scope):**
- MKV (even with H.264 — needs remuxing at minimum)
- AVI, WMV, VP8/VP9, AV1

v1: only serve natively compatible files. Unsupported files skipped during scan.

---

## Bonjour Service

- Service type: `_cast-media._tcp.local.`
- TXT record: `version=1`
- tvOS app discovers automatically on LAN

---

## Development Workflow

1. Run server: `cargo run -- --media /path/to/shows`
2. Build tvOS app in Xcode, deploy to Apple TV on same LAN
3. App auto-discovers server, browse series, play episodes
