import SwiftUI

extension View {
    func safeAreaBlur(edge: VerticalEdge, content: @escaping () -> some View) -> some View {
        self.safeAreaInset(edge: edge, content: content)
    }
} 