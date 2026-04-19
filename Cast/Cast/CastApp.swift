import SwiftUI

@main
struct CastApp: App {
    @State private var connection = ServerConnection()
    @State private var isCheckingLastServer = true

    var body: some Scene {
        WindowGroup {
            NavigationStack {
                if isCheckingLastServer {
                    ProgressView("Connecting...")
                        .task {
                            let connected = await connection.tryReconnectToLastServer()
                            isCheckingLastServer = false
                            if !connected {
                                // Will show discovery view
                            }
                        }
                } else if connection.connectionLost {
                    connectionLostView
                } else if connection.baseURL != nil {
                    SeriesGridView()
                } else {
                    ServerDiscoveryView()
                }
            }
            .environment(connection)
        }
    }

    private var connectionLostView: some View {
        ConnectionLostView(connection: connection)
    }
}

private struct ConnectionLostView: View {
    let connection: ServerConnection
    @State private var waking = false
    @State private var wakeStatus: String?

    var body: some View {
        VStack(spacing: 30) {
            Image(systemName: waking ? "bolt.fill" : "wifi.exclamationmark")
                .font(.system(size: 60))
                .foregroundStyle(waking ? .yellow : .secondary)

            Text(waking ? "Waking server..." : "Server Unreachable")
                .font(.title2)
                .bold()

            if let status = wakeStatus {
                Text(status)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 500)
            } else {
                Text("The Cast server is no longer responding. It may be asleep, shut down, or the network changed.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 500)
            }

            HStack(spacing: 30) {
                Button("Try Again") {
                    Task {
                        let ok = await connection.tryReconnectToLastServer()
                        if !ok { connection.connectionLost = true }
                    }
                }
                .buttonStyle(.borderedProminent)
                .disabled(waking)

                if let mac = connection.lastKnownMac {
                    Button {
                        Task { await attemptWake(mac: mac) }
                    } label: {
                        HStack(spacing: 8) {
                            Image(systemName: "bolt.fill")
                            Text("Wake Server")
                        }
                    }
                    .disabled(waking)
                }

                Button("Change Server") {
                    connection.disconnect()
                }
                .disabled(waking)
            }
        }
    }

    private func attemptWake(mac: String) async {
        waking = true
        wakeStatus = "Sending magic packet to \(mac)..."
        let sent = await WakeOnLan.wake(mac: mac)
        if !sent {
            waking = false
            wakeStatus = "Could not send the magic packet. Check the MAC address."
            return
        }
        wakeStatus = "Magic packet sent. Waiting for the server to come online..."
        // Poll for up to ~45 seconds — typical BIOS + Windows wake takes 5–20 seconds.
        for _ in 0..<30 {
            try? await Task.sleep(for: .seconds(1.5))
            if await connection.tryReconnectToLastServer() {
                waking = false
                wakeStatus = nil
                return
            }
        }
        waking = false
        wakeStatus = "Server didn't come online. Is WoL enabled in its BIOS / NIC / Windows power plan?"
    }
}
