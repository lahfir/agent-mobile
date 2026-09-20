import SwiftUI
import PhotosUI
import Foundation
import UIKit

struct FloatingToolbar: View {
    @ObservedObject var viewModel: NoteViewModel
    @State private var headingButtonFrame: CGRect = .zero
    @State private var showHeadingPicker = false
    @State private var selectedPhoto: PhotosPickerItem? = nil
    @State private var headingPopupSize: CGSize = .zero
    @State private var toolbarGeometry: CGRect = .zero
    
    var body: some View {
        VStack(spacing: 0) {
            DashedDivider()
                        .transition(.opacity.combined(with: .move(edge: .bottom)))
                        .zIndex(1)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 16) {
                    // Undo & Redo
                    circleButton(assetName: "ArrowBendUpLeft", disabled: !viewModel.canUndo) { viewModel.undo() }
                    circleButton(assetName: "ArrowBendUpRight", disabled: !viewModel.canRedo) { viewModel.redo() }
                    
                    divider
                    
                    // Heading button that toggles the heading picker
                    HeadingToolbarButton(showPicker: showHeadingPicker) {
                        withAnimation(.easeInOut(duration: 0.2)) {
                            showHeadingPicker.toggle()
                        }
                    }
                    
                    // Paragraph
                    toolbarButton(assetName: "Paragraph") {
                        viewModel.addBlock(Block(kind: .paragraph("New paragraph")))
                        Haptics.selection()
                    }
                    // Image picker
                    PhotosPicker(selection: $selectedPhoto, matching: .images) {
                        ZStack {
                            Capsule()
                                .fill(Color.white.opacity(0.15))
                                .frame(minWidth: 80)
                                .frame(height: 52)

                            Image("ImageSquare")
                                .resizable()
                                .scaledToFit()
                                .frame(width: 24, height: 24)
                        }
                        .contentShape(Rectangle())
                    }
                    .simultaneousGesture(TapGesture().onEnded {
                        if showHeadingPicker { showHeadingPicker = false }
                        Haptics.selection()
                    })
                    // Table
                    toolbarButton(systemName: "tablecells") {
                        // Simple placeholder table block: three cells pipe-separated
                        viewModel.addBlock(Block(kind: .paragraph("|   |   |   |")))
                        Haptics.selection()
                    }
                    // Checklist
                    toolbarButton(assetName: "ListChecks") {
                        viewModel.addBlock(
                            Block(kind: .taskList([Task(text: "New task")]))
                        )
                        Haptics.selection()
                    }
                }
                .padding(.horizontal, 24)
            }
            .coordinateSpace(name: "toolbarSpace")
            .background(
                GeometryReader { geo in
                    Color.clear
                        .onAppear {
                            toolbarGeometry = geo.frame(in: .global)
                        }
                        .onChange(of: geo.frame(in: .global)) { newValue in
                            toolbarGeometry = newValue
                        }
                }
            )
            .onPreferenceChange(HeadingToolbarButton.FrameKey.self) { value in
                self.headingButtonFrame = value
            }
            .padding(.vertical, 12)
            .background(
                GeometryReader { geo in
                    Rectangle()
                        .fill(.ultraThinMaterial)
                        .blur(radius: 10)
                        .frame(height: geo.size.height + geo.safeAreaInsets.bottom + 100)
                        .frame(maxHeight: .infinity, alignment: .bottom)
                        .allowsHitTesting(false)
                        .transition(.opacity)
                }
            )
            .transition(.move(edge: .bottom).combined(with: .opacity))
            // Popup positioned via anchor preference above the heading button
            .overlayPreferenceValue(HeadingToolbarButton.AnchorKey.self) { anchor in
                if showHeadingPicker, let anchor {
                    GeometryReader { proxy in
                        let frame = proxy[anchor]
                        HeadingPickerPopup(
                            onSelect: { level in
                                viewModel.addBlock(Block(kind: .heading(level: level, text: "New Heading")))
                                withAnimation(.spring(response: 0.35, dampingFraction: 0.75)) {
                                    showHeadingPicker = false
                                }
                            },
                            onDismiss: {
                                withAnimation(.spring(response: 0.35, dampingFraction: 0.75)) {
                                    showHeadingPicker = false
                                }
                            }
                        )
                        .background(
                            GeometryReader { popupGeo in
                                Color.clear
                                    .onAppear { headingPopupSize = popupGeo.size }
                                    .onChange(of: popupGeo.size) { headingPopupSize = $0 }
                            }
                        )
                        .position(
                            x: frame.minX + headingPopupSize.width / 2,
                            y: frame.minY - headingPopupSize.height / 2 - 8
                        )
                        .transition(
                            .asymmetric(
                                insertion: .scale(scale: 0.8, anchor: .bottom).combined(with: .opacity),
                                removal: .scale(scale: 0.8, anchor: .bottom).combined(with: .opacity)
                            )
                        )
                    }
                    .zIndex(100)
                }
            }
        }
        .frame(maxWidth: .infinity, minHeight: 72, maxHeight: 72)
        .padding(.horizontal, 0)
        .ignoresSafeArea(edges: .horizontal)
        .animation(.spring(response: 0.35, dampingFraction: 0.75), value: showHeadingPicker)
        .onChange(of: selectedPhoto) { newValue in
            guard let item = newValue else { return }
            
            _Concurrency.Task {
                await self.processPhotoSelection(item: item)
            }
        }
        .coordinateSpace(name: "toolbarRoot")
    }
    
    // Robustly load picked image (supports PNG, JPEG, HEIC)
    private func processPhotoSelection(item: PhotosPickerItem) async {
        do {
            // Fallback: SwiftUI Image → UIImage renderer (PNG)
            if let swiftUIImage = try? await item.loadTransferable(type: Image.self) {
                let renderer = ImageRenderer(content: swiftUIImage)
                if let rendered = renderer.uiImage {
                    await insertImage(rendered)
                    return
                }
            }

            // Final fallback: raw Data → UIImage
            if let data = try? await item.loadTransferable(type: Data.self),
               let dataImage = UIImage(data: data) {
                await insertImage(dataImage)
                return
            }
        } catch {
            print("Failed to load image: \(error)")
        }
        await MainActor.run { self.selectedPhoto = nil }
    }

    @MainActor
    private func insertImage(_ uiImage: UIImage) {
        withAnimation(.easeInOut(duration: 0.25)) {
            self.viewModel.addBlock(Block(kind: .image(uiImage)))
        }
        self.selectedPhoto = nil
    }
    
    private func toolbarButton(assetName: String, isText: Bool = false, action: @escaping () -> Void) -> some View {
        Button(action: {
            Haptics.selection()
            if showHeadingPicker { showHeadingPicker = false }
            action()
        }) {
            ZStack {
                Capsule()
                    .fill(Color.white.opacity(0.15))
                    .frame(minWidth: 80)
                    .frame(height: 52)
                
                if isText {
                    Text(assetName)
                        .font(.system(size: 16, weight: .bold))
                        .foregroundColor(.white)
                } else {
                    Image(assetName)
                        .resizable()
                        .scaledToFit()
                        .frame(width: 24, height: 24)
                }
            }
        }
        .buttonStyle(ToolbarButtonStyle())
        .accessibilityLabel(assetName)
        .accessibilityHint("Add \(assetName) block")
    }
    
    private func circleButton(assetName: String, disabled: Bool = false, action: @escaping () -> Void) -> some View {
        Button(action: {
            Haptics.selection()
            if showHeadingPicker { showHeadingPicker = false }
            action()
        }) {
            ZStack {
                Circle()
                    .fill(Color.clear)
                    .frame(width: 52, height: 52)
                    .overlay(
                        Circle()
                            .stroke(Color.white.opacity(0.15), lineWidth: 1)
                    )
                
                Image(assetName)
                    .resizable()
                    .scaledToFit()
                    .frame(width: 20, height: 20)
                    .foregroundColor(.white)
            }
        }
        .buttonStyle(ToolbarButtonStyle())
        .opacity(disabled ? 0.4 : 1)
        .disabled(disabled)
        .accessibilityLabel(assetName)
        .accessibilityHint("Undo")
    }
    
    private var divider: some View {
        Rectangle()
            .fill(Color.white.opacity(0.15))
            .frame(width: 1, height: 52)
    }
    
    private func toolbarButton(systemName: String, action: @escaping () -> Void) -> some View {
        Button(action: {
            Haptics.selection()
            action()
        }) {
            ZStack {
                Capsule()
                    .fill(Color.white.opacity(0.15))
                    .frame(minWidth: 80)
                    .frame(height: 52)
                Image(systemName: systemName)
                    .font(.system(size: 18, weight: .medium))
                    .foregroundColor(.white)
            }
        }
        .buttonStyle(ToolbarButtonStyle())
        .accessibilityLabel(systemName)
    }
}

