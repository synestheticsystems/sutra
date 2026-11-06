import SwiftUI

/// The menu bar's commands for the window: ⌘F shows the filter field and gives it the keyboard.
/// It stands in the Edit menu in place of the text-editing commands, whose Find would claim ⌘F;
/// the filter field is the only text the window edits. The Help menu brings back the welcome
/// screen.
public struct DashboardCommands: Commands {
    private let welcome: Welcome

    @FocusedValue(\.showFilter) private var showFilter

    public init(welcome: Welcome) {
        self.welcome = welcome
    }

    public var body: some Commands {
        CommandGroup(replacing: .textEditing) {
            Button("Find\u{2026}") { showFilter?.perform() }
                .keyboardShortcut("f")
                .disabled(showFilter == nil)
        }
        CommandGroup(after: .help) {
            Button("Welcome to Robo Sutra") { welcome.replay() }
        }
    }
}

/// What ⌘F does in the window that has the keyboard.
struct ShowFilter {
    let perform: @MainActor () -> Void
}

extension FocusedValues {
    @Entry var showFilter: ShowFilter?
}
