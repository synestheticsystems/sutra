import Foundation
import os
import UserNotifications

/// The banners the core asks for, posted through Notification Center. The core plays its own
/// sounds, so the banners are silent.
@MainActor
public final class NotificationBanners: NSObject, UNUserNotificationCenterDelegate {
    private static let log = Logger(subsystem: "systems.synesthetic.sutra", category: "banners")

    /// Whether the user lets Sutra post banners, asked once, on the first banner.
    private var permission: Task<Bool, Never>?

    public override init() {
        super.init()
        // Notification Center serves only a bundled app, and throws at a bare executable.
        if Bundle.main.bundleIdentifier != nil {
            UNUserNotificationCenter.current().delegate = self
        }
    }

    func post(title: String, body: String) {
        guard Bundle.main.bundleIdentifier != nil else {
            Self.log.notice("Unbundled, so no banner: \(title, privacy: .public): \(body, privacy: .public)")
            return
        }
        let center = UNUserNotificationCenter.current()
        let permission = self.permission ?? Task {
            do {
                return try await center.requestAuthorization(options: [.alert])
            } catch {
                Self.log.error("Asking to post banners failed: \(error, privacy: .public)")
                return false
            }
        }
        self.permission = permission
        Task {
            guard await permission.value else { return }
            let content = UNMutableNotificationContent()
            content.title = title
            content.body = body
            let request = UNNotificationRequest(
                identifier: UUID().uuidString,
                content: content,
                trigger: nil
            )
            do {
                try await center.add(request)
            } catch {
                Self.log.error("Posting a banner failed: \(error, privacy: .public)")
            }
        }
    }

    /// The core decides when a banner is worth posting, so it shows even while Sutra is in front.
    public nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        [.banner, .list]
    }
}
