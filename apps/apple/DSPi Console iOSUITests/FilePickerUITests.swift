import XCTest

/// WP SW17: the Files entries open the system document picker in the
/// Simulator, for an import and for an export, and cancelling it does
/// nothing.  No file is picked: the picker only shows what is in the
/// Simulator's own Files, inside its sandbox.
final class FilePickerUITests: XCTestCase {

    override func setUp() {
        continueAfterFailure = false
    }

    private func launch() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-LocalTransport", "demo", "-DemoMode", "YES"]
        app.launch()
        return app
    }

    /// Tools, then on an iPhone its Files submenu, then `item`.
    private func open(_ item: String, in app: XCUIApplication) {
        let tools = app.buttons["Tools"]
        XCTAssertTrue(tools.waitForExistence(timeout: 30), "no Tools menu")
        tools.tap()
        if UIDevice.current.userInterfaceIdiom == .phone {
            let files = app.buttons["Files"]
            XCTAssertTrue(files.waitForExistence(timeout: 5), "no Files submenu on the phone")
            files.tap()
        }
        let entry = app.buttons[item]
        XCTAssertTrue(entry.waitForExistence(timeout: 5), "no \(item) entry")
        entry.tap()
    }

    /// The picker is up (its browser's bar), an export offers Save, and the
    /// picker goes away on Cancel with nothing shown after it.
    private func assertPickerOpensAndCancels(_ item: String, exporting: Bool, in app: XCUIApplication) {
        open(item, in: app)
        let browser = app.navigationBars["FullDocumentManagerViewControllerNavigationBar"]
        XCTAssertTrue(browser.waitForExistence(timeout: 20), "\(item): the document picker did not open")
        XCTAssertFalse(app.alerts.firstMatch.exists, "\(item): an alert instead of the picker")
        if exporting {
            XCTAssertTrue(app.buttons["Save"].exists, "\(item): the export picker offers Save")
        }
        let cancel = app.buttons["Cancel"]
        if cancel.exists {
            cancel.tap()
        } else {
            // The export picker on an iPhone has no room for Cancel on its
            // bar: it is a sheet, dismissed by pulling it down.
            let top = browser.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.1))
            top.press(forDuration: 0.1, thenDragTo: top.withOffset(CGVector(dx: 0, dy: 700)))
        }
        expectation(for: NSPredicate(format: "exists == false"), evaluatedWith: browser)
        waitForExpectations(timeout: 10)
        XCTAssertFalse(app.alerts.firstMatch.waitForExistence(timeout: 2), "\(item): cancelling showed an alert")
    }

    func testFilesEntriesOpenTheDocumentPicker() {
        let app = launch()
        assertPickerOpensAndCancels("Import Filters...", exporting: false, in: app)
        assertPickerOpensAndCancels("Export Filters...", exporting: true, in: app)
        assertPickerOpensAndCancels("Import Device Configuration...", exporting: false, in: app)
        assertPickerOpensAndCancels("Export Device Configuration...", exporting: true, in: app)
    }
}
