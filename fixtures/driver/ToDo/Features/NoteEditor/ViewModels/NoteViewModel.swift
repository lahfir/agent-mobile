import SwiftUI

@MainActor
class NoteViewModel: ObservableObject {
    @Published var blocks: [Block] = []
    @Published var editMode: Bool = false
    
    let title: String
    let date: String
    
    private var history: [[Block]] = []
    private var redoStack: [[Block]] = []
    
    // Published flags for toolbar UI
    @Published private(set) var canUndo: Bool = false
    @Published private(set) var canRedo: Bool = false
    @Published var lastAddedBlockId: UUID? = nil
    
    init(note: Note) {
        self.title = note.title
        self.date = note.formattedDate
        self.blocks = note.blocks
        refreshUndoRedoFlags()
    }
    
    private func recordState() {
        history.append(blocks)
        if history.count > 100 { history.removeFirst() }
        redoStack.removeAll()
        refreshUndoRedoFlags()
    }
    
    func undo() {
        guard let last = history.popLast() else { return }
        redoStack.append(blocks)
        withAnimation(.easeInOut(duration: 0.25)) {
            blocks = last
        }
        refreshUndoRedoFlags()
    }
    
    func redo() {
        guard let next = redoStack.popLast() else { return }
        history.append(blocks)
        withAnimation(.easeInOut(duration: 0.25)) {
            blocks = next
        }
        refreshUndoRedoFlags()
    }
    
    func addBlock(_ block: Block) {
        recordState()
        withAnimation(.easeInOut(duration: 0.25)) {
            blocks.append(block)
        }
        lastAddedBlockId = block.id
    }
    
    func updateBlock(_ updatedBlock: Block) {
        if let index = blocks.firstIndex(where: { $0.id == updatedBlock.id }) {
            recordState()
            withAnimation(.easeInOut(duration: 0.25)) {
                blocks[index] = updatedBlock
            }
        }
    }
    
    func moveBlocks(from source: IndexSet, to destination: Int) {
        recordState()
        withAnimation(.easeInOut(duration: 0.25)) {
            blocks.move(fromOffsets: source, toOffset: destination)
        }
    }
    
    func toggleTaskInList(blockId: UUID, taskId: UUID) {
        guard let blockIndex = blocks.firstIndex(where: { $0.id == blockId }),
              case .taskList(var tasks) = blocks[blockIndex].kind,
              let taskIndex = tasks.firstIndex(where: { $0.id == taskId }) else { return }
        
        recordState()
        tasks[taskIndex].isDone.toggle()
        withAnimation(.easeInOut(duration: 0.25)) {
            blocks[blockIndex].kind = .taskList(tasks)
        }
    }
    
    func moveBlock(_ draggedId: UUID, relativeTo targetId: UUID, after: Bool) {
        guard let fromIndex = blocks.firstIndex(where: { $0.id == draggedId }),
              let toIndex = blocks.firstIndex(where: { $0.id == targetId }) else { return }
        if fromIndex == toIndex { return }
        recordState()
        let block = blocks.remove(at: fromIndex)
        var insertionIndex: Int
        if after {
            insertionIndex = fromIndex < toIndex ? toIndex : toIndex + 1
        } else {
            insertionIndex = fromIndex < toIndex ? toIndex - 1 : toIndex
        }
        insertionIndex = max(0, min(blocks.count, insertionIndex))
        withAnimation(.easeInOut(duration: 0.25)) {
            blocks.insert(block, at: insertionIndex)
        }
        refreshUndoRedoFlags()
    }
    
    func deleteBlock(id: UUID) {
        guard let idx = blocks.firstIndex(where: { $0.id == id }) else { return }
        recordState()
        withAnimation(.easeInOut(duration: 0.25)) {
            blocks.remove(at: idx)
        }
        refreshUndoRedoFlags()
    }
    
    private func refreshUndoRedoFlags() {
        canUndo = !history.isEmpty
        canRedo = !redoStack.isEmpty
    }
} 