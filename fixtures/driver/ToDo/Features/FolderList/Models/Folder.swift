import SwiftUI

struct Folder: Identifiable {
    let id: UUID = UUID()
    var name: String
    var notes: [Note]
    var color: Color = .gray
    
    var count: Int { notes.count }
} 