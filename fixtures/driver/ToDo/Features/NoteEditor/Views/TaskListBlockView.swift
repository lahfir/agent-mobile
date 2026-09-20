import SwiftUI

struct TaskListBlockView: View {
    var tasks: [Task]
    let editMode: Bool
    let onTaskToggle: (UUID) -> Void
    let onTaskUpdate: ([Task]) -> Void
    
    var body: some View {
        VStack(spacing: 12) {
            ForEach(Array(tasks.enumerated()), id: \.element.id) { index, task in
                EditableTaskItemView(
                    task: task,
                    onToggle: { onTaskToggle(task.id) },
                    onUpdate: { updatedTask in
                        var newTasks = tasks
                        newTasks[index] = updatedTask
                        onTaskUpdate(newTasks)
                    }
                )
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityLabel("Task list with \(tasks.count) items")
    }
} 