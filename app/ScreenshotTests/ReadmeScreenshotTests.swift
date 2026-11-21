import XCTest

final class ReadmeScreenshotTests: XCTestCase {
    let columns = [
        "VendorID", "tpep_pickup_datetime", "tpep_dropoff_datetime", "passenger_count", "trip_distance",
        "RatecodeID", "store_and_fwd_flag", "PULocationID", "DOLocationID", "payment_type", "fare_amount",
        "extra", "mta_tax", "tip_amount", "tolls_amount", "improvement_surcharge", "total_amount",
        "congestion_surcharge", "Airport_fee",
    ]

    @MainActor
    func testExplore() throws {
        continueAfterFailure = false
        let env = ProcessInfo.processInfo.environment
        let data = try XCTUnwrap(env["GEWU_SCREENSHOT_DATA"])
        let app = XCUIApplication()
        app.launchArguments = [
            "-ApplePersistenceIgnoreState", "YES", "-AppleLanguages", "(en)", "-AppleLocale", "en_US",
            "-ShowColumnInspector", "YES", data,
        ]
        app.launch()
        addTeardownBlock { @MainActor in app.terminate() }
        let table = app.tables.firstMatch
        XCTAssertTrue(table.waitForExistence(timeout: 30))
        var visible = columns
        func cell(_ name: String) -> XCUIElement {
            let index = try! XCTUnwrap(visible.firstIndex(of: name))
            return table.tableRows.element(boundBy: 0).cells.element(boundBy: index + 1)
        }
        func header(_ name: String) -> XCUICoordinate {
            let c = cell(name)
            return c.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: -0.6))
        }
        for name in [
            "VendorID", "tpep_dropoff_datetime", "RatecodeID", "store_and_fwd_flag", "PULocationID",
            "DOLocationID", "extra", "mta_tax", "tolls_amount", "improvement_surcharge", "congestion_surcharge",
            "Airport_fee",
        ] {
            header(name).rightClick()
            choose(app, "Hide Column")
            visible.removeAll { $0 == name }
        }
        sleep(1)
        cell("payment_type").rightClick()
        choose(app, "Filter Column")
        sleep(3)
        attach(app, "filter")
    }

    /// Pick an item in the open context menu by typing its title, which avoids
    /// querying the whole accessibility tree.
    @MainActor
    private func choose(_ app: XCUIApplication, _ title: String) {
        usleep(300_000)
        app.typeText(title)
        app.typeKey(.return, modifierFlags: [])
        usleep(300_000)
    }

    @MainActor
    private func attach(_ app: XCUIApplication, _ name: String) {
        let tree = XCTAttachment(string: app.debugDescription)
        tree.name = name + "-tree"
        tree.lifetime = .keepAlways
        add(tree)
        let shot = XCTAttachment(screenshot: app.windows.firstMatch.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }
}
