import SutraCore
import SwiftUI

/// The bar over the list: the sort, while there is more than one project to order, and at its end
/// the gauge of how full the disk is, or the filter field while ⌘F shows it. It takes no room while
/// it holds none of them.
struct ScopeBar: View {
    let dashboard: DashboardModel
    let filterFocus: FocusState<Bool>.Binding

    var body: some View {
        let toolbar = dashboard.toolbar
        if toolbar.sort != nil || toolbar.filter != nil || toolbar.disk != nil {
            VStack(spacing: 0) {
                HStack(spacing: 10) {
                    if let sort = toolbar.sort {
                        SortPicker(sort: sort, dashboard: dashboard)
                    }
                    Spacer(minLength: 0)
                    // The field the user is typing in takes the end of the bar while it shows.
                    if let filter = toolbar.filter {
                        FilterField(initial: filter, dashboard: dashboard, focus: filterFocus)
                    } else if let disk = toolbar.disk {
                        DiskGaugeView(
                            gauge: disk,
                            chooseDiskSort: toolbar.sort.map { _ in
                                { dashboard.perform(.setSort(order: .disk)) }
                            }
                        )
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 6)
                Divider()
            }
        }
    }
}

/// How full the disk is: a drive, a bar filling with the share in use, and that share. It darkens
/// once little is left, and turns red once almost nothing is; amber stays for an agent that wants
/// the user. A tick on the track marks where the free space Sutra keeps begins. When the disk
/// cannot be read, an alert sign stands in the bar's place. The tooltip names the disk and says
/// how much in bytes, or why it could not be read. The gauge sums up what the Disk sort lists, so
/// a click on it chooses that sort, while the list has a sort to choose.
private struct DiskGaugeView: View {
    let gauge: DiskGauge
    /// Chooses the Disk sort; `nil` while the list has no sort to choose.
    let chooseDiskSort: (() -> Void)?

    private static let barWidth: CGFloat = 56

    var body: some View {
        if let chooseDiskSort {
            Button(action: chooseDiskSort) { reading }
                .buttonStyle(.plain)
        } else {
            reading
        }
    }

    private var reading: some View {
        HStack(spacing: 6) {
            Image(systemName: "internaldrive")
                .foregroundStyle(.secondary)
            switch gauge {
            case .read(let used, let label, _, let look, let floor):
                FillBar(progress: used, fill: Self.fill(look), thickness: 5)
                    .clipShape(.capsule)
                    .overlay(alignment: .leading) {
                        if let floor {
                            FloorTick(at: floor, width: Self.barWidth)
                        }
                    }
                    .frame(width: Self.barWidth)
                Text(label)
                    .font(.callout)
                    // Darker and heavier asks for a look without claiming the user.
                    .fontWeight(look == .tight ? .semibold : .regular)
                    .monospacedDigit()
                    .foregroundStyle(Self.ink(look))
            case .unavailable:
                Image(systemName: "exclamationmark.triangle.fill")
                    .symbolRenderingMode(.multicolor)
            }
        }
        .contentShape(.rect)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(tooltip)
        .hoverTip(tooltip)
    }

    private var tooltip: String {
        switch gauge {
        case .read(_, _, let tooltip, _, _): tooltip
        case .unavailable(let tooltip): tooltip
        }
    }

    private static func fill(_ look: GaugeLook) -> Color {
        switch look {
        case .roomy: .secondary
        case .tight: .primary
        case .full: Color(nsColor: Hue.red.base)
        }
    }

    private static func ink(_ look: GaugeLook) -> Color {
        switch look {
        case .roomy: .secondary
        case .tight: .primary
        case .full: Hue.red.ink
        }
    }
}

/// The mark on the gauge's track where the free space Sutra keeps begins: a hairline in the
/// primary ink, a little taller than the track, so it reads as a mark rather than as fill.
private struct FloorTick: View {
    /// The share of the disk in use at which the kept free space begins, from 0 to 1.
    let at: Float
    let width: CGFloat

    var body: some View {
        Rectangle()
            .fill(.primary)
            .frame(width: 1, height: 9)
            .offset(x: width * CGFloat(min(max(at, 0), 1)) - 0.5)
            .accessibilityHidden(true)
    }
}

/// The orders the list can take, the current one selected, each with what it means in its tooltip.
private struct SortPicker: View {
    let sort: SortControl
    let dashboard: DashboardModel

    var body: some View {
        Picker("", selection: Binding(
            get: { sort.current },
            set: { dashboard.perform(.setSort(order: $0)) }
        )) {
            ForEach(sort.choices, id: \.order) { choice in
                Text(choice.label)
                    .help(choice.tooltip)
                    .tag(choice.order)
            }
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .fixedSize()
    }
}

/// The filter field. What the user types is its own until the field closes: the core's copy of the
/// text comes back a moment behind each key, and taking it would undo what was typed meanwhile.
private struct FilterField: View {
    let dashboard: DashboardModel
    let focus: FocusState<Bool>.Binding

    @State private var text: String

    init(initial: String, dashboard: DashboardModel, focus: FocusState<Bool>.Binding) {
        _text = State(initialValue: initial)
        self.dashboard = dashboard
        self.focus = focus
    }

    var body: some View {
        TextField("Filter by name, branch or path", text: $text)
            .textFieldStyle(.roundedBorder)
            .frame(width: 220)
            .focused(focus)
            // The field shows because the user pressed ⌘F to type in it.
            .onAppear { focus.wrappedValue = true }
            .onChange(of: text) {
                dashboard.perform(.filterChanged(text: text))
            }
    }
}
