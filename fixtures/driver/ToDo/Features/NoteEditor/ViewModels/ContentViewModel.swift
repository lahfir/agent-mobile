import SwiftUI

@MainActor
class ContentViewModel: ObservableObject {
    @Published var notes: [Note] = []
    
    init() {
        setupSampleNotes()
    }
    
    private func setupSampleNotes() {
        let calendar = Calendar.current
        
        notes = [
            Note(
                title: "Singing Tips",
                dateCreated: calendar.date(byAdding: .day, value: -2, to: Date()) ?? Date(),
                blocks: [
                    Block(kind: .paragraph("Hello Guys,")),
                    Block(kind: .paragraph("This App is in Progress, but you can get a Early Access")),
                    Block(kind: .taskList([
                        Task(text: "Buy X and rename it to Twitter"),
                        Task(text: "Not to download another Todo app"),
                        Task(text: "At least sit in front of books for 2 minutes, and read few words")
                    ])),
                    Block(kind: .image(UIImage(named: "instagram")!))
                ]
            ),
            Note(
                title: "Morning Routine",
                dateCreated: calendar.date(byAdding: .day, value: -5, to: Date()) ?? Date(),
                blocks: [
                    Block(kind: .heading(level: .h1, text: "My Perfect Morning")),
                    Block(kind: .paragraph("Starting the day right is crucial for productivity and happiness.")),
                    Block(kind: .taskList([
                        Task(text: "Wake up at 6:00 AM", isDone: true),
                        Task(text: "Drink a glass of water"),
                        Task(text: "10 minutes meditation"),
                        Task(text: "Light stretching or yoga")
                    ]))
                ]
            ),
            Note(
                title: "Project Ideas",
                dateCreated: calendar.date(byAdding: .day, value: -10, to: Date()) ?? Date(),
                blocks: [
                    Block(kind: .paragraph("Collection of app ideas I want to build someday.")),
                    Block(kind: .taskList([
                        Task(text: "Weather app with personality"),
                        Task(text: "Habit tracker with gamification"),
                        Task(text: "Recipe organizer with AI suggestions"),
                        Task(text: "Minimalist journal app")
                    ]))
                ]
            ),
            Note(
                title: "Travel Planning",
                dateCreated: calendar.date(byAdding: .day, value: -15, to: Date()) ?? Date(),
                blocks: [
                    Block(kind: .heading(level: .h2, text: "Summer Vacation 2025")),
                    Block(kind: .paragraph("Planning the perfect getaway to recharge and explore.")),
                    Block(kind: .taskList([
                        Task(text: "Research destinations", isDone: true),
                        Task(text: "Book flights"),
                        Task(text: "Find accommodations"),
                        Task(text: "Create itinerary")
                    ]))
                ]
            ),
            Note(
                title: "Learning Goals",
                dateCreated: calendar.date(byAdding: .day, value: -20, to: Date()) ?? Date(),
                blocks: [
                    Block(kind: .paragraph("Skills I want to develop this year.")),
                    Block(kind: .taskList([
                        Task(text: "Master SwiftUI animations", isDone: true),
                        Task(text: "Learn Core Data"),
                        Task(text: "Understand CloudKit"),
                        Task(text: "Practice design patterns")
                    ]))
                ]
            )
        ]
    }
} 
