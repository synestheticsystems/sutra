import SutraCore
import SwiftUI

/// One button of a dialog's alert, and how it answers the dialog.
public struct DialogButton: Hashable, Sendable {
    public let title: String
    /// Whether the button answers yes: only a question's action button does.
    public let yes: Bool
    public let role: Role

    public enum Role: Sendable {
        case plain
        /// A yes that deletes build folders, drawn so it reads as one.
        case destructive
        /// Escape presses it, as it presses a question's Cancel.
        case cancel
    }
}

extension Dialog {
    /// A question has its action button and Cancel; a notice has OK, which answers no.
    public var buttons: [DialogButton] {
        if let action {
            [
                DialogButton(title: action, yes: true, role: destructive ? .destructive : .plain),
                DialogButton(title: "Cancel", yes: false, role: .cancel),
            ]
        } else {
            [DialogButton(title: "OK", yes: false, role: .plain)]
        }
    }
}

/// The windows a dialog can show in.
enum DialogPlace {
    /// The window of projects, where the core's dialogs show unless the user is in Settings.
    case dashboard
    case settings
}

extension View {
    /// The core's oldest unanswered dialog, as an alert over this window when the dialog shows
    /// in `place`.
    func dialogAlert(_ dashboard: DashboardModel, in place: DialogPlace) -> some View {
        let dialog = dashboard.dialogPlace == place ? dashboard.dialog : nil
        return alert(
            dialog?.title ?? "",
            isPresented: Binding(
                get: { dashboard.dialogPlace == place && dashboard.dialog != nil },
                // Each way out of the alert is one of its buttons, which answers the dialog, and
                // the answer is what dismisses it.
                set: { _ in }
            ),
            presenting: dialog
        ) { dialog in
            ForEach(dialog.buttons, id: \.self) { button in
                Button(button.title, role: button.role.buttonRole) {
                    dashboard.answer(dialog, yes: button.yes)
                }
            }
        } message: { dialog in
            Text(dialog.detail)
        }
    }
}

extension DialogButton.Role {
    fileprivate var buttonRole: ButtonRole? {
        switch self {
        case .plain: nil
        case .destructive: .destructive
        case .cancel: .cancel
        }
    }
}
