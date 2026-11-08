import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Window 2.15

// Client-side titlebar (Linux, Windows): menu button over the sidebar, centred
// title, minimise / maximise / close (in the Windows layout on Windows).
// Moving and resizing go through the window manager (startSystemMove /
// startSystemResize), so snapping still works.
Item {
    id: bar
    property var window
    property int sidebarWidth: 260
    property bool showMenuButton: true
    property bool showMinMax: true
    property string titleText: window ? window.title : ""
    property bool showTitle: true
    // Back / Forward arrows just right of the sidebar (Settings)
    property bool showNav: false
    property bool canGoBack: false
    property bool canGoForward: false
    signal menuRequested(Item anchorItem)
    signal goBack()
    signal goForward()

    readonly property bool maximized: window && window.visibility === Window.Maximized

    function toggleMaximized() {
        if (maximized) window.showNormal()
        else window.showMaximized()
    }

    // Drag to move, double-click to maximise / restore
    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton
        onPressed: windowEffects.systemMove(bar.window)
        onDoubleClicked: bar.toggleMaximized()
        Accessible.ignored: true   // the window manager's move; the buttons maximise
    }

    component BarButton: Item {
        id: bb
        property string icon: ""
        property bool danger: false
        property bool lit: false
        property bool active: true
        property string accessibleName: ""
        signal clicked()
        opacity: active ? 1.0 : 0.35
        width: 24
        height: 24
        anchors.verticalCenter: parent.verticalCenter
        Rectangle {
            anchors.centerIn: parent
            width: 22; height: 22; radius: 11
            color: bbMouse.containsMouse ? (bb.danger ? "#e0454a" : Qt.rgba(1, 1, 1, 0.12))
                 : bb.lit ? Qt.rgba(1, 1, 1, 0.10) : "transparent"
        }
        Icon {
            anchors.centerIn: parent
            name: bb.icon
            size: 14
            color: bbMouse.containsMouse && bb.danger ? "white"
                 : bbMouse.containsMouse || bb.lit ? Qt.rgba(1, 1, 1, 0.95) : Qt.rgba(1, 1, 1, 0.65)
        }
        ClickArea {
            id: bbMouse
            anchors.fill: parent
            hoverEnabled: bb.active
            enabled: bb.active
            focusRadius: 13
            Accessible.name: bb.accessibleName
            onActivated: bb.clicked()
        }
    }

    BarButton {
        id: menuBtn
        visible: bar.showMenuButton
        x: 10
        icon: "menu"
        accessibleName: "Menu"
        onClicked: bar.menuRequested(menuBtn)
        ToolTip.visible: false
    }

    Row {
        visible: bar.showNav
        x: bar.sidebarWidth + 10
        anchors.verticalCenter: parent.verticalCenter
        spacing: 2
        BarButton { icon: "chev-left"; accessibleName: "Back"; active: bar.canGoBack; onClicked: bar.goBack() }
        BarButton { icon: "chev-right"; accessibleName: "Forward"; active: bar.canGoForward; onClicked: bar.goForward() }
    }

    Text {
        visible: bar.showTitle
        // Centred over the content area, like the KDE title
        x: bar.sidebarWidth + (bar.width - bar.sidebarWidth - width) / 2
        anchors.verticalCenter: parent.verticalCenter
        text: bar.titleText
        font.pixelSize: 13
        font.weight: Font.DemiBold
        color: bar.window && bar.window.active ? Qt.rgba(1, 1, 1, 0.85) : Qt.rgba(1, 1, 1, 0.45)
    }

    Row {
        visible: !isWindows
        anchors.right: parent.right
        anchors.rightMargin: 8
        anchors.verticalCenter: parent.verticalCenter
        spacing: 4
        BarButton { visible: bar.showMinMax; icon: "win-min"; accessibleName: "Minimise"; onClicked: bar.window.showMinimized() }
        BarButton { visible: bar.showMinMax; icon: bar.maximized ? "win-restore" : "win-max"; accessibleName: bar.maximized ? "Restore" : "Maximise"; onClicked: bar.toggleMaximized() }
        BarButton { icon: "win-close"; accessibleName: "Close"; danger: true; onClicked: bar.window.close() }
    }

    // Windows: caption buttons as Windows draws them, 46 px wide, the full
    // height of the bar, flush with the window's corner; close turns red
    component CaptionButton: Rectangle {
        id: cb
        property string icon: ""
        property bool danger: false
        property string accessibleName: ""
        signal clicked()
        width: 46
        height: bar.height
        color: cbMouse.pressed ? (danger ? "#94141e" : Qt.rgba(1, 1, 1, 0.12))
             : cbMouse.containsMouse ? (danger ? "#c42b1c" : Qt.rgba(1, 1, 1, 0.08)) : "transparent"
        Icon {
            anchors.centerIn: parent
            name: cb.icon
            size: 14
            color: cbMouse.containsMouse && cb.danger ? "white"
                 : bar.window && bar.window.active ? Qt.rgba(1, 1, 1, 0.9) : Qt.rgba(1, 1, 1, 0.45)
        }
        ClickArea {
            id: cbMouse
            anchors.fill: parent
            hoverEnabled: true
            focusRadius: 2
            Accessible.name: cb.accessibleName
            onActivated: cb.clicked()
        }
    }

    Row {
        visible: isWindows
        anchors.right: parent.right
        anchors.top: parent.top
        CaptionButton { visible: bar.showMinMax; icon: "win-min"; accessibleName: "Minimise"; onClicked: bar.window.showMinimized() }
        CaptionButton { visible: bar.showMinMax; icon: bar.maximized ? "win-restore" : "win-max"; accessibleName: bar.maximized ? "Restore" : "Maximise"; onClicked: bar.toggleMaximized() }
        CaptionButton { icon: "win-close"; accessibleName: "Close"; danger: true; onClicked: bar.window.close() }
    }
}
