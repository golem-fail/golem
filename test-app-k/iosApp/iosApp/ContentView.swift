import UIKit
import SwiftUI
import Shared

/// The Compose Multiplatform counter screen, hosted in SwiftUI.
struct ComposeCounterView: UIViewControllerRepresentable {
    func makeUIViewController(context: Context) -> UIViewController {
        MainViewControllerKt.CounterViewController()
    }

    func updateUIViewController(_ uiViewController: UIViewController, context: Context) {}
}

/// A native SwiftUI home screen that opens the Compose screen, so one app
/// covers both iOS UI layers and the boundary between them.
struct ContentView: View {
    var body: some View {
        NavigationView {
            VStack(alignment: .leading, spacing: 12) {
                Text("Home").font(.largeTitle)
                Text("Native SwiftUI screen")
                NavigationLink("Open counter") {
                    ComposeCounterView()
                }
                Spacer()
            }
            .padding()
        }
        .navigationViewStyle(.stack)
    }
}
