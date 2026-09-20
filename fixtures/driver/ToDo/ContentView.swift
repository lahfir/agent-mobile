import SwiftUI
import UIKit

private struct SearchBarOffsetKey: PreferenceKey {
    static var defaultValue: CGFloat = .infinity
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
    }
}

struct ContentView: View {
    @StateObject private var viewModel = FolderListViewModel()
    @State private var searchBarMinY: CGFloat = .infinity
    @State private var showCreateFolderModal = false
    
    private let columns: [GridItem] = [
        GridItem(.flexible(), spacing: 16),
        GridItem(.flexible(), spacing: 16)
    ]
    
    private let headerBackground = Color.white.opacity(0.08)
    private var headerHeight: CGFloat { UIScreen.main.bounds.height * 0.3 }
    private var safeAreaTop: CGFloat {
        (UIApplication.shared.connectedScenes.first as? UIWindowScene)?.windows.first?.safeAreaInsets.top ?? 0
    }
    
    private var showPinnedSearchBar: Bool {
        searchBarMinY < safeAreaTop + 8
    }
    
    var body: some View {
        NavigationStack {
            ZStack(alignment: .top) {
                Color(hex: "191919").ignoresSafeArea()
                statusBarBackground
                
                ScrollView(showsIndicators: false) {
                    VStack(spacing: 24) {
                        headerSection
                        VStack(spacing: 24) {
                            foldersHeader
                            LazyVGrid(columns: columns, spacing: 16) {
                                ForEach(viewModel.folders) { folder in
                                    NavigationLink(destination: FolderDetailView(folder: folder)) {
                                        FolderCardView(folder: folder)
                                    }
                                    .buttonStyle(.plain)
                                }
                            }
                            Spacer(minLength: 100)
                        }
                        .padding(.horizontal, 20)
                    }
                    .background(
                        GeometryReader { geo in
                            Color.clear.preference(key: SearchBarOffsetKey.self, value: geo.frame(in: .named("scroll")).minY)
                        }
                    )
                }
                .coordinateSpace(name: "scroll")
                .onPreferenceChange(SearchBarOffsetKey.self) { value in
                    searchBarMinY = value
                }
                
                if showPinnedSearchBar {
                    VStack(spacing: 0) {
                        headerBackground
                            .frame(height: safeAreaTop)
                            .ignoresSafeArea(.all)
                        SearchBar(text: $viewModel.searchText)
                            .padding(.horizontal, 20)
                            .padding(.vertical, 8)
                            .background(headerBackground)
                    }
                    .frame(maxWidth: .infinity)
                    .transition(.move(edge: .top).combined(with: .opacity))
                    .animation(.easeInOut(duration: 0.25), value: showPinnedSearchBar)
                }
            }
            .navigationBarHidden(true)
        }
        .overlay(
            Group {
                if showCreateFolderModal {
                    CreateFolderModal(isPresented: $showCreateFolderModal) { name, color in
                        viewModel.addFolder(name: name, color: color)
                    }
                }
            }
        )
    }
    
    private var statusBarBackground: some View {
        GeometryReader { geo in
            headerBackground
                .frame(height: geo.safeAreaInsets.top)
                .ignoresSafeArea(edges: .top)
        }
    }
    
    private var headerSection: some View {
        ZStack(alignment: .bottom) {
            headerBackground
                .ignoresSafeArea(.all)
                .cornerRadius(32, corners: [.bottomLeft, .bottomRight])
                .shadow(color: .black.opacity(0.6), radius: 30, x: 0, y: 15)
            
            VStack(alignment: .leading, spacing: 16) {
                profileAndGreeting
                SearchBar(text: $viewModel.searchText)
                    .padding(.horizontal, 20)
                    .background(
                        GeometryReader { geo in
                            Color.clear.preference(key: SearchBarOffsetKey.self, value: geo.frame(in: .named("scroll")).minY)
                        }
                    )
            }
            .padding(.bottom, 24)
            .padding(.top, safeAreaTop + 20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(maxWidth: .infinity)
        .frame(height: headerHeight + safeAreaTop)
    }
    
    private var profileAndGreeting: some View {
        VStack(alignment: .leading, spacing: 16) {
            Image(systemName: "person.crop.square")
                .resizable()
                .scaledToFill()
                .frame(width: 48, height: 48)
                .overlay(
                    RoundedRectangle(cornerRadius: 12)
                        .stroke(Color.white, lineWidth: 1)
                )
                .clipShape(RoundedRectangle(cornerRadius: 12))
            
            HStack {
                VStack(alignment: .leading, spacing: 4) {
                    Label {
                        Text("MORNING SAM")
                            .font(.system(size: 12, weight: .semibold))
                            .textCase(.uppercase)
                            .foregroundColor(.white.opacity(0.6))
                    } icon: {
                        Image(systemName: "sun.max.fill")
                            .font(.system(size: 12))
                            .foregroundColor(.yellow)
                    }
                    .labelStyle(.titleAndIcon)
                    
                    (Text("Note ")
                        .foregroundColor(.pink)
                        .fontWeight(.bold) + Text("that big idea brother"))
                    .font(.system(size: 35, weight: .bold))
                    .foregroundColor(.white)
                    .fixedSize(horizontal: false, vertical: true)
                }
                Spacer()
            }
        }
        .padding(.horizontal, 20)
    }

    private var foldersHeader: some View {
        HStack {
            Text("FOLDERS")
                .font(.system(size: 14, weight: .semibold))
                .foregroundColor(.white.opacity(0.6))
            Spacer()
            Button(action: {
                showCreateFolderModal = true
            }) {
                Image(systemName: "plus")
                    .font(.system(size: 20, weight: .bold))
                    .foregroundColor(.white)
            }
            .accessibilityLabel("Add Folder")
        }
    }
}

#Preview {
    ContentView()
} 