import SwiftUI

/// A thin bar along an edge, filled from the leading end to `progress`, from 0 to 1, that needs no
/// words beside it. The core reports progress in steps; the bar glides between them.
struct FillBar<Fill: ShapeStyle>: View {
    let progress: Float
    let fill: Fill
    let thickness: CGFloat

    var body: some View {
        Rectangle()
            .fill(Surface.edge)
            .overlay(alignment: .leading) {
                Rectangle()
                    .fill(fill)
                    .scaleEffect(x: CGFloat(min(max(progress, 0), 1)), y: 1, anchor: .leading)
                    .animation(.easeOut(duration: 0.4), value: progress)
            }
            .frame(height: thickness)
    }
}
