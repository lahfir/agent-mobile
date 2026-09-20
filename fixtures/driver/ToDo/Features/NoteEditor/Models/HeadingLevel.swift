import SwiftUI

enum HeadingLevel: Int, CaseIterable, Identifiable {
    case h1 = 1
    case h2 = 2
    case h3 = 3
    
    var id: Int { rawValue }
    
    var displayName: String {
        "Heading \(rawValue)"
    }
    
    var fontSize: CGFloat {
        switch self {
        case .h1: return 28
        case .h2: return 24
        case .h3: return 20
        }
    }
    
    var fontWeight: Font.Weight {
        switch self {
        case .h1: return .bold
        case .h2: return .semibold
        case .h3: return .medium
        }
    }
} 