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
                } else if connection.baseURL != nil {
                    SeriesGridView()
                } else {
                    ServerDiscoveryView()
                }
            }
            .environment(connection)
        }
    }
}
