import Foundation

@Observable
final class ProgressReporter {
    private var timer: Timer?
    private var client: APIClient?
    private var episodeId: String?

    func start(
        client: APIClient,
        episodeId: String,
        positionProvider: @Sendable @escaping () -> (position: Double, duration: Double)?
    ) {
        self.client = client
        self.episodeId = episodeId

        timer = Timer.scheduledTimer(withTimeInterval: 10.0, repeats: true) { [weak self] _ in
            guard let self,
                  let client = self.client,
                  let episodeId = self.episodeId,
                  let pos = positionProvider() else { return }

            Task {
                try? await client.updateProgress(
                    episodeId: episodeId,
                    position: pos.position,
                    duration: pos.duration
                )
            }
        }
    }

    func stop(finalPosition: Double, finalDuration: Double) {
        timer?.invalidate()
        timer = nil

        guard let client, let episodeId else { return }
        Task {
            try? await client.updateProgress(
                episodeId: episodeId,
                position: finalPosition,
                duration: finalDuration
            )
        }

        self.client = nil
        self.episodeId = nil
    }
}
