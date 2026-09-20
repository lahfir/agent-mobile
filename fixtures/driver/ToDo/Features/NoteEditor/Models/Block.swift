import SwiftUI
import UIKit

struct Block: Identifiable, Equatable {
    let id: UUID
    var kind: Kind
    
    init(kind: Kind) {
        self.id = UUID()
        self.kind = kind
    }
    
    enum Kind: Equatable {
        case paragraph(String)
        case heading(level: HeadingLevel, text: String)
        case image(UIImage)
        case taskList([Task])
        case taskItem(Task)
        
        static func == (lhs: Kind, rhs: Kind) -> Bool {
            switch (lhs, rhs) {
            case (.paragraph(let lText), .paragraph(let rText)):
                return lText == rText
            case (.heading(let lLevel, let lText), .heading(let rLevel, let rText)):
                return lLevel == rLevel && lText == rText
            case (.image(let lImage), .image(let rImage)):
                return lImage.pngData() == rImage.pngData()
            case (.taskList(let lTasks), .taskList(let rTasks)):
                return lTasks == rTasks
            case (.taskItem(let lTask), .taskItem(let rTask)):
                return lTask == rTask
            default:
                return false
            }
        }
    }
} 