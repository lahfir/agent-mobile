import SwiftUI

enum ImageSource: String, CaseIterable, Identifiable {
    case photoLibrary = "Photo Library"
    case files = "Files"
    case camera = "Take Photo"
    
    var id: String { rawValue }
    
    var systemImage: String {
        switch self {
        case .photoLibrary: return "photo.on.rectangle"
        case .files: return "folder"
        case .camera: return "camera"
        }
    }
} 