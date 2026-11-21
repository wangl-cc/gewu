import Foundation
import GewuKit
import Testing

@Suite struct ErrorPresentationTests {
    @Test func engineFailuresAreWordedFromTheirKind() {
        let invalid = ErrorPresentation(
            GewuKitError(
                .filter(.invalidValue(value: "not a number", expectedType: "Int64", reason: .invalidSyntax)),
                diagnostic: "engine text"))
        #expect(invalid.message == "“not a number” is not a valid Int64 value.")
        #expect(invalid.details == nil && invalid.text == invalid.message, "The kind says it all")
    }

    @Test func diagnosticsAreDetailsOnlyWhenTheKindCannotSayWhatHappened() {
        let missing = ErrorPresentation(GewuKitError(.io(.canonicalize), diagnostic: "No such file: a.csv"))
        #expect(missing.message == "The file could not be opened.")
        #expect(missing.details == "No such file: a.csv")
        #expect(missing.text == "The file could not be opened.\n\nNo such file: a.csv")
    }

    @Test func otherErrorsKeepTheirOwnDescription() {
        let error = CocoaError(.fileNoSuchFile)
        #expect(ErrorPresentation(error).message == error.localizedDescription)
    }
}
