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
        VStack(spacing: 30) {
            Image(systemName: "wifi.exclamationmark")
                .font(.system(size: 60))
                .foregroundStyle(.secondary)

            Text("Server Unreachable")
                .font(.title2)
                .bold()

            Text("The Cast server is no longer responding. It may have been shut down or the network changed.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 500)

            HStack(spacing: 30) {
                Button("Try Again") {
                    Task {
                        let ok = await connection.tryReconnectToLastServer()
                        if !ok {
                            connection.connectionLost = true
                        }
                    }
                }
                .buttonStyle(.borderedProminent)

                Button("Change Server") {
                    connection.disconnect()
                }
            }
        }
    }
}
