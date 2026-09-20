import SwiftUI

struct BlockView: View {
    let block: Block
    let editMode: Bool
    let onTaskToggle: (UUID) -> Void
    let onBlockUpdate: (Block) -> Void
    let onDelete: (UUID) -> Void
    let onMoveBlock: (UUID, UUID, Bool) -> Void
    @State private var isEditing = false
    @State private var editingText = ""
    @State private var isDropTargeted: Bool = false
    @State private var indicatorPulse: Bool = false
    @State private var blockHeight: CGFloat = 0
    @FocusState private var isTextFieldFocused: Bool
    
    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            contentView
            
            Spacer()
            
            DragHandle(isVisible: editMode)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.vertical, isDropTargeted ? 12 : 6)
        .animation(.spring(response: 0.25, dampingFraction: 0.8), value: isDropTargeted)
        .when(editMode && !isEditing) { view in
            view
                .draggable(block.id, preview: {
                    dragPreview
                })
                .dropDestination(for: UUID.self) { items, location in
                    guard let draggedId = items.first else { return false }
                    let dropAbove = location.y < blockHeight / 2
                    onMoveBlock(draggedId, block.id, !dropAbove)
                    return true
                } isTargeted: { hovering in
                    withAnimation(.easeInOut(duration: 0.15)) {
                        isDropTargeted = hovering
                    }
                }
        }
        .overlay(alignment: .top) {
            if isDropTargeted {
                Rectangle()
                    .fill(Color.primaryAccent)
                    .frame(height: 3)
                    .padding(.vertical, 6)
                    .scaleEffect(x: isDropTargeted ? 1 : 0.1, y: 1, anchor: .center)
                    .opacity(isDropTargeted ? 0.8 : 0)
                    .animation(.easeInOut(duration: 0.15), value: isDropTargeted)
                    .overlay(
                        Rectangle()
                            .fill(Color.primaryAccent)
                            .frame(height: 3)
                            .padding(.vertical, 6)
                            .mask(
                                Rectangle()
                                    .scaleEffect(x: indicatorPulse ? 1 : 0, y: 1, anchor: .center)
                                    .animation(.easeInOut(duration: 0.6).repeatForever(autoreverses: true), value: indicatorPulse)
                            )
                            .onAppear { indicatorPulse = true }
                    )
                    .transition(.opacity)
            }
        }
        .background(
            GeometryReader { geo in
                Color.clear
                    .onAppear { blockHeight = geo.size.height }
                    .onChange(of: geo.size.height) { blockHeight = $0 }
            }
        )
        .onChange(of: editMode) { newValue in
            if !newValue {
                commitPendingEdits()
            }
        }
        .onDisappear {
            commitPendingEdits()
        }
    }
    
    @ViewBuilder
    private var contentView: some View {
        switch block.kind {
        case .paragraph(let text):
            Group {
                if isEditing {
                    TextField("Paragraph", text: $editingText, axis: .vertical)
                        .font(.system(size: 16))
                        .foregroundColor(.white)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .focused($isTextFieldFocused)
                        .onSubmit {
                            var updated = block
                            updated.kind = .paragraph(editingText)
                            onBlockUpdate(updated)
                            isEditing = false
                        }
                        .onAppear { editingText = text; DispatchQueue.main.async { isTextFieldFocused = true } }
                        .transaction { $0.animation = nil }
                } else {
                    Text(text)
                        .font(.system(size: 16))
                        .foregroundColor(.white)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .accessibilityLabel("Paragraph: \(text)")
                        .onTapGesture {
                            editingText = text
                            withAnimation(nil) { isEditing = true }
                            DispatchQueue.main.async { isTextFieldFocused = true }
                        }
                }
            }
            .onAppear {
                if text.isEmpty {
                    isEditing = true
                }
            }
            
        case .heading(let level, let text):
            Group {
                if isEditing {
                    TextField("Heading", text: $editingText, axis: .vertical)
                        .font(.system(size: level.fontSize, weight: level.fontWeight))
                        .foregroundColor(.white)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .focused($isTextFieldFocused)
                        .onSubmit {
                            var updated = block
                            updated.kind = .heading(level: level, text: editingText)
                            onBlockUpdate(updated)
                            isEditing = false
                        }
                        .onAppear { editingText = text; DispatchQueue.main.async { isTextFieldFocused = true } }
                        .transaction { $0.animation = nil }
                } else {
                    Text(text)
                        .font(.system(size: level.fontSize, weight: level.fontWeight))
                        .foregroundColor(.white)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .accessibilityLabel("Heading \(level.rawValue): \(text)")
                        .onTapGesture {
                            editingText = text
                            withAnimation(nil) { isEditing = true }
                            DispatchQueue.main.async { isTextFieldFocused = true }
                        }
                }
            }
            .onAppear {
                if text.isEmpty {
                    isEditing = true
                }
            }
            
        case .image(let uiImage):
            Image(uiImage: uiImage)
                .resizable()
                .aspectRatio(contentMode: .fit)
                .frame(maxWidth: .infinity)
                .cornerRadius(12)
                .accessibilityLabel("Image")
            
        case .taskList(let tasks):
            TaskListBlockView(
                tasks: tasks,
                editMode: editMode,
                onTaskToggle: onTaskToggle,
                onTaskUpdate: { updatedTasks in
                    var updated = block
                    updated.kind = .taskList(updatedTasks)
                    onBlockUpdate(updated)
                }
            )
            
        case .taskItem(let task):
            TaskItemView(
                task: task,
                onToggle: { onTaskToggle(task.id) }
            )
        }
    }
    
    // Commit pending text edits when external edit mode turns off or view disappears
    private func commitPendingEdits() {
        guard isEditing else { return }
        switch block.kind {
        case .paragraph:
            var updated = block
            updated.kind = .paragraph(editingText)
            onBlockUpdate(updated)
        case .heading(let level, _):
            var updated = block
            updated.kind = .heading(level: level, text: editingText)
            onBlockUpdate(updated)
        default:
            break
        }
        isEditing = false
    }
}

// MARK: - Drag Preview

extension BlockView {
    private var dragPreview: some View {
        contentView
            .padding(12)
            .background(
                RoundedRectangle(cornerRadius: 12)
                    .fill(Color.white.opacity(0.1))
                    .blur(radius: 3)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 12)
                    .stroke(Color.primaryAccent.opacity(0.8), lineWidth: 1)
            )
            .shadow(color: .black.opacity(0.4), radius: 4, y: 2)
    }
}