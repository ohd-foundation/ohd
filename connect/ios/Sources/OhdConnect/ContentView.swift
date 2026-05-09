// Entry view for the OHD Connect iOS app.
//
// v0 scaffold: renders a placeholder. The real app surface — log tabs,
// dashboard, grants, pending, cases, emergency settings — lands in the
// implementation phase. See ../../../../SPEC.md.

#if canImport(SwiftUI)
import SwiftUI

public struct ContentView: View {
    public init() {}

    public var body: some View {
        VStack {
            Text("OHD Connect v0")
                .font(.largeTitle)
        }
        .padding()
    }
}

#Preview {
    ContentView()
}
#endif