struct ToolbarButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.9 : 1.0)
            .animation(.easeInOut(duration: 0.1), value: configuration.isPressed)
    }
}

// MARK: - Heading Toolbar Button

private struct HeadingToolbarButton: View {
    let showPicker: Bool
    let action: () -> Void

    // Preference to capture frame
    struct FrameKey: PreferenceKey {
        static var defaultValue: CGRect = .zero
        static func reduce(value: inout CGRect, nextValue: () -> CGRect) {
            value = nextValue()
        }
    }

    // Anchor preference to get button bounds
    struct AnchorKey: PreferenceKey {
        static var defaultValue: Anchor<CGRect>? = nil
        static func reduce(value: inout Anchor<CGRect>?, nextValue: () -> Anchor<CGRect>?) {
            value = nextValue() ?? value
        }
    }

    var body: some View {
        Button(action: {
            Haptics.selection()
            action()
        }) {
            HStack(spacing: 6) {
                Image("TextH")
                    .resizable()
                    .scaledToFit()
                    .frame(width: 24, height: 24)

                Image(systemName: "chevron.down")
                    .resizable()
                    .scaledToFit()
                    .frame(width: 10, height: 10)
                    .rotationEffect(.degrees(showPicker ? 180 : 0))
                    .animation(.easeInOut(duration: 0.2), value: showPicker)
                    .foregroundColor(.white.opacity(0.6))
            }
            .frame(minWidth: 80)
            .frame(height: 52)
            .background(
                Capsule()
                    .fill(Color.white.opacity(0.15))
            )
        }
        .buttonStyle(ToolbarButtonStyle())
        .accessibilityLabel("Heading")
        .accessibilityHint("Add heading block")
        .anchorPreference(key: AnchorKey.self, value: .bounds) { $0 }
    }
}

