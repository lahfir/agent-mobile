import SwiftUI

struct DragHandle: View {
    let isVisible: Bool
    
    var body: some View {
        Image("DragHandle")
            .resizable()
            .scaledToFit()
            .foregroundColor(.white)
            .opacity(isVisible ? 1 : 0)
            .frame(width: 24, height: 24)
            .contentShape(Rectangle())
            .accessibilityLabel("Reorder")
            .accessibilityHint("Drag to reorder this block")
            .onTapGesture {
                if isVisible {
                    Haptics.selection()
                }
            }
    }
} 