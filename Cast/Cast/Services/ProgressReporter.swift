import Foundation

/// Reports playback progress to the server every 10 seconds while playing, plus a final
/// write on stop. Uses a caller-supplied update closure so it works for both series
/// episodes and movies without knowing their specific API endpoints.
@Observable
final class ProgressReporter {
    typealias Updater = @Sendable (_ position: Double, _ duration: Double) async -> Void

    private var timer: Timer?
    private var updater: Updater?
    private var inflightTasks: [Task<Void, Never>] = []

    func start(
        updater: @escaping Updater,
        positionProvider: @Sendable @escaping () -> (position: Double, duration: Double)?
    ) {
        self.updater = updater
        timer = Timer.scheduledTimer(withTimeInterval: 10.0, repeats: true) { [weak self] _ in
            guard let self,
                  let updater = self.updater,
                  let pos = positionProvider() else { return }
            let task = Task { [weak self] in
                await updater(pos.position, pos.duration)
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

        guard let updater else { return }
        // Fire-and-forget final write; bound by URLSession's timeout so it can't leak indefinitely.
        Task {
            await updater(finalPosition, finalDuration)
        }

        self.updater = nil
    }

    deinit {
        timer?.invalidate()
        for task in inflightTasks { task.cancel() }
    }
}
