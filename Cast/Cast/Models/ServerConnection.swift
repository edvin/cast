import Foundation
import Observation

@Observable
final class ServerConnection {
    var baseURL: URL?

    private static let lastServerKey = "lastServerAddress"

    func connect(host: String, port: UInt16) {
        baseURL = URL(string: "http://\(host):\(port)")
        // Remember for next launch
        UserDefaults.standard.set("\(host):\(port)", forKey: Self.lastServerKey)
    }

    func connect(address: String) {
        let parts = address.split(separator: ":")
        let host = String(parts[0])
        let port: UInt16 = parts.count > 1 ? UInt16(parts[1]) ?? 3456 : 3456
        connect(host: host, port: port)
    }

    func disconnect() {
        baseURL = nil
    }

    /// Try to reconnect to the last known server. Returns true if successful.
    func tryReconnectToLastServer() async -> Bool {
        guard let address = UserDefaults.standard.string(forKey: Self.lastServerKey) else {
            return false
        }
        let parts = address.split(separator: ":")
        let host = String(parts[0])
        let port: UInt16 = parts.count > 1 ? UInt16(parts[1]) ?? 3456 : 3456

        guard let url = URL(string: "http://\(host):\(port)/api/series") else {
            return false
        }

        do {
            let (_, response) = try await URLSession.shared.data(from: url)
            if let http = response as? HTTPURLResponse, http.statusCode == 200 {
                connect(host: host, port: port)
                return true
            }
        } catch {}

        return false
    }
}
