import SwiftUI

struct FolderDetailView: View {
    let folder: Folder
    @Environment(\.dismiss) private var dismiss
    @State private var showCreateNoteModal = false
    @State private var navigateToNote: Note?
    
    var body: some View {
        ZStack {
            backgroundLayer
            statusBarBackground
            contentLayer
        }
        .navigationBarHidden(true)
        .overlay(
            Group {
                if showCreateNoteModal {
                    CreateNoteModal(isPresented: $showCreateNoteModal) { title in
                        createNewNote(title: title)
                    }
                }
            }
        )
        .background(
            NavigationLink(
                destination: navigateToNote.map { NoteView(note: $0) },
                isActive: Binding(
                    get: { navigateToNote != nil },
                    set: { if !$0 { navigateToNote = nil } }
                )
            ) { EmptyView() }
        )
    }
    
    private var backgroundLayer: some View {
        Color(hex: "191919").ignoresSafeArea()
    }
    
    private var statusBarBackground: some View {
        GeometryReader { geo in
            Color.white.opacity(0.08)
                .frame(height: geo.safeAreaInsets.top)
                .ignoresSafeArea(edges: .top)
        }
    }
    
    private var contentLayer: some View {
        VStack(spacing: 0) {
            navigationBar
            scrollContent
        }
        .ignoresSafeArea(edges: .horizontal)
    }
    
    private var navigationBar: some View {
        HStack {
            Button(action: { dismiss() }) {
                HStack(spacing: 4) {
                    Image(systemName: "chevron.left")
                        .font(.system(size: 16, weight: .medium))
                    Text("Back")
                        .font(.system(size: 17, weight: .medium))
                        .foregroundColor(.white.opacity(0.3))
                }
                .foregroundColor(.white)
            }
            .accessibilityLabel("Back")
            
            Spacer()
            
            Text(folder.name)
                .font(.system(size: 17, weight: .semibold))
                .foregroundColor(.white)
                .lineLimit(1)
            
            Spacer()
            
            Button(action: {
                showCreateNoteModal = true
            }) {
                Image(systemName: "plus")
                    .font(.system(size: 20, weight: .bold))
                    .foregroundColor(.white)
            }
            .accessibilityLabel("Add Note")
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 12)
        .background(
            Rectangle()
                .fill(.white.opacity(0.08))
        )
    }
    
    private var scrollContent: some View {
        ScrollView {
            VStack(spacing: 16) {
                headerSection
                notesListSection
                Spacer(minLength: 100)
            }
            .padding(.horizontal, 20)
            .padding(.top, 20)
        }
    }
    
    private var headerSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(folder.name)
                .font(.system(size: 34, weight: .bold))
                .foregroundColor(.white)
                .frame(maxWidth: .infinity, alignment: .leading)
            
            Text("\(folder.notes.count) notes")
                .font(.system(size: 16))
                .foregroundColor(.white.opacity(0.6))
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.bottom, 20)
    }
    
    private var notesListSection: some View {
        let columns = [
            GridItem(.flexible(), spacing: 12),
            GridItem(.flexible(), spacing: 12)
        ]
        
        return LazyVGrid(columns: columns, spacing: 16) {
            ForEach(folder.notes) { note in
                NavigationLink(destination: NoteView(note: note)) {
                    NoteCardView(note: note)
                }
                .buttonStyle(.plain)
            }
        }
    }
    
    private func createNewNote(title: String) {
        let newNote = Note(
            title: title,
            dateCreated: Date(),
            blocks: [Block(kind: .paragraph("Start writing your thoughts..."))]
        )
        navigateToNote = newNote
    }
}

struct NoteCardView: View {
    let note: Note
    
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Image("NoteIcon")
                    .resizable()
                    .scaledToFit()
                    .frame(width: 28, height: 28)
                    .opacity(0.8)
                
                Spacer()
                
                Text(note.formattedDate)
                    .font(.system(size: 12, weight: .medium))
                    .foregroundColor(.white.opacity(0.5))
            }
            
            VStack(alignment: .leading, spacing: 8) {
                Text(note.title)
                    .font(.system(size: 16, weight: .bold))
                    .foregroundColor(.white)
                    .lineLimit(2)
                    .frame(maxWidth: .infinity, alignment: .leading)
                
                if let firstBlock = note.blocks.first {
                    Text(previewText(from: firstBlock))
                        .font(.system(size: 13))
                        .foregroundColor(.white.opacity(0.7))
                        .lineLimit(3)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            
            Spacer()
        }
        .padding(16)
        .frame(minHeight: 140)
        .background(
            RoundedRectangle(cornerRadius: 16)
                .fill(
                    LinearGradient(
                        gradient: Gradient(colors: [
                            Color.white.opacity(0.08),
                            Color.white.opacity(0.03)
                        ]),
                        startPoint: .topLeading,
                        endPoint: .bottomTrailing
                    )
                )
        )
        .overlay(
            RoundedRectangle(cornerRadius: 16)
                .stroke(
                    LinearGradient(
                        gradient: Gradient(colors: [
                            Color.white.opacity(0.15),
                            Color.white.opacity(0.05)
                        ]),
                        startPoint: .topLeading,
                        endPoint: .bottomTrailing
                    ),
                    lineWidth: 1
                )
        )
        .shadow(color: Color.black.opacity(0.1), radius: 8, x: 0, y: 4)
        .accessibilityLabel("Note: \(note.title), created \(note.formattedDate)")
        .accessibilityHint("Tap to view full note")
    }
    
    private func previewText(from block: Block) -> String {
        switch block.kind {
        case .paragraph(let text):
            return text
        case .heading(_, let text):
            return text
        case .taskList(let tasks):
            return tasks.first?.text ?? "Task list"
        case .taskItem(let task):
            return task.text
        case .image:
            return "Image"
        }
    }
}

#Preview {
    FolderDetailView(folder: Folder(name: "Sample Folder", notes: []))
} 