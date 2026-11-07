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
                        DiskGaugeView(gauge: disk)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 6)
                Divider()
            }
        }
    }
}

/// How full the disk is: a drive, a bar filling with the share in use, and that share; amber once
/// little is left, red once almost nothing is. Its tooltip says how much in bytes.
private struct DiskGaugeView: View {
    let gauge: DiskGauge

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "internaldrive")
                .foregroundStyle(.secondary)
            FillBar(progress: gauge.used, fill: fill, thickness: 5)
                .clipShape(.capsule)
                .frame(width: 56)
            Text(gauge.label)
                .font(.callout)
                .monospacedDigit()
                .foregroundStyle(ink)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(gauge.tooltip)
        .hoverTip(gauge.tooltip)
    }

    private var fill: Color {
        switch gauge.look {
        case .roomy: .secondary
        case .tight: Color(nsColor: Hue.amber.base)
        case .full: Color(nsColor: Hue.red.base)
        }
    }

    private var ink: Color {
        switch gauge.look {
        case .roomy: .secondary
        case .tight: Hue.amber.ink
        case .full: Hue.red.ink
        }
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
