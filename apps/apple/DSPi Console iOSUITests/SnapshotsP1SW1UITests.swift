import XCTest

/// WP P1SW1: the Snapshots sheet on the iPhone, on the core's demo device:
/// it opens at half height in Simple, Advanced captures, a recall with
/// rollback shows the countdown bar over the console, and Keep ends it.
/// Screenshots are kept in the result bundle.
final class SnapshotsP1SW1UITests: XCTestCase {

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

    func testP1SW1SnapshotsSheetCaptureRecallWithRollbackAndKeep() {
        let app = XCUIApplication()
        app.launchArguments = ["-LocalTransport", "demo", "-DemoMode", "YES", "-CoreTransport", "YES",
                               "-OpenTool", "snapshots"]
        app.launch()

        let sheet = app.descendants(matching: .any)["snapshotsView"].firstMatch
        XCTAssertTrue(sheet.waitForExistence(timeout: 60), "the Snapshots sheet opens")
        // Simple first (an earlier run may have left Advanced on)
        app.buttons["Simple"].firstMatch.tap()
        pause(2)
        keep(app, "ios-p1sw1-01-simple")
        XCTAssertFalse(app.buttons["snapshotCapture"].exists, "Simple: no capture")

        app.buttons["Advanced"].firstMatch.tap()
        let capture = app.buttons["snapshotCapture"].firstMatch
        XCTAssertTrue(capture.waitForExistence(timeout: 10))
        capture.tap()
        let recall = app.buttons["snapshotRecall"].firstMatch
        XCTAssertTrue(recall.waitForExistence(timeout: 15), "the captured snapshot is listed")
        capture.tap()
        pause(2)
        keep(app, "ios-p1sw1-02-advanced-two-snapshots")

        // Recall with rollback: the countdown bar over the console.
        let rollback = app.switches["snapshotRollback"].firstMatch
        if !rollback.isHittable { app.swipeUp() }
        XCTAssertTrue(rollback.waitForExistence(timeout: 5))
        if (rollback.value as? String) != "1" { rollback.tap() }
        if !recall.isHittable { app.swipeDown() }
        XCTAssertTrue(app.buttons["Recall with rollback"].firstMatch.waitForExistence(timeout: 5))
        app.buttons["Recall with rollback"].firstMatch.tap()
        pause(1)
        app.buttons["Done"].firstMatch.tap()

        let bar = app.descendants(matching: .any)["safeApplyBar"].firstMatch
        let keepButton = app.buttons["safeApplyKeep"].firstMatch
        XCTAssertTrue(keepButton.waitForExistence(timeout: 15), "the countdown bar with Keep")
        pause(1)
        keep(app, "ios-p1sw1-03-safe-apply-bar")
        keepButton.tap()
        XCTAssertTrue(app.buttons["safeApplyOK"].firstMatch.waitForExistence(timeout: 10), "kept: the end's sentence")
        keep(app, "ios-p1sw1-04-kept")
        app.buttons["safeApplyOK"].firstMatch.tap()
        XCTAssertFalse(bar.waitForExistence(timeout: 2) && app.buttons["safeApplyOK"].exists, "OK hides it")

        // The Files entries.
        app.buttons["Tools"].tap()
        if app.buttons["Files"].waitForExistence(timeout: 3) { app.buttons["Files"].tap() }
        XCTAssertTrue(app.buttons["Open File..."].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["Open Template..."].exists)
        XCTAssertTrue(app.buttons["Save as Template..."].exists)
        keep(app, "ios-p1sw1-05-files-menu")
        app.buttons["Open Template..."].tap()
        XCTAssertTrue(app.descendants(matching: .any)["templatePicker"].firstMatch.waitForExistence(timeout: 10),
                      "the template list opens")
        pause(1)
        keep(app, "ios-p1sw1-06-templates")
        app.terminate()
    }
}
