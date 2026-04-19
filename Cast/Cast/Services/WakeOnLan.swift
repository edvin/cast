import Foundation
import Network

/// Sends a Wake-on-LAN magic packet to the local subnet.
/// A magic packet is 6 bytes of 0xFF followed by 16 repetitions of the target's MAC
/// address (6 bytes each), broadcast over UDP on port 9. Most NICs ship their own
/// broadcast listener in hardware — the OS doesn't need to be running, as long as the
/// BIOS and NIC firmware have WoL enabled.
enum WakeOnLan {

    /// Parse a MAC address string like "AA:BB:CC:DD:EE:FF" or "aa-bb-cc-dd-ee-ff"
    /// into 6 bytes. Returns nil for malformed input.
    static func parseMAC(_ mac: String) -> [UInt8]? {
        let cleaned = mac.replacingOccurrences(of: "-", with: ":").uppercased()
        let parts = cleaned.split(separator: ":")
        guard parts.count == 6 else { return nil }
        var bytes: [UInt8] = []
        for p in parts {
            guard p.count == 2, let b = UInt8(p, radix: 16) else { return nil }
            bytes.append(b)
        }
        return bytes
    }

    /// Build the 102-byte magic packet.
    private static func buildPacket(mac: [UInt8]) -> Data {
        var packet = Data(repeating: 0xFF, count: 6)
        for _ in 0..<16 { packet.append(contentsOf: mac) }
        return packet
    }

    /// Broadcast the magic packet to 255.255.255.255:9.
    /// Returns true if we managed to send the packet (not whether the server actually woke).
    @discardableResult
    static func wake(mac: String) async -> Bool {
        guard let bytes = parseMAC(mac) else { return false }
        let packet = buildPacket(mac: bytes)

        // Fire to a few common WoL ports for belt-and-suspenders coverage.
        let ports: [NWEndpoint.Port] = [9, 7, 40000]
        var anyOk = false
        for port in ports {
            if await sendBroadcast(packet, port: port) {
                anyOk = true
            }
        }
        return anyOk
    }

    private static func sendBroadcast(_ packet: Data, port: NWEndpoint.Port) async -> Bool {
        await withCheckedContinuation { continuation in
            let params = NWParameters.udp
            params.allowLocalEndpointReuse = true
            // Enable broadcast on the underlying socket.
            if let ip = params.defaultProtocolStack.internetProtocol as? NWProtocolIP.Options {
                ip.version = .v4
            }
            let endpoint = NWEndpoint.hostPort(
                host: .ipv4(.broadcast), port: port
            )
            let connection = NWConnection(to: endpoint, using: params)
            var finished = false
            let finish: (Bool) -> Void = { ok in
                if !finished {
                    finished = true
                    connection.cancel()
                    continuation.resume(returning: ok)
                }
            }
            connection.stateUpdateHandler = { state in
                switch state {
                case .ready:
                    connection.send(content: packet, completion: .contentProcessed { err in
                        finish(err == nil)
                    })
                case .failed, .cancelled:
                    finish(false)
                default:
                    break
                }
            }
            connection.start(queue: .global(qos: .userInitiated))
            // Bound the wait so one failing port doesn't block the rest.
            Task {
                try? await Task.sleep(for: .seconds(2))
                finish(false)
            }
        }
    }
}
