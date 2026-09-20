import SwiftUI

/// A read-only task row used when the note is in view-only mode.
struct TaskItemView: View {
    let task: Task
    let onToggle: () -> Void
    
    var body: some View {
        HStack(spacing: 12) {
            Button(action: {
                Haptics.selection()
                onToggle()
            }) {
                Image(systemName: task.isDone ? "checkmark.circle.fill" : "circle")
                    .font(.system(size: 20))
                    .foregroundColor(task.isDone ? .primaryAccent : .white.opacity(0.6))
            }
            .accessibilityLabel(task.isDone ? "Completed task" : "Incomplete task")
            .accessibilityHint("Tap to toggle completion")
            
            Text(task.text)
                .font(.system(size: 16))
                .foregroundColor(.white)
                .strikethrough(task.isDone)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .animation(.easeInOut(duration: 0.25), value: task.isDone)
    }
}

/// A task row that becomes editable when tapped, used inside editable task lists.
struct EditableTaskItemView: View {
    var task: Task
    let onToggle: () -> Void
    let onUpdate: (Task) -> Void
    
    @State private var isEditing = false
    @State private var editingText = ""
    
    var body: some View {
        HStack(spacing: 12) {
            Button(action: {
                Haptics.selection()
                onToggle()
            }) {
                Image(systemName: task.isDone ? "checkmark.circle.fill" : "circle")
                    .font(.system(size: 20))
                    .foregroundColor(task.isDone ? .primaryAccent : .white.opacity(0.6))
            }
            .accessibilityLabel(task.isDone ? "Completed task" : "Incomplete task")
            .accessibilityHint("Tap to toggle completion")
            
            if isEditing {
                TextField("Task", text: $editingText)
                    .font(.system(size: 16))
                    .foregroundColor(.white)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .onSubmit {
                        var updatedTask = task
                        updatedTask.text = editingText
                        onUpdate(updatedTask)
                        isEditing = false
                    }
                    .onAppear { editingText = task.text }
                    .transaction { $0.animation = nil }
            } else {
                Text(task.text)
                    .font(.system(size: 16))
                    .foregroundColor(.white)
                    .strikethrough(task.isDone)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .onTapGesture {
                        editingText = task.text
                        isEditing = true
                    }
            }
        }
        .animation(.easeInOut(duration: 0.25), value: task.isDone)
    }
} 