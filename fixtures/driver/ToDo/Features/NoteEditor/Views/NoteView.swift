import SwiftUI

struct NoteView: View {
    let note: Note
    @StateObject private var viewModel: NoteViewModel
    @State private var dividerY: CGFloat = .infinity
    @Environment(\.dismiss) private var dismiss
    
    init(note: Note) {
        self.note = note
        self._viewModel = StateObject(wrappedValue: NoteViewModel(note: note))
    }
    
    var body: some View {
        ZStack {
            backgroundLayer
            statusBarBackground
            contentLayer
        }
        .navigationBarHidden(true)
    }
    
    private var contentLayer: some View {
        VStack(spacing: 0) {
            stickyNavigationBar
            
            if dividerY <= 60 {
                DashedDivider()
                    .transition(.opacity.combined(with: .move(edge: .top)))
                    .zIndex(1)
            }
            
            scrollLayer
                .safeAreaInset(edge: .bottom, spacing: 0) {
                    if viewModel.editMode {
                        FloatingToolbar(viewModel: viewModel)
                            .transition(.move(edge: .bottom).combined(with: .opacity))
                    }
                }
        }
        .ignoresSafeArea(edges: .horizontal)
    }
    
    private var stickyNavigationBar: some View {
        let headerHeight: CGFloat = 60
        let shouldShowCompact = dividerY <= headerHeight
        
        return ZStack {
            HStack {
                Button(action: { dismiss() }) {
                    HStack(spacing: 4) {
                        Image(systemName: "chevron.left")
                            .font(.system(size: 16, weight: .medium))
                        Text("Back")
                            .font(.system(size: 17, weight: .medium))
                            .foregroundColor(.white.opacity(0.3))
                    }
                    .foregroundColor(.white)
                }
                .accessibilityLabel("Back")
                
                Spacer()
                
                HStack(spacing: 8) {
                    Text("Edit mode")
                        .font(.system(size: 17, weight: .medium))
                        .foregroundColor(.white)
                    
                    Toggle("", isOn: $viewModel.editMode)
                        .toggleStyle(.switch)
                        .tint(.primaryAccent)
                        .scaleEffect(1)
                        .fixedSize()
                }
            }
            
            if shouldShowCompact {
                Text(viewModel.title)
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundColor(.white)
                    .lineLimit(1)
                    .transition(.opacity.combined(with: .move(edge: .bottom)))
            }
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 12)
        .background(
            Rectangle()
                .fill(.white.opacity(0.01))
        )
        .animation(.easeInOut(duration: 0.25), value: shouldShowCompact)
    }
    
    private var heroSection: some View {
        let headerHeight: CGFloat = 60
        let progress = min(max((headerHeight - dividerY) / 140, 0), 1)
        let titleScale = 1 - (0.6 * progress)
        
        return VStack(alignment: .leading, spacing: 8) {
            Text(viewModel.title)
                .font(.system(size: 37, weight: .semibold))
                .foregroundColor(.white)
                .scaleEffect(titleScale, anchor: .leading)
                .frame(maxWidth: .infinity, alignment: .leading)
            
            Text(viewModel.date)
                .font(.system(size: 18))
                .foregroundColor(.white.opacity(0.6))
                .opacity(1 - progress)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 20)
        .padding(.top, 24)
        .padding(.bottom, 32)
        .frame(maxWidth: .infinity)
        .background(.white.opacity(0.01))
        .shadow(color: .black.opacity(0.2), radius: 34, x: 0, y: 18)
        .animation(.easeInOut(duration: 0.25), value: progress)
    }
    
    private var backgroundLayer: some View {
        Color(hex: "191919").ignoresSafeArea()
    }
    
    private var statusBarBackground: some View {
        GeometryReader { geo in
            Color.white.opacity(0.01)
                .frame(height: geo.safeAreaInsets.top)
                .ignoresSafeArea(edges: .top)
        }
    }
    
    private var scrollLayer: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(spacing: 0) {
                    heroSection
                    dividerAndBlocks
                    Spacer(minLength: 100)
                }
            }
            .coordinateSpace(name: "scroll")
            .onPreferenceChange(DividerOffsetPreferenceKey.self) { value in
                dividerY = value
            }
            .onChange(of: viewModel.lastAddedBlockId) { newId in
                if let newId = newId {
                    withAnimation(.easeInOut(duration: 0.3)) {
                        proxy.scrollTo(newId, anchor: .center)
                    }
                }
            }
        }
    }
    
    private var dividerAndBlocks: some View {
        VStack(spacing: 0) {
            DashedDivider()
                .opacity(dividerY <= 60 ? 0 : 1)
                .background(
                    GeometryReader { geo in
                        Color.clear.preference(key: DividerOffsetPreferenceKey.self,
                                               value: geo.frame(in: .global).minY)
                    }
                )
            blocksList
        }
    }
    
    private var blocksList: some View {
        LazyVStack(spacing: 12) {
            ForEach(viewModel.blocks, id: \.id) { block in
                BlockView(
                    block: block,
                    editMode: viewModel.editMode,
                    onTaskToggle: { taskId in
                        viewModel.toggleTaskInList(blockId: block.id, taskId: taskId)
                    },
                    onBlockUpdate: { updatedBlock in
                        viewModel.updateBlock(updatedBlock)
                    },
                    onDelete: { id in
                        withAnimation(.easeInOut(duration: 0.25)) {
                            viewModel.deleteBlock(id: id)
                        }
                    },
                    onMoveBlock: { draggedId, targetId, after in
                        withAnimation(.easeInOut(duration: 0.25)) {
                            viewModel.moveBlock(draggedId, relativeTo: targetId, after: after)
                        }
                    }
                )
            }
        }
        .padding(.horizontal, 20)
        .padding(.top, 20)
    }
}

#Preview {
    NoteView(note: Note(title: "Note 1", dateCreated: Date(), blocks: []))
}

// MARK: - Divider Offset PreferenceKey

private struct DividerOffsetPreferenceKey: PreferenceKey {
    static var defaultValue: CGFloat = .infinity
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
    }
}
