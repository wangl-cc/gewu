// Run the live app with its fixture, restoring the macOS pasteboard after testing Cmd+C.
import AppKit
import Foundation

guard CommandLine.arguments.count == 2 else {
    fputs("usage: swift tests/run-workflow.swift <built GPUI executable>\n", stderr)
    exit(2)
}

let board = NSPasteboard.general
let originalCount = board.changeCount
// Materialize each representation in memory while the original owner still exists.
let original: [NSPasteboardItem] = (board.pasteboardItems ?? []).map { item in
    let saved = NSPasteboardItem()
    for type in item.types {
        guard let data = item.data(forType: type), saved.setData(data, forType: type) else {
            fputs("pasteboard contains a type that cannot be saved; test skipped\n", stderr)
            exit(2)
        }
    }
    return saved
}

let test = Process()
test.executableURL = URL(fileURLWithPath: CommandLine.arguments[1])
let fixture = URL(fileURLWithPath: #filePath).deletingLastPathComponent().appendingPathComponent("workflow.csv")
test.arguments = [fixture.path, "--workflow-test"]
try test.run()
let deadline = Date(timeIntervalSinceNow: 45)
while test.isRunning, Date() < deadline { Thread.sleep(forTimeInterval: 0.05) }
if test.isRunning {
    fputs("workflow-test exceeded 45 seconds\n", stderr)
    test.terminate()
}
test.waitUntilExit()

if board.changeCount != originalCount {
    let copied = board.string(forType: .string)?.trimmingCharacters(in: .newlines)
    // Leave a later copy by the user alone.
    if copied == "keep\t5\tmarker-low" {
        board.clearContents()
        if !original.isEmpty && !board.writeObjects(original) {
            fputs("could not restore the pasteboard\n", stderr)
            exit(2)
        }
    }
}
exit(test.terminationReason == .exit ? test.terminationStatus : 1)
