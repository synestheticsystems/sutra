import SutraCore
import SwiftUI

/// The bar over the list for what acts on the list: the sort, while there is more than one project
/// to order, and the filter field while ⌘F shows it. It takes no room while it holds neither.
struct ScopeBar: View {
    let dashboard: DashboardModel
    let filterFocus: FocusState<Bool>.Binding

    var body: some View {
        let toolbar = dashboard.toolbar
        if toolbar.sort != nil || toolbar.filter != nil {
            VStack(spacing: 0) {
                HStack(spacing: 10) {
                    if let sort = toolbar.sort {
                        SortPicker(sort: sort, dashboard: dashboard)
                    }
                    Spacer(minLength: 0)
                    if let filter = toolbar.filter {
                        FilterField(initial: filter, dashboard: dashboard, focus: filterFocus)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 6)
                Divider()
            }
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
