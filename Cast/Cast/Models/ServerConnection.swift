import Foundation
import Observation

@Observable
final class ServerConnection {
    var baseURL: URL?
    var connectionLost = false

    private static let lastServerKey = "lastServerAddress"
    private var healthCheckTask: Task<Void, Never>?

    func connect(host: String, port: UInt16) {
        baseURL = URL(string: "http://\(host):\(port)")
        connectionLost = false
        UserDefaults.standard.set("\(host):\(port)", forKey: Self.lastServerKey)
        startHealthCheck()
    }

    func connect(address: String) {
        let parts = address.split(separator: ":")
        let host = String(parts[0])
        let port: UInt16 = parts.count > 1 ? UInt16(parts[1]) ?? 3456 : 3456
        connect(host: host, port: port)
    }

    func disconnect() {
        healthCheckTask?.cancel()
        healthCheckTask = nil
        baseURL = nil
        connectionLost = false
    }

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

    /// Periodic health check — pings the server every 30 seconds
    private func startHealthCheck() {
        healthCheckTask?.cancel()
        healthCheckTask = Task { @MainActor in
            var failCount = 0
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(30))
                guard let url = baseURL?.appendingPathComponent("api/series") else { break }

                do {
                    let (_, response) = try await URLSession.shared.data(from: url)
                    if let http = response as? HTTPURLResponse, http.statusCode == 200 {
                        failCount = 0
                        if connectionLost {
                            connectionLost = false
                        }
                    } else {
                        failCount += 1
                    }
                } catch {
                    failCount += 1
                }

                // After 3 consecutive failures (90 seconds), mark as lost
                if failCount >= 3 {
                    connectionLost = true
                }
            }
        }
    }
}
