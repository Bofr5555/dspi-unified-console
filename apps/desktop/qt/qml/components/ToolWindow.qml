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

    // -- WP QTPOP -- Fitted to its content: when it shows, and when the
    // content's own size changes while it shows (never because the person
    // changed the window's size), the window takes the height at which none of
    // `fitBodies` (its scrolling Flickables; a ScrollView's contentItem)
    // scrolls, never less than defaultHeight and, growing, no more than
    // Shell.fitCap (the main window's size and the screen's, less 48 px each
    // side), beyond which the body scrolls. It widens only when a body's
    // content is wider than it. A size the person gives it sticks: once they
    // resize it during a showing, a refit may grow it but never shrinks it.
    // Lists of records are no fit body: they scroll by design. With no fit body
    // showing, the size is left alone.
    property var fitBodies: []
    // How much more room the content wants than the bodies have (less than 0
    // when it is shorter); NaN with no fit body showing. Read when fitting,
    // never a trigger: it follows the window's size.
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
    // The trigger: the content's size alone (and which bodies show)
    readonly property real fitContentSize: {
        let sum = 0
        for (const b of fitBodies) {
            if (b && b.visible && !isNaN(b.contentHeight)) sum += b.contentHeight + b.contentWidth / 65536
        }
        return sum
    }
    // -- WP LAY1a -- The size the bodies are laid out in. A window resized while
    // hidden shows its first frame at its new size with its content still laid
    // out in the old one (its resize event comes later, or not at all), so the
    // fit counts from here and never from the window's own size.
    readonly property real fitLaidWidth: contentItem.width
    readonly property real fitLaidHeight: contentItem.height + (header && header.visible ? header.height : 0)
    // -- WP LAY1a -- The bodies' own sizes: layouts that follow the window only
    // after a fit has measured them change these, not the content, and it fits
    // again; unless the person resized the window in this showing (that is
    // known by the next frame): then it is their resize.
    readonly property real fitBodySize: {
        let sum = 0
        for (const b of fitBodies) {
            if (b && b.visible && !isNaN(b.contentHeight)) sum += b.height + b.width / 65536
        }
        return sum
    }
    property bool fitBodyPending: false
    property bool fitPending: false
    property bool fitShown: false       // fitted once in this showing
    property bool fitUserSized: false   // resized by the person in this showing
    property size fitAsked: Qt.size(-1, -1)
    property bool fitResizing: false    // inside Shell.fitWindow (a resize may land at once)
    // The window's width just changed: wrapped text re-flows before the next
    // frame, and that change of content is the window's doing, not the content's
    property bool fitReflow: false
    function fitLater() {
        if (!visible) return
        fitPending = true
        update()
    }
    function fitSizeChanged() {
        if (!visible || !fitShown || fitResizing) return
        if (Math.abs(width - fitAsked.width) > 0.5 || Math.abs(height - fitAsked.height) > 0.5) fitUserSized = true
    }
    onWidthChanged: {
        fitSizeChanged()
        if (visible && !fitResizing && Math.abs(width - fitAsked.width) > 0.5) {
            fitReflow = true
            update()
        }
    }
    onHeightChanged: fitSizeChanged()
    onFitContentSizeChanged: if (!fitReflow) fitLater()
    onFitBodySizeChanged: {
        if (!visible || fitUserSized) return
        fitBodyPending = true
        update()
    }
    onVisibleChanged: {
        fitBodyPending = false
        fitShown = false
        fitUserSized = false
        fitReflow = false
        fitAsked = Qt.size(-1, -1)
        fitLater()
    }
    // Measured after the next frame, once the layouts have settled (wrapped
    // text is briefly very tall before its width is set)
    onFrameSwapped: {
        fitReflow = false
        if (fitBodyPending && !fitUserSized) fitPending = true
        fitBodyPending = false
        if (!fitPending) return
        fitPending = false
        if (!visible || isNaN(fitExtraHeight)) return
        const w = fitExtraWidth > 0.5 ? Math.max(width, Math.ceil(fitLaidWidth + fitExtraWidth)) : width
        let h = Math.max(defaultHeight, Math.ceil(fitLaidHeight + fitExtraHeight))
        if (fitUserSized) h = Math.max(h, height)
        fitAsked = Qt.size(width, height)
        if (w !== width || Math.abs(h - height) > 0.5) {
            fitResizing = true
            fitAsked = Shell.fitWindow(window, w, h)
            fitResizing = false
        }
        fitShown = true
    }

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
