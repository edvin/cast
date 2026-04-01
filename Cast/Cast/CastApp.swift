import SwiftUI

@main
struct CastApp: App {
    @State private var connection = ServerConnection()

    var body: some Scene {
        WindowGroup {
            NavigationStack {
                if connection.baseURL != nil {
                    SeriesGridView()
                } else {
                    ServerDiscoveryView()
                }
            }
            .environment(connection)
        }
    }
}
