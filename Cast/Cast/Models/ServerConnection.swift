import Foundation
import Observation

@Observable
final class ServerConnection {
    var baseURL: URL?

    func connect(host: String, port: UInt16) {
        baseURL = URL(string: "http://\(host):\(port)")
    }

    func connect(address: String) {
        let parts = address.split(separator: ":")
        let host = String(parts[0])
        let port: UInt16 = parts.count > 1 ? UInt16(parts[1]) ?? 3456 : 3456
        connect(host: host, port: port)
    }
}
