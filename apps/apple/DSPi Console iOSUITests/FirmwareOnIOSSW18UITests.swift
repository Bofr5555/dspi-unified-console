import XCTest

/// WP SW18: no firmware window on iOS.  An iPhone or iPad cannot reach a
/// board's bootloader drive and the app carries no images, so the Tools menu
/// has no Firmware Update entry, the mismatch banner has no button that could
/// open the window and points to a computer instead, and the setup wizard's
/// firmware step does the same.  `-DemoFirmwareMatch` (DEBUG) makes the demo
/// device report a mismatch so the banner shows.
final class FirmwareOnIOSSW18UITests: XCTestCase {

    override func setUp() {
        continueAfterFailure = false
        XCUIDevice.shared.orientation = .portrait
    }

    private func launch(_ args: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-LocalTransport", "demo", "-DemoMode", "YES"] + args
        app.launch()
        return app
    }

    private let firmwareWords = NSPredicate(format: "label CONTAINS[c] 'firmware'")

    func testSW18ToolsHasNoFirmwareEntry() {
        let app = launch()
        let tools = app.buttons["Tools"]
        XCTAssertTrue(tools.waitForExistence(timeout: 30), "no Tools menu")
        tools.tap()
        XCTAssertTrue(app.buttons["Matrix Mixer"].waitForExistence(timeout: 5), "the Tools menu did not open")
        XCTAssertEqual(app.buttons.matching(firmwareWords).count, 0,
                       "a firmware entry in Tools: \(app.buttons.matching(firmwareWords).allElementsBoundByIndex.map(\.label))")
        // The whole menu, down to its end, where the entry used to be.
        let factoryReset = app.buttons["Factory Reset..."]
        if !factoryReset.waitForExistence(timeout: 2) { app.buttons["Matrix Mixer"].swipeUp() }
        XCTAssertTrue(factoryReset.waitForExistence(timeout: 5), "the end of the Tools menu was not reached")
        XCTAssertTrue(app.buttons["What's New"].exists, "the end of the Tools menu was not reached")
        XCTAssertEqual(app.buttons.matching(firmwareWords).count, 0, "a firmware entry at the end of Tools")
    }

    func testSW18TheBannerCannotOpenTheFirmwareWindow() {
        for (match, words) in [("older", "uConsole for DSPi expects"), ("newer", "newer than uConsole for DSPi")] {
            let app = launch(["-DemoFirmwareMatch", match])
            let banner = app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", words)).firstMatch
            XCTAssertTrue(banner.waitForExistence(timeout: 30), "\(match): no mismatch banner")
            XCTAssertTrue(banner.label.contains("Mac, Windows or Linux computer"),
                          "\(match): the banner does not point to a computer: \(banner.label)")
            for button in ["Update...", "Details..."] {
                XCTAssertFalse(app.buttons[button].exists, "\(match): the banner has \(button)")
            }
            XCTAssertTrue(app.buttons["Hide"].exists, "\(match): the banner lost Hide")

            // Tapping the banner itself opens nothing either.
            banner.tap()
            XCTAssertFalse(app.navigationBars["Firmware Update"].waitForExistence(timeout: 3),
                           "\(match): the firmware window opened")
            XCTAssertEqual(app.staticTexts.matching(NSPredicate(format: "label == 'Firmware Update'")).count, 0)

            app.buttons["Hide"].tap()
            expectation(for: NSPredicate(format: "exists == false"), evaluatedWith: banner)
            waitForExpectations(timeout: 5)
            app.terminate()
        }
    }

    /// The wizard's firmware step on iOS: no install, a computer instead,
    /// and Continue goes on (there is nothing here to wait for).
    func testSW18TheWizardSendsTheFirmwareStepToAComputer() {
        let app = launch(["-DSPiOnboardingForceWizard", "YES"])
        let continueButton = app.buttons["Continue"]
        XCTAssertTrue(continueButton.waitForExistence(timeout: 30), "no wizard")
        continueButton.tap()

        let title = app.staticTexts["Install the firmware on a computer"]
        XCTAssertTrue(title.waitForExistence(timeout: 10), "the board step does not point to a computer")
        XCTAssertTrue(app.staticTexts["Use a computer for the firmware"].exists)
        for button in ["Install DSPi Firmware", "Update DSPi Firmware", "Downgrade Firmware"] {
            XCTAssertFalse(app.buttons[button].exists, "\(button) on iOS")
        }
        XCTAssertTrue(app.buttons["Continue"].exists, "the step holds the person up")
        app.buttons["Continue"].tap()
        XCTAssertTrue(app.staticTexts["Ready when your DSPi is"].waitForExistence(timeout: 10))
    }
}
