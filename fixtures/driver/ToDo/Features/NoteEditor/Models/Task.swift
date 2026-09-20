import SwiftUI

struct Task: Identifiable, Equatable, Codable {
    let id: UUID
    var text: String
    var isDone: Bool
    
    init(text: String, isDone: Bool = false) {
        self.id = UUID()
        self.text = text
        self.isDone = isDone
    }
    
    init(id: UUID, text: String, isDone: Bool = false) {
        self.id = id
        self.text = text
        self.isDone = isDone
    }
} 