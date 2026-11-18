import XCTest

final class ScatterPerformanceTests: XCTestCase {
    @MainActor
    func testHoverAcross2048ScatterPoints() throws {
        continueAfterFailure = false
        let file = try XCTUnwrap(
            Bundle(for: Self.self).url(forResource: "Interaction performance", withExtension: "csv"))
        let app = XCUIApplication()
        app.launchArguments = [
            "-ApplePersistenceIgnoreState", "YES", "-AppleLanguages", "(en)", "-AppleLocale", "en_US",
            "-ShowColumnInspector", "YES", file.path,
        ]
        app.launch()
        addTeardownBlock { @MainActor in app.terminate() }
        let elements = app.descendants(matching: .any)
        let chartTab = elements["Chart"].firstMatch
        XCTAssertTrue(chartTab.waitForExistence(timeout: 10))
        chartTab.click()
        XCTAssertTrue(app.staticTexts["2,048 sampled of 8,192 pairs"].waitForExistence(timeout: 15))

        let plot = elements["scatterPlot"].firstMatch
        let sourceRow = elements["scatterHoveredRow"].firstMatch
        func hoveredRow() throws -> Int? {
            let snapshot = try sourceRow.snapshot()
            return descendants(of: snapshot).lazy.compactMap { element in
                Int(element.value as? String ?? "") ?? Int(element.label) ?? Int(element.title)
            }.first
        }

        let chart = try plot.snapshot()
        // Charts exposes horizontal data ranges as accessibility regions. The
        // fixture plots its source row on both axes, so the hits are diagonal.
        let dataBounds = chart.children.reduce(CGRect.null) { $0.union($1.frame) }
        XCTAssertFalse(dataBounds.isEmpty)
        let origin = plot.coordinate(withNormalizedOffset: .zero)
        let sweep = (1...5).map { step in
            let fraction = CGFloat(step) / 6
            let point = CGPoint(
                x: dataBounds.minX + dataBounds.width * fraction,
                y: dataBounds.maxY - dataBounds.height * fraction)
            return origin.withOffset(CGVector(dx: point.x - chart.frame.minX, dy: point.y - chart.frame.minY))
        }
        let options = XCTMeasureOptions()
        options.iterationCount = 3
        options.invocationOptions = [.manuallyStart, .manuallyStop]
        measure(metrics: [XCTCPUMetric(application: app), XCTClockMetric()], options: options) {
            sweep[0].hover()
            let left = try? hoveredRow()
            XCTAssertNotNil(left, "The first hover should reveal a source row.")
            startMeasuring()
            for position in sweep.dropFirst() { position.hover() }
            stopMeasuring()
            let right = try? hoveredRow()
            XCTAssertNotNil(right, "The last hover should reveal a source row.")
            if let left, let right { XCTAssertLessThan(left, right) }
        }
    }

    private func descendants(of snapshot: any XCUIElementSnapshot) -> [any XCUIElementSnapshot] {
        snapshot.children.flatMap { [$0] + descendants(of: $0) }
    }
}
