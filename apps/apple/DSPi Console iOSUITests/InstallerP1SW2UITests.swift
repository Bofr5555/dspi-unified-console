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

    // -- WP SW5F -- (review-p1sw3 F7, recurring) The run failed on a different
    // line each time (:44, :52, :110, :117, :151): every time a tap whose
    // effect the next line assumed had been dropped by the Simulator under
    // the full suite's load (a sheet or the keyboard still animating).  Each
    // step now waits for its control to be hittable, taps, waits for its
    // effect, and taps again if the effect did not come (at most `tries`).

    /// Tap `element` until `effect` holds; false (and a failure naming
    /// `what`) if it never does.
    @discardableResult
    private func tap(_ app: XCUIApplication, _ element: XCUIElement, until effect: @escaping () -> Bool,
                     _ what: String, wait: TimeInterval = 5, tries: Int = 3,
                     file: StaticString = #filePath, line: UInt = #line) -> Bool {
        for _ in 0..<tries {
            if effect() { return true }
            reveal(app, element)
            _ = element.waitForExistence(timeout: wait)
            let hittable = Date().addingTimeInterval(wait)
            while element.exists && !element.isHittable && Date() < hittable { pause(0.1) }
            if element.exists && element.isHittable { element.tap() }
            let deadline = Date().addingTimeInterval(wait)
            while Date() < deadline {
                if effect() { return true }
                pause(0.1)
            }
        }
        let ok = effect()
        XCTAssertTrue(ok, what, file: file, line: line)
        return ok
    }

    /// The field has the keyboard within 5 s.
    private func spinFocus(_ app: XCUIApplication, _ field: XCUIElement) -> Bool {
        let deadline = Date().addingTimeInterval(5)
        while Date() < deadline {
            if field.hasKeyboardFocusSW5F || app.keyboards.firstMatch.exists { return true }
            pause(0.1)
        }
        return false
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
        tap(app, app.buttons["modeSwitchApply"].firstMatch, until: { now.label.contains("Installer") },
            "the mode switch applies: \(now.label)")
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
        let code = app.staticTexts["installerRecoveryCode"].firstMatch
        tap(app, app.buttons["installerSetPin"].firstMatch, until: { code.exists },
            "the recovery code, shown once", wait: 10)
        keep(app, "ios-p1sw2-04-recovery-code")
        tap(app, app.buttons["installerRecoveryDone"].firstMatch, until: { !code.exists },
            "the recovery code goes once written down")

        let cover = app.switches["installerLockSettings"].firstMatch
        reveal(app, cover)
        XCTAssertTrue(cover.waitForExistence(timeout: 10))
        if let before = cover.value as? String {
            tap(app, cover.switches.firstMatch, until: { (cover.value as? String).map { $0 != before } ?? false },
                "the lock-settings switch turns over")
        } else {
            cover.switches.firstMatch.tap()
        }
        pause(1)
        // Locked: the Installer pages are gone from Settings.
        let lockNow = app.buttons["installerLockNow"].firstMatch
        tap(app, lockNow, until: { !lockNow.exists }, "Lock Now locks (the Installer pages go)")
        pause(1)

        // 3. The locked user's front page: the banner, Unlock, the analyser.
        let banner = app.descendants(matching: .any)["lockBanner"].firstMatch
        tap(app, app.buttons["Done"].firstMatch, until: { banner.exists && banner.isHittable },
            "Settings closes onto the locked-user banner", wait: 8)
        keep(app, "ios-p1sw2-05-locked-banner")
        tap(app, app.buttons["Tools"].firstMatch, until: { app.buttons["Spectrum Analyser"].exists }, "the Tools menu opens")
        XCTAssertTrue(app.buttons["Unlock..."].waitForExistence(timeout: 5), "Tools: Unlock")
        XCTAssertTrue(app.buttons["Spectrum Analyser"].exists, "the analyser stays")
        XCTAssertFalse(app.buttons["Matrix Mixer"].exists, "the rest is not offered")
        keep(app, "ios-p1sw2-06-locked-tools")
        let sheet = app.descendants(matching: .any)["unlockSheet"].firstMatch
        tap(app, app.collectionViews.buttons["Unlock..."].firstMatch,   // the Tools menu's
            until: { sheet.exists }, "the one unlock prompt", wait: 8)
        let pin = app.secureTextFields["unlockPin"].firstMatch
        let wrong = app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "Wrong PIN")).firstMatch
        tap(app, pin, until: { pin.hasKeyboardFocusSW5F || app.keyboards.firstMatch.exists },
            "the PIN field takes the keyboard")
        pin.typeText("1111")
        tap(app, app.buttons["unlockButton"].firstMatch, until: { wrong.exists },
            "a wrong PIN, in the core's words", wait: 10)
        keep(app, "ios-p1sw2-07-unlock-wrong-pin")
        let badge = app.descendants(matching: .any)["installerBadge"].firstMatch
        XCTAssertTrue(pin.waitForExistence(timeout: 5))
        let hittable = Date().addingTimeInterval(5)
        while !pin.isHittable && Date() < hittable { pause(0.1) }
        pin.tap()
        XCTAssertTrue(spinFocus(app, pin), "the PIN field takes the keyboard again")
        pin.typeText("2468")
        tap(app, app.buttons["unlockButton"].firstMatch, until: { badge.exists },   // (Return would also unlock)
            "unlocked: the Installer line with Lock Now", wait: 10)
        expectation(for: NSPredicate(format: "exists == false"), evaluatedWith: banner)
        waitForExpectations(timeout: 5)   // the banner has gone
        keep(app, "ios-p1sw2-08-unlocked")

        // 4. Clean up: remove the PIN on the Installer page.
        tap(app, app.buttons["Settings"].firstMatch, until: { page.exists }, "Settings > Installer again", wait: 10)
        let confirm = app.alerts.buttons["Remove PIN"].firstMatch
        tap(app, app.buttons["installerRemovePin"].firstMatch, until: { confirm.exists }, "Remove PIN asks first")
        let newPin = app.secureTextFields["installerNewPin"].firstMatch
        tap(app, confirm, until: { newPin.exists }, "no PIN again: nothing is locked", wait: 10)
    }
}

private extension XCUIElement {
    /// -- WP SW5F -- The field has the keyboard (XCUIElement's own key path).
    var hasKeyboardFocusSW5F: Bool { (value(forKey: "hasKeyboardFocus") as? Bool) ?? false }
}
