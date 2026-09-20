import SwiftUI

struct CreateFolderModal: View {
    @Binding var isPresented: Bool
    @State private var folderName: String = ""
    @State private var selectedColor: Color = .gray
    @State private var showContent: Bool = false
    let onSave: (String, Color) -> Void
    
    private let colors: [Color] = [
        .gray, .pink, .blue, .green, .orange, .purple, .red, .yellow
    ]
    
    var body: some View {
        ZStack {
            Color.black.opacity(showContent ? 0.4 : 0.0)
                .ignoresSafeArea()
                .onTapGesture {
                    dismissModal()
                }
            
            VStack(spacing: 0) {
                modalHeader
                modalContent
                modalFooter
            }
            .background(Color(hex: "191919"))
            .cornerRadius(20)
            .shadow(color: .black.opacity(0.3), radius: 20, y: 10)
            .padding(.horizontal, 20)
            .scaleEffect(showContent ? 1.0 : 0.95)
            .opacity(showContent ? 1.0 : 0.0)
        }
        .onAppear {
            withAnimation(.spring(response: 0.4, dampingFraction: 0.8)) {
                showContent = true
            }
        }
        .onDisappear {
            showContent = false
        }
    }
    
    private var modalHeader: some View {
        HStack {
            Button("Cancel") {
                dismissModal()
            }
            .font(.system(size: 17))
            .foregroundColor(.white.opacity(0.6))
            
            Spacer()
            
            Text("New Folder")
                .font(.system(size: 17, weight: .semibold))
                .foregroundColor(.white)
            
            Spacer()
            
            Button("Save") {
                saveFolder()
            }
            .font(.system(size: 17, weight: .semibold))
            .foregroundColor(folderName.isEmpty ? .white.opacity(0.3) : .white)
            .disabled(folderName.isEmpty)
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 16)
        .background(
            Rectangle()
                .fill(Color.white.opacity(0.05))
        )
    }
    
    private var modalContent: some View {
        VStack(spacing: 24) {
            folderNameInput
            colorPicker
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 32)
    }
    
    private var folderNameInput: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Folder Name")
                .font(.system(size: 14, weight: .medium))
                .foregroundColor(.white.opacity(0.6))
            
            TextField("Enter folder name", text: $folderName)
                .font(.system(size: 16))
                .foregroundColor(.white)
                .padding(.horizontal, 16)
                .frame(height: 48)
                .background(
                    RoundedRectangle(cornerRadius: 12)
                        .fill(Color.white.opacity(0.05))
                        .overlay(
                            RoundedRectangle(cornerRadius: 12)
                                .stroke(Color.white.opacity(0.1), lineWidth: 1)
                        )
                )
        }
    }
    
    private var colorPicker: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Color")
                .font(.system(size: 14, weight: .medium))
                .foregroundColor(.white.opacity(0.6))
            
            LazyVGrid(columns: Array(repeating: GridItem(.flexible()), count: 4), spacing: 12) {
                ForEach(colors, id: \.self) { color in
                    Button(action: {
                        selectedColor = color
                        Haptics.selection()
                    }) {
                        Circle()
                            .fill(color)
                            .frame(width: 40, height: 40)
                            .overlay(
                                Circle()
                                    .stroke(Color.white, lineWidth: selectedColor == color ? 2 : 0)
                            )
                            .scaleEffect(selectedColor == color ? 1.1 : 1.0)
                    }
                    .animation(.spring(response: 0.3, dampingFraction: 0.7), value: selectedColor)
                }
            }
        }
    }
    
    private var modalFooter: some View {
        Rectangle()
            .fill(Color.white.opacity(0.05))
            .frame(height: 1)
    }
    
    private func dismissModal() {
        withAnimation(.spring(response: 0.35, dampingFraction: 0.8)) {
            showContent = false
        }
        
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) {
            isPresented = false
            folderName = ""
            selectedColor = .gray
        }
    }
    
    private func saveFolder() {
        guard !folderName.isEmpty else { return }
        onSave(folderName, selectedColor)
        dismissModal()
        Haptics.success()
    }
}

#Preview {
    CreateFolderModal(isPresented: .constant(true)) { name, color in
        print("Created folder: \(name) with color: \(color)")
    }
} 