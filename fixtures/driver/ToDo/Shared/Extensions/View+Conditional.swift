import SwiftUI

public extension View {
    /// Conditionally apply a transform to a `View`.
    /// - Parameters:
    ///   - condition: Boolean that decides whether to apply the transform.
    ///   - transform: Closure returning the modified view.
    /// - Returns: Either the transformed view (when `condition` is true) or the original.
    @ViewBuilder
    func when<Content: View>(_ condition: Bool, transform: (Self) -> Content) -> some View {
        if condition {
            transform(self)
        } else {
            self
        }
    }
} 