struct HeadingPickerPopup: View {
    let onSelect: (HeadingLevel) -> Void
    let onDismiss: () -> Void
    
    private func iconSize(for level: HeadingLevel) -> CGFloat {
        switch level {
        case .h1: return 24
        case .h2: return 20
        case .h3: return 16
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(HeadingLevel.allCases.enumerated()), id: \.offset) { index, level in
                Button(action: {
                    Haptics.selection()
                    onSelect(level)
                }) {
                    HStack(spacing: 12) {
                        Image("TextH")
                            .resizable()
                            .scaledToFit()
                            .frame(width: iconSize(for: level), height: iconSize(for: level))

                        Text(level.displayName)
                            .font(.system(size: 16))
                            .foregroundColor(.white)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 12)
                }
                .accessibilityLabel(level.displayName)

                if index < HeadingLevel.allCases.count - 1 {
                    DashedDivider()
                        .padding(.horizontal, 16)
                }
            }
        }
        .background(.ultraThinMaterial)
        .cornerRadius(16)
        .shadow(color: .black.opacity(0.3), radius: 20, y: 10)
        .frame(width: 180)
        .zIndex(100)
    }
}

struct ImageSourcePopup: View {
    let onSelect: (ImageSource) -> Void
    let onDismiss: () -> Void
    
    var body: some View {
        ZStack {
            Color.black.opacity(0.3)
                .ignoresSafeArea()
                .onTapGesture { onDismiss() }
            
            VStack(alignment: .leading, spacing: 8) {
                ForEach(ImageSource.allCases) { source in
                    Button(action: {
                        Haptics.selection()
                        onSelect(source)
                    }) {
                        HStack(spacing: 12) {
                            Image(systemName: source.systemImage)
                                .font(.system(size: 14))
                                .foregroundColor(.white.opacity(0.6))
                            
                            Text(source.rawValue)
                                .font(.system(size: 16))
                                .foregroundColor(.white)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        .padding(.horizontal, 16)
                        .padding(.vertical, 12)
                    }
                    .accessibilityLabel(source.rawValue)
                }
            }
            .background(.ultraThinMaterial)
            .cornerRadius(16)
            .shadow(color: .black.opacity(0.3), radius: 20, y: 10)
            .frame(width: 180)
        }
    }
} 