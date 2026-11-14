import XCTest

/// WP P1SW2: the mode switch, the Installer page, the PIN with its recovery
/// code, the locked-user banner and the one unlock prompt on the iPhone, on
/// the core's demo device.  Screenshots are kept in the result bundle.
final class InstallerP1SW2UITests: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
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

    private func launch(_ extra: [String]) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-LocalTransport", "demo", "-DemoMode", "YES", "-CoreTransport", "YES"] + extra
        app.launch()
        return app
    }

    /// Scroll until `element` can be tapped (a long Form on an iPhone).
    private func reveal(_ app: XCUIApplication, _ element: XCUIElement, tries: Int = 6) {
        // A tap scrolls the Form to the element itself; swipe only for one
        // not built yet (a lazy row further down), never with the keyboard up.
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

    func testP1SW2ModeSwitchInstallerPinLockedBannerAndUnlock() {
        // 1. No PIN, iOS opens in Simple; Settings > Mode switches to Installer
        //    with no prompt.
        var app = launch(["-ResetInstaller", "YES", "-OpenTool", "settings", "-SettingsPage", "mode"])
        let modePage = app.descendants(matching: .any)["modeSwitch"].firstMatch
        XCTAssertTrue(modePage.waitForExistence(timeout: 60), "Settings > Mode")
        let now = app.staticTexts["modeNow"].firstMatch
        XCTAssertTrue(now.waitForExistence(timeout: 10))
        XCTAssertTrue(now.label.contains("Simple"), "iOS opens in Simple: \(now.label)")
        keep(app, "ios-p1sw2-01-mode-simple")
        app.buttons["Installer"].firstMatch.tap()
        app.buttons["modeSwitchApply"].firstMatch.tap()
        let installerNow = NSPredicate(format: "label CONTAINS %@", "Installer")
        expectation(for: installerNow, evaluatedWith: now)
        waitForExpectations(timeout: 10)
        XCTAssertFalse(app.secureTextFields["modePin"].exists, "no PIN set: nothing asks")
        keep(app, "ios-p1sw2-02-mode-installer")

        // 2. The Installer page (the choice is kept for this device).
        app.terminate()
        app = launch(["-OpenTool", "settings", "-SettingsPage", "installer"])
        let page = app.descendants(matching: .any)["installerPage"].firstMatch
        XCTAssertTrue(page.waitForExistence(timeout: 60), "Settings > Installer in the Installer mode")
        XCTAssertTrue(app.textFields["limit.masterVolumeMaxDb"].firstMatch.waitForExistence(timeout: 10),
                      "the limits editor")
        keep(app, "ios-p1sw2-03-installer-page")

        type(app, app.secureTextFields["installerNewPin"].firstMatch, "2468")
        type(app, app.secureTextFields["installerNewPinAgain"].firstMatch, "2468\n")   // Return: the keyboard goes
        pause(1)
        let setPin = app.buttons["installerSetPin"].firstMatch
        reveal(app, setPin)
        setPin.tap()
        let code = app.staticTexts["installerRecoveryCode"].firstMatch
        XCTAssertTrue(code.waitForExistence(timeout: 15), "the recovery code, shown once")
        keep(app, "ios-p1sw2-04-recovery-code")
        app.buttons["installerRecoveryDone"].firstMatch.tap()
        expectation(for: NSPredicate(format: "exists == false"), evaluatedWith: code)
        waitForExpectations(timeout: 5)   // gone once written down

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
        // Locked: the Installer pages are gone from Settings.
        app.buttons["Done"].firstMatch.tap()

        // 3. The locked user's front page: the banner, Unlock, the analyser.
        let banner = app.descendants(matching: .any)["lockBanner"].firstMatch
        XCTAssertTrue(banner.waitForExistence(timeout: 15), "the locked-user banner")
        keep(app, "ios-p1sw2-05-locked-banner")
        app.buttons["Tools"].tap()
        XCTAssertTrue(app.buttons["Unlock..."].waitForExistence(timeout: 5), "Tools: Unlock")
        XCTAssertTrue(app.buttons["Spectrum Analyser"].exists, "the analyser stays")
        XCTAssertFalse(app.buttons["Matrix Mixer"].exists, "the rest is not offered")
        keep(app, "ios-p1sw2-06-locked-tools")
        app.collectionViews.buttons["Unlock..."].firstMatch.tap()   // the Tools menu's

        let sheet = app.descendants(matching: .any)["unlockSheet"].firstMatch
        XCTAssertTrue(sheet.waitForExistence(timeout: 10), "the one unlock prompt")
        let pin = app.secureTextFields["unlockPin"].firstMatch
        pin.tap()
        pin.typeText("1111")
        app.buttons["unlockButton"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "Wrong PIN")).firstMatch
                        .waitForExistence(timeout: 15), "a wrong PIN, in the core's words")
        keep(app, "ios-p1sw2-07-unlock-wrong-pin")
        pin.tap()
        pin.typeText("2468")
        app.buttons["unlockButton"].firstMatch.tap()   // (Return would also unlock)
        XCTAssertTrue(app.descendants(matching: .any)["installerBadge"].firstMatch.waitForExistence(timeout: 15),
                      "unlocked: the Installer line with Lock Now")
        expectation(for: NSPredicate(format: "exists == false"), evaluatedWith: banner)
        waitForExpectations(timeout: 5)   // the banner has gone
        keep(app, "ios-p1sw2-08-unlocked")

        // 4. Clean up: remove the PIN on the Installer page.
        app.buttons["Settings"].firstMatch.tap()
        XCTAssertTrue(page.waitForExistence(timeout: 15))
        let remove = app.buttons["installerRemovePin"].firstMatch
        reveal(app, remove)
        remove.tap()
        let confirm = app.alerts.buttons["Remove PIN"].firstMatch
        XCTAssertTrue(confirm.waitForExistence(timeout: 5))
        confirm.tap()
        XCTAssertTrue(app.secureTextFields["installerNewPin"].firstMatch.waitForExistence(timeout: 15),
                      "no PIN again: nothing is locked")
    }
}
