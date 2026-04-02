import SwiftUI

struct ErrorView: View {
    let title: String
    let message: String
    let detail: String?
    let onRetry: (() -> Void)?
    let onDismiss: (() -> Void)?

    init(
        title: String,
        message: String,
        detail: String? = nil,
        onRetry: (() -> Void)? = nil,
        onDismiss: (() -> Void)? = nil
    ) {
        self.title = title
        self.message = message
        self.detail = detail
        self.onRetry = onRetry
        self.onDismiss = onDismiss
    }

    var body: some View {
        VStack(spacing: 40) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 80))
                .foregroundColor(.orange)

            Text(title)
                .font(.title)
                .multilineTextAlignment(.center)

            Text(message)
                .font(.body)
                .foregroundColor(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 600)

            if let detail {
                Text(detail)
                    .font(.caption)
                    .foregroundColor(.secondary)
                    .multilineTextAlignment(.center)
            }

            HStack(spacing: 40) {
                if let onRetry {
                    Button("Try Again") { onRetry() }
                        .buttonStyle(.borderedProminent)
                }
                if let onDismiss {
                    Button("Go Back") { onDismiss() }
                }
            }
        }
        .padding(60)
    }
}
