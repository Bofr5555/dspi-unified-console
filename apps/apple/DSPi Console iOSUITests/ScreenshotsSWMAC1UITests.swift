import XCTest

/// WP SWMAC1: screenshots of the main screens on the demo device, light and
/// dark, kept in the result bundle.  Skipped unless the run sets
/// TEST_RUNNER_SWMAC1_SHOTS=1, so the matrix row does not pay for them.
final class ScreenshotsSWMAC1UITests: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = true
        guard ProcessInfo.processInfo.environment["SWMAC1_SHOTS"] == "1" else {
            throw XCTSkip("screenshots only with TEST_RUNNER_SWMAC1_SHOTS=1")
        }
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

    private func run(_ appearance: XCUIDevice.Appearance, _ tag: String) {
        XCUIDevice.shared.appearance = appearance
        let app = XCUIApplication()
        // The core's demo device, so Undo and Redo have the core's journal.
        app.launchArguments = ["-LocalTransport", "demo", "-DemoMode", "YES", "-DemoPreset", "2",
                               "-CoreTransport", "YES"]
        app.launch()
        XCTAssertTrue(app.buttons["Devices"].waitForExistence(timeout: 60))
        pause(3)
        keep(app, "ios-\(tag)-01-main")

        if app.buttons["Channels"].exists {
            app.buttons["Channels"].tap()
            pause(1)
            keep(app, "ios-\(tag)-02-channels")
            let usb1 = app.staticTexts["USB 1"].firstMatch
            if usb1.waitForExistence(timeout: 5) { usb1.tap() }
            pause(2)
            keep(app, "ios-\(tag)-03-channel-usb1")
        }

        app.buttons["Tools"].tap()
        pause(1)
        keep(app, "ios-\(tag)-04-tools-menu")
        let stats = app.buttons["Stats for nerbs"]
        if stats.waitForExistence(timeout: 5) {
            stats.tap()
            pause(3)
            keep(app, "ios-\(tag)-05-stats-for-nerbs")
            if app.buttons["Done"].exists { app.buttons["Done"].tap() } else { app.swipeDown(velocity: .fast) }
            pause(1)
        } else {
            app.tap()
        }

        app.buttons["Devices"].tap()
        pause(2)
        keep(app, "ios-\(tag)-06-devices")
        if app.buttons["Done"].exists { app.buttons["Done"].tap() } else { app.swipeDown(velocity: .fast) }
        pause(1)

        app.buttons["Settings"].tap()
        pause(2)
        keep(app, "ios-\(tag)-07-settings")
        app.terminate()
    }

    func testSWMAC1ScreenshotsLight() { run(.light, "light") }
    func testSWMAC1ScreenshotsDark() { run(.dark, "dark") }
}
