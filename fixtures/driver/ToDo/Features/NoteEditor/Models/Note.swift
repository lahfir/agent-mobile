import SwiftUI

struct Note: Identifiable, Equatable {
    let id: UUID
    let title: String
    let dateCreated: Date
    let blocks: [Block]
    
    init(title: String, dateCreated: Date = Date(), blocks: [Block] = []) {
        self.id = UUID()
        self.title = title
        self.dateCreated = dateCreated
        self.blocks = blocks
    }
    
    var formattedDate: String {
        let formatter = DateFormatter()
        formatter.dateFormat = "MMMM d, yyyy"
        return formatter.string(from: dateCreated)
    }
} 