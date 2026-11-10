import SutraCore
import SwiftUI

/// The disk Docker takes, in a row of its own under Docker's line, on a tray as a project's card
/// shows its folder's: the total, then the stores large enough to name. Its place under Docker's
/// line says whose disk it is, so it has no title. The socket Docker is read through shows while
/// the pointer is over it, where a card's tray shows its folder.
struct DockerDiskRow: View {
    let tray: Tray
    /// The socket Docker is read through, with `~` for the home folder.
    let location: String

    @State private var pointerOver = false

    var body: some View {
        TrayView(tray: tray, location: location, hovered: pointerOver)
            .clipShape(.rect(cornerRadius: Surface.cornerRadius - 1))
            // Inside the edge, so the tray never covers it.
            .padding(Surface.edgeWidth)
            .background { CardSurface(waiting: false) }
            .onHover { over in
                withAnimation(.easeOut(duration: 0.15)) { pointerOver = over }
            }
    }
}
