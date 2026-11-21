import QtCore
import QtQuick
import QtQuick.Controls.Basic
import UConsole

// A tool window (the Mac's NSWindow tools): the system's frame, the theme's
// background, the menu table's shortcuts, and its size and place remembered
// under `windowId` (Wayland ignores the place). Default
// sizes are the Mac's content sizes (mockups).
ApplicationWindow {
    id: window
    required property string windowId
    property int defaultWidth: 600
    property int defaultHeight: 400
    // -- WP QTSENDER -- The menu action that opened this window (Shell sets
    // it), and whether that action does nothing on the open device (the alink
    // sender): the content is then greyed under a banner that says why
    property string actionId: ""
    readonly property bool senderLocked: actionId.length > 0 && DeviceInfoController.sender
                                         && !DeviceInfoController.senderKeeps(actionId)
    contentItem.enabled: !senderLocked
    header: Banner {
        objectName: "ToolWindow.senderBanner"
        visible: window.senderLocked
        kind: "info"
        iconName: "radio"
        text: qsTr("This is an audio link sender. It has no DSP, so this window has nothing to show or change.")
    }

    // -- WP QTPOP -- Fitted to its content: on every show, and whenever the
    // content's size changes while it shows, the window takes the height at
    // which none of `fitBodies` (its scrolling Flickables; a ScrollView's
    // contentItem) scrolls, never less than defaultHeight and, growing, no
    // more than Shell.fitCap (the main window's size and the screen's, less
    // 48 px each side), beyond which the body scrolls. It widens only when a
    // body's content is wider than it. Lists of records are no fit body: they
    // scroll by design. With no fit body showing, the size is left alone.
    property var fitBodies: []
    // How much more room the content wants than the bodies have (less than 0
    // when it is shorter); NaN with no fit body showing
    readonly property real fitExtraWidth: fitExtra(true)
    readonly property real fitExtraHeight: fitExtra(false)
    function fitExtra(across) {
        let extra = -Infinity
        for (const b of fitBodies) {
            if (!b || !b.visible) continue
            const more = across ? b.contentWidth - b.width : b.contentHeight - b.height
            if (more > extra) extra = more   // NaN (not a Flickable) never is
        }
        return extra === -Infinity ? NaN : extra
    }
    // Measured after the next frame, once the layouts have settled (wrapped
    // text is briefly very tall before its width is set)
    property bool fitPending: false
    function fitLater() {
        if (!visible) return
        fitPending = true
        update()
    }
    onFrameSwapped: {
        if (!fitPending) return
        fitPending = false
        if (isNaN(fitExtraHeight)) return
        const w = fitExtraWidth > 0.5 ? Math.ceil(width + fitExtraWidth) : width
        const h = Math.max(defaultHeight, Math.ceil(height + fitExtraHeight))
        if (w !== width || Math.abs(h - height) > 0.5) Shell.fitWindow(window, w, h)
    }
    onVisibleChanged: fitLater()
    onFitExtraWidthChanged: fitLater()
    onFitExtraHeightChanged: fitLater()

    width: defaultWidth
    height: defaultHeight
    color: Theme.window
    palette.window: Theme.window
    palette.windowText: Theme.text
    palette.base: Theme.control
    palette.text: Theme.text
    palette.button: Theme.control
    palette.buttonText: Theme.text
    palette.highlight: Theme.accent
    palette.highlightedText: Theme.textOnAccent

    Settings {
        category: "windows/" + window.windowId
        property alias x: window.x
        property alias y: window.y
        property alias width: window.width
        property alias height: window.height
    }

    Shortcut {
        sequences: [StandardKey.Close]
        onActivated: window.close()
    }

    // The menu table's shortcuts work here too
    TableShortcuts {
        window: window
    }
}
