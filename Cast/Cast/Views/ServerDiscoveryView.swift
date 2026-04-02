import SwiftUI

struct ServerDiscoveryView: View {
    @Environment(ServerConnection.self) private var connection
    @State private var browser = BonjourBrowser()
    @State private var showManualEntry = false
    @State private var manualAddress = "127.0.0.1:3456"
    @State private var isConnecting = false
    @State private var errorMessage: String?

    var body: some View {
        if showManualEntry {
            manualEntryView
        } else {
            discoveryView
        }
    }

    // MARK: - Discovery View

    private var discoveryView: some View {
        VStack(spacing: 40) {
            Spacer()

            VStack(spacing: 16) {
                Image(systemName: "tv.and.mediabox")
                    .font(.system(size: 64))
                    .foregroundStyle(.secondary)

                Text("Cast")
                    .font(.largeTitle)
                    .bold()

                if browser.servers.isEmpty {
                    ProgressView()
                        .padding(.top, 8)
                    Text("Looking for Cast servers...")
                        .font(.headline)
                        .foregroundStyle(.secondary)
                } else {
                    Text("Select a server")
                        .font(.headline)
                        .foregroundStyle(.secondary)
                }
            }

            if !browser.servers.isEmpty {
                VStack(spacing: 20) {
                    ForEach(browser.servers) { server in
                        Button {
                            connectTo(server)
                        } label: {
                            HStack {
                                Image(systemName: "server.rack")
                                    .font(.title3)
                                Text(server.name)
                                    .font(.title3)
                                Spacer()
                                Image(systemName: "chevron.right")
                                    .foregroundStyle(.secondary)
                            }
                            .padding(.horizontal, 24)
                            .padding(.vertical, 16)
                        }
                    }
                }
                .padding(.horizontal, 200)
            }

            Button {
                showManualEntry = true
            } label: {
                HStack {
                    Image(systemName: "keyboard")
                    Text("Enter server address manually")
                }
            }
            .padding(.top, 20)

            if isConnecting {
                ProgressView("Connecting...")
            }

            if let errorMessage {
                Text(errorMessage)
                    .foregroundStyle(.red)
                    .font(.subheadline)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 80)
            }

            Spacer()
        }
        .onAppear { browser.startBrowsing() }
        .onDisappear { browser.stopBrowsing() }
    }

    // MARK: - Manual Entry View

    private var manualEntryView: some View {
        VStack(spacing: 40) {
            Spacer()

            Text("Connect to Server")
                .font(.title2)
                .bold()

            Text("Enter the server IP address and port")
                .font(.headline)
                .foregroundStyle(.secondary)

            TextField("192.168.1.50:3456", text: $manualAddress)
                .textFieldStyle(.plain)
                .frame(maxWidth: 500)
                .padding()

            HStack(spacing: 40) {
                Button("Connect") {
                    connectManually()
                }
                .disabled(manualAddress.isEmpty)

                Button("Cancel") {
                    showManualEntry = false
                    manualAddress = ""
                }
            }

            if isConnecting {
                ProgressView("Connecting...")
            }

            if let errorMessage {
                Text(errorMessage)
                    .foregroundStyle(.red)
                    .font(.subheadline)
            }

            Spacer()
        }
    }

    // MARK: - Connection Logic

    private func connectTo(_ server: DiscoveredServer) {
        isConnecting = true
        errorMessage = nil
        browser.resolve(server) { host, port in
            if let host {
                print("[Cast] Resolved \(server.name) → \(host):\(port)")
                Task {
                    await validateAndConnect(host: host, port: port)
                }
            } else {
                isConnecting = false
                errorMessage = "Could not resolve server address. Try entering the IP manually."
            }
        }
    }

    private func connectManually() {
        let parts = manualAddress.split(separator: ":")
        let host = String(parts[0])
        let port: UInt16 = parts.count > 1 ? UInt16(parts[1]) ?? 3456 : 3456
        isConnecting = true
        errorMessage = nil
        Task {
            await validateAndConnect(host: host, port: port)
        }
    }

    private func validateAndConnect(host: String, port: UInt16) async {
        let urlString = "http://\(host):\(port)/api/series"
        print("[Cast] Validating connection to \(urlString)")
        guard let url = URL(string: urlString) else {
            isConnecting = false
            errorMessage = "Invalid address: \(urlString)"
            return
        }
        do {
            let (_, response) = try await URLSession.shared.data(from: url)
            if let http = response as? HTTPURLResponse, http.statusCode == 200 {
                print("[Cast] Connected successfully to \(host):\(port)")
                connection.connect(host: host, port: port)
            } else {
                let code = (response as? HTTPURLResponse)?.statusCode ?? -1
                errorMessage = "Server returned status \(code)"
            }
        } catch {
            print("[Cast] Connection failed: \(error)")
            errorMessage = "Could not connect to \(host):\(port) — \(error.localizedDescription)"
        }
        isConnecting = false
    }
}
