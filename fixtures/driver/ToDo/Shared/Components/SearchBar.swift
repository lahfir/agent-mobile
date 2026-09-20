import SwiftUI

struct SearchBar: View {
    @Binding var text: String
    var onTap: (() -> Void)? = nil
    
    var body: some View {
        HStack {
            TextField("Search", text: $text)
                .font(.system(size: 16))
                .foregroundColor(.white)
                .autocapitalization(.none)
                .disableAutocorrection(true)
            Image(systemName: "magnifyingglass")
                .foregroundColor(.white.opacity(0.6))
        }
        .padding(.horizontal, 16)
        .frame(height: 48)
        .background(
            RoundedRectangle(cornerRadius: 16)
                .strokeBorder(Color.white.opacity(0.2), style: StrokeStyle(lineWidth: 1, dash: [6]))
        )
        .onTapGesture {
            onTap?()
        }
    }
} 