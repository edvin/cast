import Foundation

@Observable
final class ProgressReporter {
    private var timer: Timer?
    private var client: APIClient?
    private var episodeId: String?
    private var inflightTasks: [Task<Void, Never>] = []

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

            let task = Task { [weak self] in
                try? await client.updateProgress(
                    episodeId: episodeId,
                    position: pos.position,
                    duration: pos.duration
                )
                self?.inflightTasks.removeAll { $0.isCancelled }
            }
            self.inflightTasks.append(task)
        }
    }

    func stop(finalPosition: Double, finalDuration: Double) {
        timer?.invalidate()
        timer = nil

        for task in inflightTasks { task.cancel() }
        inflightTasks.removeAll()

        guard let client, let episodeId else { return }
        // Fire-and-forget final write; bound by URLSession's timeout so it can't leak indefinitely.
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

    deinit {
        timer?.invalidate()
        for task in inflightTasks { task.cancel() }
    }
}
