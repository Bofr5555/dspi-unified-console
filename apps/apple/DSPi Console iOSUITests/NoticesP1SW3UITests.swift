import XCTest

/// WP P1SW3: on the iPhone, a finished Tools action shows on the main
/// window's notice host, not in a "Success" alert (the question before it
/// stays an alert); and a demo device the installer has locked keeps the
/// main window for reading, with the core's reason over the page.
/// Screenshots are kept in the result bundle.
final class NoticesP1SW3UITests: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
        XCUIDevice.shared.orientation = .portrait
    }

    private func keep(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    private func pause(_ seconds: TimeInterval) {
        RunLoop.current.run(until: Date().addingTimeInterval(seconds))
    }

    private func launch(_ extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-LocalTransport", "demo", "-DemoMode", "YES", "-CoreTransport", "YES"] + extra
        app.launch()
        return app
    }

    private func tools(_ app: XCUIApplication, _ item: String) {
        let tools = app.buttons["Tools"]
        XCTAssertTrue(tools.waitForExistence(timeout: 60), "no Tools menu")
        tools.tap()
        let button = app.buttons[item].firstMatch
        XCTAssertTrue(button.waitForExistence(timeout: 5), "no \(item) in Tools")
        button.tap()
    }

    private func noticeTitle(_ app: XCUIApplication) -> XCUIElement {
        app.staticTexts.matching(identifier: "noticeTitle").firstMatch
    }

    func testP1SW3AFinishedToolsActionIsANoticeNotAnAlert() {
        let app = launch(["-ResetInstaller", "YES"])

        // Revert to Saved: the question is still an alert ...
        tools(app, "Revert to Saved...")
        let ask = app.alerts["Revert to Saved"]
        XCTAssertTrue(ask.waitForExistence(timeout: 10), "the question stays modal")
        ask.buttons["Revert"].tap()
        // ... its outcome is a notice.
        let title = noticeTitle(app)
        XCTAssertTrue(title.waitForExistence(timeout: 15), "the outcome on the notice host")
        XCTAssertFalse(app.alerts.firstMatch.exists, "no modal for the outcome")
        XCTAssertFalse(title.label.contains("Success"), title.label)
        keep(app, "ios-p1sw3-01-notice-revert")
        app.buttons["noticeDismiss"].firstMatch.tap()
        expectation(for: NSPredicate(format: "exists == false"), evaluatedWith: title)
        waitForExpectations(timeout: 5)

        // Commit Parameters: Save, then "Preset saved" on the host.
        tools(app, "Commit Parameters...")
        let save = app.alerts["Save Preset"]
        XCTAssertTrue(save.waitForExistence(timeout: 10))
        save.buttons["Save"].tap()
        let saved = app.staticTexts.matching(NSPredicate(format: "identifier == 'noticeTitle' AND label == 'Preset saved'"))
            .firstMatch
        XCTAssertTrue(saved.waitForExistence(timeout: 15), "Preset saved, as a notice")
        XCTAssertFalse(app.alerts.firstMatch.exists, "no Success alert")
        keep(app, "ios-p1sw3-02-notice-saved")
        // An info notice goes by itself.
        expectation(for: NSPredicate(format: "exists == false"), evaluatedWith: saved)
        waitForExpectations(timeout: 15)
    }

    /// Scroll until `element` can be tapped (a long Form on an iPhone).
    private func reveal(_ app: XCUIApplication, _ element: XCUIElement, tries: Int = 6) {
        var n = 0
        while !element.exists && n < tries {
            app.collectionViews.firstMatch.swipeUp()
            n += 1
        }
    }

    private func type(_ app: XCUIApplication, _ field: XCUIElement, _ text: String) {
        // -- WP P1SW3F -- F7: the field before's keyboard could cover this one
        // (it exists, so `reveal` did not scroll, and the tap hit the keyboard):
        // every field ends with Return, so the keyboard goes, and the Form
        // scrolls while the field is not hittable.
        reveal(app, field)
        var n = 0
        while field.exists && !field.isHittable && n < 6 {
            app.collectionViews.firstMatch.swipeUp()
            n += 1
        }
        field.tap()
        field.typeText(text.hasSuffix("\n") ? text : text + "\n")
    }

    func testP1SW3ALockedDemoDeviceKeepsTheMainWindowForReading() {
        // Installer mode with no PIN asks nothing (P1SW2); set a PIN, lock.
        var app = launch(["-ResetInstaller", "YES", "-OpenTool", "settings", "-SettingsPage", "mode"])
        let now = app.staticTexts["modeNow"].firstMatch
        XCTAssertTrue(now.waitForExistence(timeout: 60), "Settings > Mode")
        app.buttons["Installer"].firstMatch.tap()
        app.buttons["modeSwitchApply"].firstMatch.tap()
        expectation(for: NSPredicate(format: "label CONTAINS %@", "Installer"), evaluatedWith: now)
        waitForExpectations(timeout: 10)
        app.terminate()

        app = launch(["-OpenTool", "settings", "-SettingsPage", "installer"])
        let page = app.descendants(matching: .any)["installerPage"].firstMatch
        XCTAssertTrue(page.waitForExistence(timeout: 60), "Settings > Installer")
        type(app, app.secureTextFields["installerNewPin"].firstMatch, "3579")
        type(app, app.secureTextFields["installerNewPinAgain"].firstMatch, "3579\n")
        pause(1)
        let setPin = app.buttons["installerSetPin"].firstMatch
        reveal(app, setPin)
        setPin.tap()
        let code = app.staticTexts["installerRecoveryCode"].firstMatch
        XCTAssertTrue(code.waitForExistence(timeout: 15))
        app.buttons["installerRecoveryDone"].firstMatch.tap()
        let cover = app.switches["installerLockSettings"].firstMatch
        reveal(app, cover)
        XCTAssertTrue(cover.waitForExistence(timeout: 10))
        cover.switches.firstMatch.tap()
        pause(2)
        let lockNow = app.buttons["installerLockNow"].firstMatch
        reveal(app, lockNow)
        lockNow.tap()
        // -- WP P1SW3F -- locked, the Installer pages go: wait for that, and
        // tap once more if the first tap was lost (a full run's flake).
        let gone = NSPredicate(format: "exists == false")
        if XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: gone, object: lockNow)], timeout: 5) != .completed,
           lockNow.exists {
            lockNow.tap()
        }
        pause(1)
        app.buttons["Done"].firstMatch.tap()

        // The main window: the lock's banner and, over the page, the floor's
        // line in the core's words; the page is there to read, no alert.
        let banner = app.descendants(matching: .any)["lockBanner"].firstMatch
        XCTAssertTrue(banner.waitForExistence(timeout: 15), "the locked-user banner")
        var line = app.descendants(matching: .any)["readOnlyFloorLine"].firstMatch
        if !line.waitForExistence(timeout: 5), app.buttons["Console"].exists {
            app.buttons["Console"].tap()   // a single pane showing the channel list
            line = app.descendants(matching: .any)["readOnlyFloorLine"].firstMatch
        }
        XCTAssertTrue(line.waitForExistence(timeout: 10), "the read-only floor's line over the page")
        XCTAssertTrue(line.label.contains("only volume, mute, source and presets"), line.label)
        XCTAssertFalse(app.alerts.firstMatch.exists, "nothing is refused after the fact")
        keep(app, "ios-p1sw3-03-locked-floor")

        // Clean up: unlock, remove the PIN.
        app.buttons["Tools"].tap()
        XCTAssertTrue(app.buttons["Unlock..."].waitForExistence(timeout: 5), "Tools: Unlock")
        app.collectionViews.buttons["Unlock..."].firstMatch.tap()   // the Tools menu's
        let pin = app.secureTextFields["unlockPin"].firstMatch
        XCTAssertTrue(pin.waitForExistence(timeout: 10))
        pin.tap()
        pin.typeText("3579")
        app.buttons["unlockButton"].firstMatch.tap()
        expectation(for: NSPredicate(format: "exists == false"), evaluatedWith: banner)
        waitForExpectations(timeout: 15)
        expectation(for: NSPredicate(format: "exists == false"), evaluatedWith: line)
        waitForExpectations(timeout: 5)   // the floor has gone with the lock
        app.buttons["Settings"].firstMatch.tap()
        XCTAssertTrue(page.waitForExistence(timeout: 15))
        let remove = app.buttons["installerRemovePin"].firstMatch
        reveal(app, remove)
        remove.tap()
        let confirm = app.alerts.buttons["Remove PIN"].firstMatch
        XCTAssertTrue(confirm.waitForExistence(timeout: 5))
        confirm.tap()
        XCTAssertTrue(app.secureTextFields["installerNewPin"].firstMatch.waitForExistence(timeout: 15))
    }
}
