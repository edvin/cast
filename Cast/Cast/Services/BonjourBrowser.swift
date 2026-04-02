import Foundation
import Network
import Observation

struct DiscoveredServer: Identifiable, Hashable {
    let id: String
    let name: String
    let host: String
    let port: UInt16
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
            if case .service(let name, let type, let domain, _) = result.endpoint {
                discovered.append(DiscoveredServer(
                    id: "\(name).\(type)\(domain)",
                    name: name,
                    host: "",
                    port: 0
                ))
            }
        }

        servers = discovered
    }

    func resolve(_ server: DiscoveredServer, completion: @escaping @Sendable (String?, UInt16) -> Void) {
        guard let results = browser?.browseResults else {
            completion(nil, 0)
            return
        }
        guard let result = results.first(where: {
            if case .service(let name, _, _, _) = $0.endpoint {
                return name == server.name
            }
            return false
        }) else {
            completion(nil, 0)
            return
        }

        var completed = false
        let connection = NWConnection(to: result.endpoint, using: .tcp)
        connection.stateUpdateHandler = { state in
            guard !completed else { return }
            switch state {
            case .ready:
                completed = true
                if let path = connection.currentPath,
                   let endpoint = path.remoteEndpoint,
                   case .hostPort(let host, let port) = endpoint {
                    let rawHost: String
                    switch host {
                    case .ipv4(let addr):
                        rawHost = "\(addr)"
                    case .ipv6(let addr):
                        rawHost = "\(addr)"
                    case .name(let name, _):
                        rawHost = name
                    @unknown default:
                        rawHost = "\(host)"
                    }
                    // Strip interface scope (e.g. "%en0") from resolved address
                    let hostString = rawHost.split(separator: "%").first.map(String.init) ?? rawHost
                    DispatchQueue.main.async {
                        completion(hostString, port.rawValue)
                    }
                } else {
                    DispatchQueue.main.async { completion(nil, 0) }
                }
                connection.cancel()
            case .failed, .cancelled:
                completed = true
                DispatchQueue.main.async { completion(nil, 0) }
            default:
                break
            }
        }
        connection.start(queue: .global())

        // Timeout after 5 seconds
        DispatchQueue.global().asyncAfter(deadline: .now() + 5) {
            guard !completed else { return }
            completed = true
            connection.cancel()
            DispatchQueue.main.async { completion(nil, 0) }
        }
    }
}
