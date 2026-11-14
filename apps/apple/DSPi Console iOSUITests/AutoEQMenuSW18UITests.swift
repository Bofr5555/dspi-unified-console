import XCTest

/// WP SW18: Tools > AutoEQ on iOS, the Mac's AutoEQ menu.  Browse Profiles
/// opens the browser, a heart there makes a favourite, Favorite Profiles
/// lists and applies it, Clear Favorites empties the list.  Update Database
/// offers the core's update (CG6): Rebuild from GitHub asks first with the
/// core's warning and is cancelled here (no network in a test), Import File
/// opens the document picker.
final class AutoEQMenuSW18UITests: XCTestCase {

    /// The heart on the HD 650 row (oratory1990), by its identifier.
    private let hd650 = "autoeq.favorite.oratory1990/Sennheiser HD 650"

    override func setUp() {
        continueAfterFailure = false
        XCUIDevice.shared.orientation = .portrait
    }

    private func launch() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-LocalTransport", "demo", "-DemoMode", "YES"]
        app.launch()
        return app
    }

    /// Tools, then its AutoEQ submenu.
    private func openAutoEQ(_ app: XCUIApplication) {
        let tools = app.buttons["Tools"]
        XCTAssertTrue(tools.waitForExistence(timeout: 30), "no Tools menu")
        tools.tap()
        let autoEQ = app.buttons["AutoEQ"].firstMatch
        XCTAssertTrue(autoEQ.waitForExistence(timeout: 5), "no AutoEQ submenu in Tools")
        autoEQ.tap()
    }

    private func tapItem(_ name: String, in app: XCUIApplication) {
        let item = app.buttons[name].firstMatch
        XCTAssertTrue(item.waitForExistence(timeout: 5), "no \(name) item")
        item.tap()
    }

    /// Tools > AutoEQ > Favorite Profiles, open.
    private func openFavorites(_ app: XCUIApplication) {
        openAutoEQ(app)
        tapItem("Favorite Profiles", in: app)
    }

    func testSW18AutoEQMenuMakesAppliesAndClearsAFavorite() {
        let app = launch()

        // The menu holds the Mac's items.
        openAutoEQ(app)
        for item in ["Browse Profiles...", "Favorite Profiles", "Update Database..."] {
            XCTAssertTrue(app.buttons[item].firstMatch.waitForExistence(timeout: 5), "no \(item) in the AutoEQ menu")
        }

        // Browse, find the HD 650, make it a favourite (unless an earlier run did).
        tapItem("Browse Profiles...", in: app)
        let search = app.textFields["Search headphones..."]
        XCTAssertTrue(search.waitForExistence(timeout: 10), "the browser did not open")
        search.tap()
        search.typeText("HD 650")
        let heart = app.buttons[hd650]
        XCTAssertTrue(heart.waitForExistence(timeout: 10), "no favourite heart on the HD 650's row")
        if heart.label.hasPrefix("Add") { heart.tap() }
        XCTAssertTrue(app.buttons[hd650].label.hasPrefix("Remove"), "the heart did not take: \(app.buttons[hd650].label)")
        let done = app.navigationBars.buttons["Done"].firstMatch
        XCTAssertTrue(done.exists, "no Done on the browser's sheet")
        done.tap()
        expectation(for: NSPredicate(format: "exists == false"), evaluatedWith: search)
        waitForExpectations(timeout: 10)

        // Listed under Favorite Profiles, and applying it closes the menu.
        openFavorites(app)
        let favourite = app.buttons["Sennheiser HD 650"].firstMatch
        XCTAssertTrue(favourite.waitForExistence(timeout: 5), "the favourite is not listed")
        XCTAssertTrue(app.buttons["Clear Favorites"].isEnabled)
        favourite.tap()
        XCTAssertFalse(app.buttons["Clear Favorites"].waitForExistence(timeout: 2), "the menu stayed open")

        // Clear Favorites empties the list.
        openFavorites(app)
        tapItem("Clear Favorites", in: app)
        openFavorites(app)
        XCTAssertTrue(app.buttons["No favorites yet"].waitForExistence(timeout: 5), "the list did not empty")
        XCTAssertFalse(app.buttons["Sennheiser HD 650"].exists)
        XCTAssertFalse(app.buttons["Clear Favorites"].isEnabled, "Clear Favorites with nothing to clear")
    }

    func testSW18UpdateDatabaseOffersTheCoresUpdate() {
        let app = launch()

        openAutoEQ(app)
        tapItem("Update Database...", in: app)
        let dialog = app.alerts["Update AutoEQ Database"]
        XCTAssertTrue(dialog.waitForExistence(timeout: 5), "no Update Database dialog")
        XCTAssertTrue(dialog.staticTexts.matching(NSPredicate(format: "label CONTAINS 'Entries:'")).firstMatch.exists,
                      "the dialog does not say what is in use")
        for button in ["Rebuild from GitHub", "Import File...", "Cancel"] {
            XCTAssertTrue(dialog.buttons[button].exists, "no \(button)")
        }

        // Rebuild asks first, in the core's words, and Cancel starts nothing.
        dialog.buttons["Rebuild from GitHub"].tap()
        let confirm = app.alerts["Rebuild AutoEQ Database"]
        XCTAssertTrue(confirm.waitForExistence(timeout: 5), "no confirmation before a rebuild")
        XCTAssertTrue(confirm.staticTexts.matching(NSPredicate(format: "label CONTAINS 'api.github.com'")).firstMatch.exists,
                      "the confirmation lacks the core's warning")
        confirm.buttons["Cancel"].tap()
        XCTAssertFalse(app.navigationBars["Rebuilding AutoEQ Database"].waitForExistence(timeout: 2), "a rebuild started")

        // Import File opens the document picker; cancelling it changes nothing.
        openAutoEQ(app)
        tapItem("Update Database...", in: app)
        XCTAssertTrue(app.alerts["Update AutoEQ Database"].waitForExistence(timeout: 5))
        app.alerts["Update AutoEQ Database"].buttons["Import File..."].tap()
        let browser = app.navigationBars["FullDocumentManagerViewControllerNavigationBar"]
        XCTAssertTrue(browser.waitForExistence(timeout: 20), "the document picker did not open")
        app.buttons["Cancel"].tap()
        expectation(for: NSPredicate(format: "exists == false"), evaluatedWith: browser)
        waitForExpectations(timeout: 10)
        XCTAssertFalse(app.alerts.firstMatch.waitForExistence(timeout: 2), "cancelling the picker showed an alert")
    }
}
