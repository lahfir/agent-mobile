import SwiftUI

struct DashedDivider: View {
    var body: some View {
        Rectangle()
            .fill(Color.white.opacity(0))
            .frame(height: 1)
            .overlay(
                Rectangle()
                    .strokeBorder(Color.white.opacity(0.2), style: StrokeStyle(lineWidth: 1, dash: [6]))
            )
    }
}