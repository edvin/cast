import Foundation
import Observation

@Observable
final class ServerConnection {
    var baseURL: URL?
    var connectionLost = false

    private static let lastServerKey = "lastServerAddress"
    private static let lastServerMacKey = "lastServerMac"
    private var healthCheckTask: Task<Void, Never>?

    /// Last-known MAC address of the server, stored for Wake-on-LAN attempts.
    var lastKnownMac: String? {
        UserDefaults.standard.string(forKey: Self.lastServerMacKey)
    }

    func connect(host: String, port: UInt16) {
        let hostPart = host.contains(":") ? "[\(host)]" : host
        baseURL = URL(string: "http://\(hostPart):\(port)")
        connectionLost = false
        UserDefaults.standard.set("\(host)|\(port)", forKey: Self.lastServerKey)
        startHealthCheck()
        // Refresh the stored MAC in the background — used for WoL the next time the
        // server is unreachable.
        Task { await fetchAndStoreMac() }
    }

    private func fetchAndStoreMac() async {
        guard let base = baseURL,
              let url = URL(string: "http://\(base.host ?? "")\(base.port.map { ":\($0)" } ?? "")/api/network-info")
        else { return }
        do {
            let (data, _) = try await URLSession.shared.data(from: url)
            struct Info: Decodable { let primary_mac: String? }
            if let info = try? JSONDecoder().decode(Info.self, from: data),
               let mac = info.primary_mac, !mac.isEmpty {
                UserDefaults.standard.set(mac, forKey: Self.lastServerMacKey)
            }
        } catch {}
    }

    func connect(address: String) {
        guard let (host, port) = Self.parseStoredAddress(address) else { return }
        connect(host: host, port: port)
    }

    func disconnect() {
        healthCheckTask?.cancel()
        healthCheckTask = nil
        baseURL = nil
        connectionLost = false
    }

    /// Parses a stored address. Supports both the new "host|port" format (IPv6-safe)
    /// and the legacy "host:port" format.
    private static func parseStoredAddress(_ address: String) -> (String, UInt16)? {
        if let sep = address.firstIndex(of: "|") {
            let host = String(address[..<sep])
            let portStr = address[address.index(after: sep)...]
            guard let p = UInt16(portStr) else { return nil }
            return (host, p)
        }
        // Legacy IPv4/hostname:port format
        guard let colon = address.lastIndex(of: ":"), address.filter({ $0 == ":" }).count == 1 else {
            return (address, 3456)
        }
        let host = String(address[..<colon])
        let portStr = address[address.index(after: colon)...]
        guard let p = UInt16(portStr) else { return nil }
        return (host, p)
    }

    func tryReconnectToLastServer() async -> Bool {
        guard let address = UserDefaults.standard.string(forKey: Self.lastServerKey),
              let (host, port) = Self.parseStoredAddress(address) else {
            return false
        }

        let hostPart = host.contains(":") ? "[\(host)]" : host
        guard let url = URL(string: "http://\(hostPart):\(port)/api/series") else {
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
