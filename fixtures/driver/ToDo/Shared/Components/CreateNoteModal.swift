import SwiftUI

struct CreateNoteModal: View {
    @Binding var isPresented: Bool
    @State private var noteTitle: String = ""
    @State private var showContent: Bool = false
    let onSave: (String) -> Void
    
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
            
            Text("New Note")
                .font(.system(size: 17, weight: .semibold))
                .foregroundColor(.white)
            
            Spacer()
            
            Button("Create") {
                saveNote()
            }
            .font(.system(size: 17, weight: .semibold))
            .foregroundColor(noteTitle.isEmpty ? .white.opacity(0.3) : .white)
            .disabled(noteTitle.isEmpty)
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
            noteTitleInput
            notePreview
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 32)
    }
    
    private var noteTitleInput: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Note Title")
                .font(.system(size: 14, weight: .medium))
                .foregroundColor(.white.opacity(0.6))
            
            TextField("Enter note title", text: $noteTitle)
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
    
    private var notePreview: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Preview")
                .font(.system(size: 14, weight: .medium))
                .foregroundColor(.white.opacity(0.6))
            
            VStack(alignment: .leading, spacing: 8) {
                Text(noteTitle.isEmpty ? "Untitled Note" : noteTitle)
                    .font(.system(size: 18, weight: .semibold))
                    .foregroundColor(.white)
                    .frame(maxWidth: .infinity, alignment: .leading)
                
                Text(Date().formatted(date: .abbreviated, time: .omitted))
                    .font(.system(size: 14))
                    .foregroundColor(.white.opacity(0.6))
                    .frame(maxWidth: .infinity, alignment: .leading)
                
                Text("Start writing your thoughts...")
                    .font(.system(size: 14))
                    .foregroundColor(.white.opacity(0.4))
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(16)
            .background(
                RoundedRectangle(cornerRadius: 12)
                    .fill(Color.white.opacity(0.02))
                    .overlay(
                        RoundedRectangle(cornerRadius: 12)
                            .stroke(Color.white.opacity(0.05), lineWidth: 1)
                    )
            )
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
            noteTitle = ""
        }
    }
    
    private func saveNote() {
        guard !noteTitle.isEmpty else { return }
        onSave(noteTitle)
        dismissModal()
        Haptics.success()
    }
}

#Preview {
    CreateNoteModal(isPresented: .constant(true)) { title in
        print("Created note: \(title)")
    }
} 